param([string]$EvidenceDir = (Join-Path $PSScriptRoot '..\target\windows-117-evidence'))
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$EvidenceDir = [IO.Path]::GetFullPath($EvidenceDir)
$cli = Join-Path $repo 'target\debug\porthole.exe'
$daemonExe = Join-Path $repo 'target\debug\portholed.exe'
$fixture = Join-Path $repo 'target\debug\examples\desktop_fixture.exe'
foreach ($file in @($cli, $daemonExe, $fixture)) { if (!(Test-Path -LiteralPath $file)) { throw "Build first: missing $file" } }
$session = (Get-Process -Id $PID).SessionId
if ($session -eq 0 -or !(Get-Process explorer | Where-Object SessionId -EQ $session)) { throw 'Run inside the existing explorer GUI session.' }
if (Get-Process portholed -ErrorAction SilentlyContinue) { throw 'An existing portholed is running; stop only your own daemon before this isolated smoke.' }
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$log = Join-Path $EvidenceDir 'commands.txt'
Set-Content -LiteralPath $log -Value "UTC: $([DateTime]::UtcNow.ToString('o')); session: $session; OS: $([Environment]::OSVersion.VersionString)"
Add-Content -LiteralPath $log -Value (git -C $repo rev-parse HEAD)
$oldToken = $env:PORTHOLE_AGENT_TOKEN
$env:PORTHOLE_AGENT_TOKEN = $null
$identity = $null
$surface = $null
$closed = $false

function Invoke-Porthole([string[]]$Arguments, [string]$ExpectedError = '') {
    # Capture output before returning it: token creation is never logged or printed.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = & $cli @Arguments 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = $previous
    $text = ($output | ForEach-Object { "$_" }) -join "`n"
    if ($ExpectedError) {
        if ($code -eq 0 -or $text -notmatch [regex]::Escape($ExpectedError)) { throw "Expected $ExpectedError; got exit ${code}: $text" }
        Add-Content -LiteralPath $log -Value "porthole $($Arguments -join ' ') -> $ExpectedError"
        return
    }
    if ($code -ne 0) { throw "porthole $($Arguments -join ' ') -> $text" }
    if ($Arguments[0] -ne 'agents' -or $Arguments[1] -ne 'create') {
        Add-Content -LiteralPath $log -Value "porthole $($Arguments -join ' ') -> $text"
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

$daemon = Start-Process -FilePath $daemonExe -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $EvidenceDir 'daemon.stdout.log') -RedirectStandardError (Join-Path $EvidenceDir 'daemon.stderr.log')
try {
    if ($daemon.SessionId -ne $session) { throw 'Daemon is in the wrong GUI session.' }
    Start-Sleep -Milliseconds 800
    $info = Invoke-Porthole @('info')
    if ($info -notmatch 'windows') { throw "Expected native Windows adapter: $info" }
    Invoke-Porthole @('launch', '--app', $fixture, '--json') 'agent_identity_required'
    $identity = Invoke-Porthole @('agents', 'create', '--name', 'Windows #117 temporary validation', '--json') | ConvertFrom-Json
    $env:PORTHOLE_AGENT_TOKEN = $identity.token
    Invoke-Porthole @('launch', '--app', $fixture, '--json') 'agent_permission_needed'
    Approve-Pending
    $launch = Invoke-Porthole @('launch', '--app', $fixture, '--require-fresh-surface', '--json') | ConvertFrom-Json
    $surface = $launch.surface_id
    if ($launch.confidence -ne 'strong' -or $launch.surface_was_preexisting) { throw 'Launch did not prove a fresh owned window.' }
    Invoke-Porthole @('focus', $surface) 'agent_permission_needed'
    Approve-Pending
    Invoke-Porthole @('focus', $surface) | Out-Null
    Invoke-Porthole @('text', $surface, 'Porthole #117: native Windows input') | Out-Null
    Invoke-Porthole @('key', $surface, '--key', 'Enter') | Out-Null
    Invoke-Porthole @('text', $surface, 'Named pipe + token/grant + HWND correlation + PNG') | Out-Null
    Invoke-Porthole @('key', $surface, '--key', 'KeyA', '--mod', 'ctrl') | Out-Null
    Invoke-Porthole @('key', $surface, '--key', 'End') | Out-Null
    $png = Join-Path $EvidenceDir 'visible-input.png'
    Invoke-Porthole @('screenshot', $surface, '--out', $png) 'agent_permission_needed'
    Approve-Pending
    Start-Sleep -Milliseconds 300
    Invoke-Porthole @('screenshot', $surface, '--out', $png) | Out-Null
    Invoke-Porthole @('wait', $surface, '--condition', 'stable', '--timeout-ms', '100') 'adapter_unsupported'
    Invoke-Porthole @('focus', 'surf_missing_117') 'surface_not_found'
    Invoke-Porthole @('launch', '--app', (Join-Path $EvidenceDir 'does-not-exist.exe'), '--json') 'launch_correlation_failed'
    Invoke-Porthole @('launch', '--app', $fixture, '--arg=--exit-without-window', '--json') 'launch_correlation_failed'
    Invoke-Porthole @('close', $surface) 'agent_permission_needed'
    Approve-Pending
    Invoke-Porthole @('close', $surface) | Out-Null
    $closed = $true
    Invoke-Porthole @('focus', $surface) 'surface_dead'
    # The second window exits via its native Alt+F4 handling. The daemon has
    # not marked this handle dead via /close, so focus must detect native death.
    $launch = Invoke-Porthole @('launch', '--app', $fixture, '--json') | ConvertFrom-Json
    $surface = $launch.surface_id
    $closed = $false
    Invoke-Porthole @('focus', $surface) 'agent_permission_needed'
    Approve-Pending
    Invoke-Porthole @('focus', $surface) | Out-Null
    Invoke-Porthole @('key', $surface, '--key', 'F4', '--mod', 'alt') | Out-Null
    Start-Sleep -Milliseconds 200
    Invoke-Porthole @('focus', $surface) 'surface_dead'
    $closed = $true
    Add-Content -LiteralPath $log -Value "PNG SHA256: $((Get-FileHash -LiteralPath $png -Algorithm SHA256).Hash)"
    Write-Output "PASS: live named-pipe workflow; inspect $png. Evidence: $log"
} finally {
    if ($surface -and !$closed) {
        try {
            $previous = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            $cleanup = & $cli close $surface 2>&1
            $cleanupCode = $LASTEXITCODE
            $ErrorActionPreference = $previous
            if ($cleanupCode -ne 0 -and "$cleanup" -match 'agent_permission_needed') {
                Approve-Pending
                Invoke-Porthole @('close', $surface) | Out-Null
            } elseif ($cleanupCode -ne 0) { throw "$cleanup" }
        } catch { Write-Warning "Test surface $surface still needs cleanup: $_" }
    }
    if ($identity) { Invoke-Porthole @('agents', 'revoke', $identity.agent_id, '--json') | Out-Null }
    $env:PORTHOLE_AGENT_TOKEN = $oldToken
    if (!$daemon.HasExited) { Stop-Process -Id $daemon.Id }
}
