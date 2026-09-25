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
# the workstation or disconnects RDP.
#
# -WatchSeconds N keeps the session running for N seconds after the resize
# check and logs every status change to status-watch.log (with the raw WTS
# session state), for a person to lock/unlock or disconnect/reconnect RDP
# meanwhile. See docs/2026-09-25-windows-native-capture-evidence.md.
#
# Build first:
#   cargo build -p porthole --locked
#   cargo build -p portholed --examples --locked
#   cargo build -p porthole-adapter-windows --example capture_fixture --locked
param(
    [string]$EvidenceDir = (Join-Path $PSScriptRoot '..\target\windows-native-capture-evidence'),
    [int]$ResizeAfterMs = 14000,
    [int]$WatchSeconds = 0
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$EvidenceDir = [IO.Path]::GetFullPath($EvidenceDir)
$cli = Join-Path $repo 'target\debug\porthole.exe'
$server = Join-Path $repo 'target\debug\examples\windows_capture_server.exe'
$consumerExe = Join-Path $repo 'target\debug\examples\windows_native_capture_consumer.exe'
$fixture = Join-Path $repo 'target\debug\examples\capture_fixture.exe'
foreach ($file in @($cli, $server, $consumerExe, $fixture)) { if (!(Test-Path -LiteralPath $file)) { throw "Build first: missing $file" } }
$session = (Get-Process -Id $PID).SessionId
if ($session -eq 0) { throw 'Run inside the interactive GUI session.' }
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$log = Join-Path $EvidenceDir 'commands.txt'
Set-Content -LiteralPath $log -Value "UTC: $([DateTime]::UtcNow.ToString('o')); session: $session; OS: $([Environment]::OSVersion.VersionString)"
Add-Content -LiteralPath $log -Value "revision: $(git -C $repo rev-parse HEAD) (worktree changes: $((git -C $repo status --porcelain | Measure-Object).Count))"

$suffix = "p1cap-$([guid]::NewGuid().ToString('N').Substring(0, 12))"
$oldUser = $env:USERNAME
$oldToken = $env:PORTHOLE_AGENT_TOKEN
$identity = $null
$surface = $null
$closed = $false
$captureSession = $null
$consumer = $null

function Write-Log([string]$Text) {
    $line = "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fff'))] $Text"
    Add-Content -LiteralPath $log -Value $line
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
    $fixtureArgs = @("--arg=--resize-after-ms", "--arg=$ResizeAfterMs", '--arg=--resize', '--arg=480x300', '--arg=--max-seconds', "--arg=$(300 + $WatchSeconds)")
    Invoke-Porthole (@('launch', '--app', $fixture) + $fixtureArgs + @('--json')) 'agent_permission_needed'
    Approve-Pending
    $launch = Invoke-Porthole (@('launch', '--app', $fixture) + $fixtureArgs + @('--require-fresh-surface', '--json')) | ConvertFrom-Json
    $surface = $launch.surface_id
    if ($launch.confidence -ne 'strong' -or $launch.surface_was_preexisting) { throw 'Launch did not prove a fresh owned window.' }
    $launchedAt = Get-Date
    Invoke-Porthole @('capture-session', 'surface', $surface, '--native', '--json') 'agent_permission_needed'
    Approve-Pending
    $captureSession = Invoke-Porthole @('capture-session', 'surface', $surface, '--native', '--json') -Secret | ConvertFrom-Json
    Write-Log "native session $($captureSession.session_id): status $($captureSession.status), transport $($captureSession.native.transport_kind), endpoint $($captureSession.native.endpoint)"
    # A consumer with the wrong token is refused before any Jackstay setup.
    $env:PORTHOLE_ATTACH_TOKEN = 'ptas_wrong'
    $impostor = Start-Process -FilePath $consumerExe -WindowStyle Hidden -PassThru -Wait `
        -ArgumentList @('--session-id', $captureSession.session_id, '--endpoint', $captureSession.native.endpoint, '--seconds', '5', '--log', (Join-Path $EvidenceDir 'wrong-token-consumer.log'))
    if ($impostor.ExitCode -eq 0 -or !(Select-String -LiteralPath (Join-Path $EvidenceDir 'wrong-token-consumer.log') -Pattern 'not authorized' -Quiet)) { throw 'a wrong attach token was not refused' }
    Write-Log 'wrong attach token refused (see wrong-token-consumer.log)'
    $env:PORTHOLE_ATTACH_TOKEN = $captureSession.native.attach_token
    $consumer = Start-Process -FilePath $consumerExe -WindowStyle Hidden -PassThru `
        -ArgumentList @('--session-id', $captureSession.session_id, '--endpoint', $captureSession.native.endpoint, '--seconds', "$(90 + $WatchSeconds)", '--log', (Join-Path $EvidenceDir 'consumer.log')) `
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
        $watchLog = Join-Path $EvidenceDir 'status-watch.log'
        Write-Log "watching for $WatchSeconds s: lock/unlock or disconnect/reconnect RDP now; changes go to $watchLog"
        $last = ''
        $watchEnd = (Get-Date).AddSeconds($WatchSeconds)
        while ((Get-Date) -lt $watchEnd) {
            $previous = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            $raw = & $cli capture-session status $captureSession.session_id --json 2>&1
            $ErrorActionPreference = $previous
            $wts = (quser 2>$null | Select-String -SimpleMatch ([Environment]::UserName)) -join ' '
            try {
                $now = ($raw -join "`n") | ConvertFrom-Json
                $line = "$($now.status) $($now.width)x$($now.height): $($now.status_message -replace 'published=\d+, dropped=\d+', '')"
                if ($line -ne $last) {
                    Add-Content -LiteralPath $watchLog -Value "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fff'))] $($now.status) $($now.width)x$($now.height): $($now.status_message) | quser: $wts"
                    $last = $line
                }
            } catch {
                Add-Content -LiteralPath $watchLog -Value "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fff'))] status query failed: $raw | quser: $wts"
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
    if ($consumer.ExitCode -ne 0) { throw 'consumer did not verify frames' }
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
        try { Invoke-Porthole @('close', $surface) | Out-Null } catch { Write-Warning "Test surface $surface still needs cleanup: $_" }
    }
    if ($identity) { Invoke-Porthole @('agents', 'revoke', $identity.agent_id, '--json') | Out-Null }
    $env:PORTHOLE_AGENT_TOKEN = $oldToken
    $env:USERNAME = $oldUser
    if (!$daemon.HasExited) { Stop-Process -Id $daemon.Id }
    Write-Log 'cleanup done: identity revoked, own daemon stopped'
}
