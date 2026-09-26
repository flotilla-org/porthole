# Live Windows native capture check (#186) against an isolated daemon.
#
# Starts its own portholed (examples/windows_capture_server) on a unique pipe
# with an in-memory policy store, creates a temporary agent identity, approves
# only that identity's requests, launches a test-owned capture fixture window,
# starts a native capture session, and runs a separate consumer process that
# reads D3D11 frames back. The fixture resizes itself; the script then closes
# the fixture through Porthole and checks that the session fails and refuses
# new consumers. It revokes the identity and stops its own daemon at the end.
# It never touches another daemon, pipe, identity or window, and never locks
# the workstation or disconnects RDP. If the fixture it launched is still
# running at the end (for example because `porthole close` was refused), it
# stops that one process by PID.
#
# -WatchSeconds N keeps the session running for N seconds after the resize
# check and logs every status change to status-watch.log and
# status-transitions.jsonl (with the raw WTS session state), for a person to
# lock/unlock or disconnect/reconnect RDP meanwhile. See
# docs/2026-09-25-windows-native-capture-evidence.md.
#
# Frame forensics (#189): the consumer writes frames\frames.jsonl (one line per
# frame) and PNGs of up to 200 frames that are not one fixture colour; the
# fixture writes fixture-events.jsonl (repaints, cursor over the window, WTS
# session changes). At the end the script prints, and writes to
# classification.txt, counts of non-uniform frames by kind and their timing
# relative to status and session transitions, fixture repaints and the cursor.
# -NoCursor starts the session without cursor capture. -ClassifyOnly re-runs
# only the classification over an existing -EvidenceDir.
#
# Logs are written with shared read/write access, so a reader such as
# `Get-Content -Wait` or `tail -f` cannot make a write fail.
#
# Build first:
#   cargo build -p porthole --locked
#   cargo build -p portholed --examples --locked
#   cargo build -p porthole-adapter-windows --example capture_fixture --locked
param(
    [string]$EvidenceDir = (Join-Path $PSScriptRoot '..\target\windows-native-capture-evidence'),
    [int]$ResizeAfterMs = 14000,
    [int]$WatchSeconds = 0,
    [switch]$NoCursor,
    [switch]$ClassifyOnly
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$EvidenceDir = [IO.Path]::GetFullPath($EvidenceDir)
$cli = Join-Path $repo 'target\debug\porthole.exe'
$server = Join-Path $repo 'target\debug\examples\windows_capture_server.exe'
$consumerExe = Join-Path $repo 'target\debug\examples\windows_native_capture_consumer.exe'
$fixture = Join-Path $repo 'target\debug\examples\capture_fixture.exe'
$log = Join-Path $EvidenceDir 'commands.txt'
$watchLog = Join-Path $EvidenceDir 'status-watch.log'
$transitionsLog = Join-Path $EvidenceDir 'status-transitions.jsonl'
$fixtureLog = Join-Path $EvidenceDir 'fixture-events.jsonl'
$framesDir = Join-Path $EvidenceDir 'frames'
$classificationLog = Join-Path $EvidenceDir 'classification.txt'

# Append (or with -Truncate, replace) one line, opening the file with shared
# read/write/delete access and retrying briefly; never throws.
function Add-SharedLine([string]$Path, [string]$Line, [switch]$Truncate) {
    $mode = if ($Truncate) { [IO.FileMode]::Create } else { [IO.FileMode]::Append }
    $bytes = [Text.Encoding]::UTF8.GetBytes($Line + "`r`n")
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try {
            $stream = [IO.File]::Open($Path, $mode, [IO.FileAccess]::Write, [IO.FileShare]'ReadWrite, Delete')
            try { $stream.Write($bytes, 0, $bytes.Length) } finally { $stream.Dispose() }
            return
        } catch {
            Start-Sleep -Milliseconds 50
        }
    }
    Write-Warning "could not write to ${Path}: $Line"
}

function Get-UnixSeconds { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0 }

$suffix = "p1cap-$([guid]::NewGuid().ToString('N').Substring(0, 12))"
$oldUser = $env:USERNAME
$oldToken = $env:PORTHOLE_AGENT_TOKEN
$identity = $null
$surface = $null
$closed = $false
$captureSession = $null
$consumer = $null
$fixturePid = $null
$fixtureStart = $null

function Write-Log([string]$Text) {
    $line = "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fff'))] $Text"
    Add-SharedLine $log $line
    Write-Host $line
}

function Invoke-Porthole([string[]]$Arguments, [string]$ExpectedError = '', [switch]$Secret) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = & $cli @Arguments 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = $previous
    $text = ($output | ForEach-Object { "$_" }) -join "`n"
    if ($ExpectedError) {
        if ($code -eq 0 -or $text -notmatch [regex]::Escape($ExpectedError)) { throw "Expected $ExpectedError; got exit ${code}: $text" }
        Write-Log "porthole $($Arguments -join ' ') -> $ExpectedError"
        return
    }
    if ($code -ne 0) { throw "porthole $($Arguments -join ' ') -> $text" }
    if ($Secret) {
        Write-Log "porthole $($Arguments -join ' ') -> (output withheld: contains a secret)"
    } else {
        Write-Log "porthole $($Arguments -join ' ') -> $text"
    }
    return $text
}

function Approve-Pending {
    $pending = (Invoke-Porthole @('agents', 'requests', '--json') | ConvertFrom-Json) | Where-Object agent_id -EQ $identity.agent_id
    if (!$pending) { throw 'Expected a pending request for the temporary identity.' }
    foreach ($request in $pending) {
        Invoke-Porthole @('agents', 'approve', $request.request_id, '--duration', 'persistent', '--json') | Out-Null
    }
}

function Get-Status {
    $status = Invoke-Porthole @('capture-session', 'status', $captureSession.session_id, '--json') | ConvertFrom-Json
    return $status
}

# The fixture process this run launched, if it is still that process.
function Get-OwnFixture {
    if (!$fixturePid) { return $null }
    $process = Get-Process -Id $fixturePid -ErrorAction SilentlyContinue
    if ($process -and $process.StartTime -eq $fixtureStart -and $process.Path -eq $fixture) { return $process }
    return $null
}

function Write-Classification {
    $lines = New-Object System.Collections.Generic.List[string]
    $framesFile = Join-Path $framesDir 'frames.jsonl'
    if (!(Test-Path -LiteralPath $framesFile)) {
        $lines.Add('classification: no frames.jsonl (the consumer did not record frames)')
    } else {
        $all = @(Get-Content -LiteralPath $framesFile)
        $verified = @($all | Where-Object { $_ -like '*"result":"verified"*' }).Count
        $odd = @($all | Where-Object { $_ -notlike '*"result":"verified"*' } | ForEach-Object { $_ | ConvertFrom-Json })
        $transitional = @($odd | Where-Object result -EQ 'transitional').Count
        $lines.Add("frames: $($all.Count); verified $verified; transitional $transitional; not uniform $($odd.Count - $transitional)")
        $fixtureEvents = @()
        if (Test-Path -LiteralPath $fixtureLog) {
            $fixtureEvents = @(Get-Content -LiteralPath $fixtureLog | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json })
        }
        $paints = @($fixtureEvents | Where-Object event -EQ 'paint' | Sort-Object { [double]$_.qpc_ns })
        $cursors = @($fixtureEvents | Where-Object event -EQ 'cursor' | Sort-Object { [double]$_.qpc_ns })
        # The timeline: status changes seen by the watch (its first entry is
        # the state when watching began, not a change), WTS session changes
        # and client-area resizes as the fixture saw them.
        $transitions = New-Object System.Collections.Generic.List[object]
        if (Test-Path -LiteralPath $transitionsLog) {
            $first = $true
            foreach ($entry in @(Get-Content -LiteralPath $transitionsLog | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json })) {
                if ($first) { $first = $false; continue }
                $label = $entry.status
                if ($entry.message -match 'desktop unavailable: ([^;]+)') { $label = "$label ($($Matches[1]))" }
                $transitions.Add([pscustomobject]@{ unix = [double]$entry.unix; label = "status $label" })
            }
        }
        foreach ($entry in @($fixtureEvents | Where-Object event -EQ 'session')) {
            $transitions.Add([pscustomobject]@{ unix = [double]$entry.wall; label = "wts $($entry.change)" })
        }
        foreach ($entry in @($fixtureEvents | Where-Object { $_.event -eq 'size' -and $_.kind -ne 1 } | Select-Object -Skip 1)) {
            $transitions.Add([pscustomobject]@{ unix = [double]$entry.wall; label = "fixture resize to $($entry.client -join 'x')" })
        }
        $transitions = @($transitions | Sort-Object unix)
        $lines.Add("timeline entries (status changes, WTS session changes, fixture resizes): $($transitions.Count); fixture paints: $($paints.Count); cursor events: $($cursors.Count)")
        $lines.Add("(a frame's time for paint and cursor matching is WGC's SystemRelativeTime, the composition time; 'last paint' is the fixture's newest repaint before it)")
        if ($odd.Count -gt 0) {
            $lines.Add('by kind: ' + ((@($odd | Group-Object kind | Sort-Object Count -Descending | ForEach-Object { "$($_.Name)=$($_.Count)" })) -join ', '))
            $details = New-Object System.Collections.Generic.List[string]
            $timing = @{}
            $cursorInside = 0
            foreach ($frame in $odd) {
                $wall = [double]$frame.wall
                $before = @($transitions | Where-Object { $_.unix -le $wall }) | Select-Object -Last 1
                $after = @($transitions | Where-Object { $_.unix -gt $wall }) | Select-Object -First 1
                $window = 'steady'
                $sinceBefore = if ($before) { $wall - $before.unix } else { [double]::MaxValue }
                $untilAfter = if ($after) { $after.unix - $wall } else { [double]::MaxValue }
                if ($sinceBefore -le 15 -and $sinceBefore -le $untilAfter) { $window = "within 15 s after $($before.label)" }
                elseif ($untilAfter -le 15) { $window = "within 15 s before $($after.label)" }
                elseif ($frame.result -eq 'transitional') { $window = 'new pool generation' }
                $timing[$window] = 1 + [int]$timing[$window]
                $frameQpc = [double]$frame.frame_qpc_ns
                $paint = @($paints | Where-Object { [double]$_.qpc_ns -le $frameQpc }) | Select-Object -Last 1
                $cursor = @($cursors | Where-Object { [double]$_.qpc_ns -le $frameQpc }) | Select-Object -Last 1
                $inside = $cursor -and $cursor.state -ne 'leave'
                if ($inside) { $cursorInside++ }
                $paintText = if ($paint) { "last paint $($paint.colour) $([math]::Round(($frameQpc - [double]$paint.qpc_ns) / 1e6, 1)) ms earlier" } else { 'no earlier paint' }
                $cursorText = if ($inside) { "cursor in window at client ($($cursor.client -join ','))" } else { 'cursor outside' }
                $top = ($frame.summary.top_colours | ForEach-Object { "$($_[0]) $($_[1])" }) -join ' / '
                $box = if ($frame.summary.other_bbox) { "other pixels $($frame.summary.other_pixels) in $($frame.summary.other_bbox.width)x$($frame.summary.other_bbox.height) at ($($frame.summary.other_bbox.x),$($frame.summary.other_bbox.y))" } else { '' }
                $near = @()
                if ($before) { $near += "+$([math]::Round($sinceBefore, 1)) s after $($before.label)" }
                if ($after) { $near += "$([math]::Round($untilAfter, 1)) s before $($after.label)" }
                $utc = [DateTimeOffset]::FromUnixTimeMilliseconds([int64]($wall * 1000)).UtcDateTime.ToString('HH:mm:ss.fff')
                $details.Add("  [$utc] $($frame.result) $($frame.kind): $top; $box; seq $($frame.sequence) fence $($frame.fence_value) slot $($frame.slot) gen $($frame.generation) epoch $($frame.epoch); $($near -join ', '); $paintText; $cursorText; png $($frame.png)")
            }
            $lines.Add('by timing: ' + ((@($timing.GetEnumerator() | Sort-Object Value -Descending | ForEach-Object { "$($_.Key)=$($_.Value)" })) -join ', '))
            $lines.Add("cursor inside the fixture window at the frame's capture time: $cursorInside of $($odd.Count)")
            $lines.Add('frames:')
            $lines.AddRange($details)
        }
    }
    foreach ($line in $lines) { Add-SharedLine $classificationLog $line }
    Write-Host '--- frame classification (classification.txt) ---'
    $shown = 0
    foreach ($line in $lines) {
        if ($line.StartsWith('  [') -and $shown++ -ge 40) { continue }
        Write-Host $line
    }
    if ($shown -gt 40) { Write-Host "  ... $($shown - 40) more in $classificationLog" }
}

if ($ClassifyOnly) {
    # Re-run the classification over an existing evidence directory.
    Remove-Item -LiteralPath $classificationLog -Force -ErrorAction SilentlyContinue
    Write-Classification
    return
}
foreach ($file in @($cli, $server, $consumerExe, $fixture)) { if (!(Test-Path -LiteralPath $file)) { throw "Build first: missing $file" } }
$session = (Get-Process -Id $PID).SessionId
if ($session -eq 0) { throw 'Run inside the interactive GUI session.' }
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
foreach ($stale in @($watchLog, $transitionsLog, $fixtureLog, $classificationLog)) { Remove-Item -LiteralPath $stale -Force -ErrorAction SilentlyContinue }
if (Test-Path -LiteralPath $framesDir) { Remove-Item -LiteralPath $framesDir -Recurse -Force }
Add-SharedLine $log "UTC: $([DateTime]::UtcNow.ToString('o')); session: $session; OS: $([Environment]::OSVersion.VersionString)" -Truncate
Add-SharedLine $log "revision: $(git -C $repo rev-parse HEAD) (worktree changes: $((git -C $repo status --porcelain | Measure-Object).Count))"

$daemon = Start-Process -FilePath $server -ArgumentList $suffix -WorkingDirectory $repo -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput (Join-Path $EvidenceDir 'daemon.stdout.log') -RedirectStandardError (Join-Path $EvidenceDir 'daemon.stderr.log')
Write-Log "isolated daemon pid $($daemon.Id) on \\.\pipe\porthole-$suffix"
try {
    $env:USERNAME = $suffix
    $env:PORTHOLE_AGENT_TOKEN = $null
    Start-Sleep -Milliseconds 1000
    Invoke-Porthole @('info') | Out-Null
    $identity = Invoke-Porthole @('agents', 'create', '--name', 'Porthole #186 temporary native capture check', '--json') -Secret | ConvertFrom-Json
    Write-Log "temporary identity $($identity.agent_id)"
    $env:PORTHOLE_AGENT_TOKEN = $identity.token
    $fixtureArgs = @("--arg=--resize-after-ms", "--arg=$ResizeAfterMs", '--arg=--resize', '--arg=480x300', '--arg=--max-seconds', "--arg=$(300 + $WatchSeconds)", '--arg=--log', "--arg=$fixtureLog")
    Invoke-Porthole (@('launch', '--app', $fixture) + $fixtureArgs + @('--json')) 'agent_permission_needed'
    Approve-Pending
    $launch = Invoke-Porthole (@('launch', '--app', $fixture) + $fixtureArgs + @('--require-fresh-surface', '--json')) | ConvertFrom-Json
    $surface = $launch.surface_id
    $launchedAt = Get-Date
    # The fixture is a child of this run's daemon; remember it by PID and
    # start time so cleanup can stop exactly that process.
    $own = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($daemon.Id) AND Name = 'capture_fixture.exe'" | Where-Object { $_.ExecutablePath -eq $fixture })
    if ($own.Count -eq 1) {
        $fixturePid = [int]$own[0].ProcessId
        $fixtureStart = (Get-Process -Id $fixturePid).StartTime
        Write-Log "fixture pid $fixturePid (child of daemon pid $($daemon.Id))"
    } else {
        Write-Log "warning: found $($own.Count) fixture processes under daemon pid $($daemon.Id); cleanup will rely on porthole close"
    }
    if ($launch.confidence -ne 'strong' -or $launch.surface_was_preexisting) { throw 'Launch did not prove a fresh owned window.' }
    $captureArgs = @('capture-session', 'surface', $surface, '--native')
    if ($NoCursor) { $captureArgs += '--no-cursor' }
    Invoke-Porthole ($captureArgs + @('--json')) 'agent_permission_needed'
    Approve-Pending
    $captureSession = Invoke-Porthole ($captureArgs + @('--json')) -Secret | ConvertFrom-Json
    Write-Log "native session $($captureSession.session_id): status $($captureSession.status), transport $($captureSession.native.transport_kind), endpoint $($captureSession.native.endpoint), cursor capture $(!$NoCursor)"
    # A consumer with the wrong token is refused before any Jackstay setup.
    $env:PORTHOLE_ATTACH_TOKEN = 'ptas_wrong'
    $impostor = Start-Process -FilePath $consumerExe -WindowStyle Hidden -PassThru -Wait `
        -ArgumentList @('--session-id', $captureSession.session_id, '--endpoint', $captureSession.native.endpoint, '--seconds', '5', '--log', (Join-Path $EvidenceDir 'wrong-token-consumer.log'))
    if ($impostor.ExitCode -eq 0 -or !(Select-String -LiteralPath (Join-Path $EvidenceDir 'wrong-token-consumer.log') -Pattern 'not authorized' -Quiet)) { throw 'a wrong attach token was not refused' }
    Write-Log 'wrong attach token refused (see wrong-token-consumer.log)'
    $env:PORTHOLE_ATTACH_TOKEN = $captureSession.native.attach_token
    $consumer = Start-Process -FilePath $consumerExe -WindowStyle Hidden -PassThru `
        -ArgumentList @('--session-id', $captureSession.session_id, '--endpoint', $captureSession.native.endpoint, '--seconds', "$(90 + $WatchSeconds)", '--log', (Join-Path $EvidenceDir 'consumer.log'), '--frames-dir', $framesDir, '--max-saved', '200') `
        -RedirectStandardOutput (Join-Path $EvidenceDir 'consumer.stdout.log') -RedirectStandardError (Join-Path $EvidenceDir 'consumer.stderr.log')
    $null = $consumer.Handle  # cache the handle so ExitCode is readable later
    $env:PORTHOLE_ATTACH_TOKEN = $null
    Write-Log "consumer pid $($consumer.Id) (daemon pid $($daemon.Id))"

    Start-Sleep -Milliseconds 1500
    $status = Get-Status
    Write-Log "status before resize: $($status.status) $($status.width)x$($status.height): $($status.status_message)"
    if ($status.status -ne 'ready') { throw "expected ready, got $($status.status)" }

    $wait = [Math]::Max(0, $ResizeAfterMs - ((Get-Date) - $launchedAt).TotalMilliseconds + 2500)
    Start-Sleep -Milliseconds $wait
    $status = Get-Status
    Write-Log "status after resize: $($status.status) $($status.width)x$($status.height): $($status.status_message)"
    if ($status.status -ne 'ready' -or $status.width -ne 480 -or $status.height -ne 300) { throw "expected ready at 480x300 after the resize" }

    if ($WatchSeconds -gt 0) {
        Write-Log "watching for $WatchSeconds s: lock/unlock or disconnect/reconnect RDP now; changes go to $watchLog"
        $last = ''
        $watchEnd = (Get-Date).AddSeconds($WatchSeconds)
        while ((Get-Date) -lt $watchEnd) {
            $previous = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            $raw = & $cli capture-session status $captureSession.session_id --json 2>&1
            $ErrorActionPreference = $previous
            $wts = (quser 2>$null | Select-String -SimpleMatch ([Environment]::UserName)) -join ' '
            $stamp = "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fff'))]"
            $unix = Get-UnixSeconds
            try {
                $now = ($raw -join "`n") | ConvertFrom-Json
                $line = "$($now.status) $($now.width)x$($now.height): $($now.status_message -replace 'published=\d+, dropped=\d+', '')"
                if ($line -ne $last) {
                    Add-SharedLine $watchLog "$stamp $($now.status) $($now.width)x$($now.height): $($now.status_message) | quser: $wts"
                    Add-SharedLine $transitionsLog (@{ unix = $unix; utc = $stamp.Trim('[', ']'); status = $now.status; message = $now.status_message; quser = $wts } | ConvertTo-Json -Compress)
                    $last = $line
                }
            } catch {
                Add-SharedLine $watchLog "$stamp status query failed: $raw | quser: $wts"
            }
            Start-Sleep -Milliseconds 500
        }
        Write-Log 'watch finished'
    }

    Invoke-Porthole @('close', $surface) 'agent_permission_needed'
    Approve-Pending
    Invoke-Porthole @('close', $surface) | Out-Null
    $closed = $true
    $deadline = (Get-Date).AddSeconds(5)
    do {
        Start-Sleep -Milliseconds 200
        $status = Get-Status
    } while ($status.status -notin @('failed') -and (Get-Date) -lt $deadline)
    Write-Log "status after window close: $($status.status): $($status.status_message)"
    if ($status.status -ne 'failed' -or $status.status_message -notmatch 'captured window closed') { throw 'expected the session to fail on window close' }
    if (!$consumer.WaitForExit(15000)) { throw 'consumer did not exit after the session failed' }
    Write-Log "consumer exit code $($consumer.ExitCode)"
    if ($consumer.ExitCode -ne 0) { throw 'consumer did not verify frames, or saw non-uniform frames (see the classification below)' }
    $deadline = (Get-Date).AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 250
        $status = Get-Status
    } while ($status.status_message -notmatch 'retired' -and (Get-Date) -lt $deadline)
    Write-Log "status after drain: $($status.status): $($status.status_message)"
    Write-Output "PASS: native capture, D3D11 consumer, resize and window close. Evidence: $EvidenceDir"
} finally {
    if ($consumer -and !$consumer.HasExited) { Stop-Process -Id $consumer.Id }
    if ($captureSession) {
        try { Invoke-Porthole @('capture-session', 'close', $captureSession.session_id) | Out-Null } catch { Write-Warning "$_" }
    }
    if ($surface -and !$closed) {
        try { Invoke-Porthole @('close', $surface) | Out-Null } catch { Write-Log "porthole close during cleanup failed: $_" }
    }
    # A refused close (agent_permission_needed) or an early failure must not
    # leave the fixture window behind: stop the one process this run launched.
    $leftover = Get-OwnFixture
    if ($leftover) {
        Stop-Process -Id $leftover.Id -Force
        Write-Log "stopped fixture pid $($leftover.Id) left running"
    }
    if ($identity) { Invoke-Porthole @('agents', 'revoke', $identity.agent_id, '--json') | Out-Null }
    $env:PORTHOLE_AGENT_TOKEN = $oldToken
    $env:USERNAME = $oldUser
    if (!$daemon.HasExited) { Stop-Process -Id $daemon.Id }
    Write-Log 'cleanup done: identity revoked, own daemon stopped'
    try { Write-Classification } catch { Write-Warning "classification failed: $_" }
}
