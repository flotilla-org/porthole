param(
    [Parameter(Mandatory=$true)][string]$RunDirectory,
    [Parameter(Mandatory=$true)][string]$PortholeBinDirectory,
    [Parameter(Mandatory=$true)][string]$CleatExecutable,
    [Parameter(Mandatory=$true)][string]$CodexCommand,
    [Parameter(Mandatory=$true)][string]$Workspace,
    [string]$Server = 'beaufort-parity',
    [string]$Session = 'coding-agent',
    [switch]$UseExistingDaemon
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'pipe-client.ps1')
$RunDirectory = [IO.Path]::GetFullPath($RunDirectory)
$PortholeBinDirectory = [IO.Path]::GetFullPath($PortholeBinDirectory)
$CleatExecutable = [IO.Path]::GetFullPath($CleatExecutable)
$CodexCommand = [IO.Path]::GetFullPath($CodexCommand)
$Workspace = [IO.Path]::GetFullPath($Workspace)
$statePath = Join-Path $RunDirectory 'state.json'
if (Test-Path -LiteralPath $statePath) {
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    $config = Get-Content -LiteralPath (Join-Path $RunDirectory 'config.json') -Raw | ConvertFrom-Json
    $entryPath = Join-Path $RunDirectory 'agent-entry.json'
    if (-not (Test-Path -LiteralPath $entryPath)) { throw 'Existing run has no verified agent entry; inspect it rather than replacing it' }
    $entry = Get-Content -LiteralPath $entryPath -Raw | ConvertFrom-Json
    $live = Get-Process -Id $entry.pid -ErrorAction SilentlyContinue
    if (-not $live -or $live.StartTime.ToUniversalTime().ToString('o') -ne $entry.started) { throw 'Existing run ended; explicitly clean it up before starting a new run' }
    $daemon = Get-Process -Id $state.portholed_pid -ErrorAction SilentlyContinue
    if (-not $daemon -or $daemon.StartTime.ToUniversalTime().ToString('o') -ne $state.portholed_started -or $daemon.Path -ne (Join-Path (Split-Path $config.porthole) 'portholed.exe')) { throw 'Owned Porthole daemon is unavailable or replaced' }
    $inspection = & $config.cleat --runtime-root $config.runtime --server $config.server inspect $config.session --json
    if ($LASTEXITCODE -ne 0) { throw 'Existing Cleat session could not be inspected' }
    $inspection = $inspection | ConvertFrom-Json
    if ($inspection.session.state -ne 'running' -or $entry.agent_id -ne $state.agent_id) { throw 'Existing agent session is not reusable' }
    Write-Output "REUSED: $($config.server)/$($config.session), agent entry PID $($entry.pid), identity $($state.agent_id)"
    exit 0
}
if (Test-Path -LiteralPath (Join-Path $RunDirectory 'config.json')) { throw 'Incomplete prior run exists; inspect and clean it up first' }
$sessionId = (Get-Process -Id $PID).SessionId
if ($sessionId -eq 0 -or -not (Get-Process explorer -ErrorAction SilentlyContinue | Where-Object SessionId -eq $sessionId)) { throw 'Start from the existing GUI login' }
$existingDaemons = @(Get-Process portholed -ErrorAction SilentlyContinue)
$existingDaemon = $null
if ($UseExistingDaemon) {
    if ($existingDaemons.Count -ne 1 -or $existingDaemons[0].SessionId -ne $sessionId -or
        $existingDaemons[0].Path -ne (Join-Path $PortholeBinDirectory 'portholed.exe')) {
        throw 'Expected exactly one Porthole daemon from the requested binary directory in this GUI session'
    }
    $existingDaemon = $existingDaemons[0]
} elseif ($existingDaemons.Count -gt 0) {
    throw 'A Porthole daemon already exists; use -UseExistingDaemon to explicitly reuse the matching GUI-session daemon'
}
foreach ($file in @((Join-Path $PortholeBinDirectory 'porthole.exe'),(Join-Path $PortholeBinDirectory 'portholed.exe'),(Join-Path $PortholeBinDirectory 'examples\desktop_fixture.exe'),$CleatExecutable,$CodexCommand)) {
    if (-not (Test-Path -LiteralPath $file)) { throw "Missing prerequisite: $file" }
}
New-Item -ItemType Directory -Force -Path $RunDirectory,$Workspace | Out-Null
$config = @{porthole=(Join-Path $PortholeBinDirectory 'porthole.exe'); cleat=$CleatExecutable; codex=$CodexCommand; fixture=(Join-Path $PortholeBinDirectory 'examples\desktop_fixture.exe'); workspace=$Workspace; runtime=(Join-Path $RunDirectory 'cleat-state'); server=$Server; session=$Session}
$config | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $RunDirectory 'config.json')
$proof = Join-Path $PSScriptRoot 'desktop-proof.ps1'
$task = @"
Run the following PowerShell script once, passing the stated run directory:
powershell -NoProfile -File "$proof" -RunDirectory "$RunDirectory"

This is the authorized native Windows desktop acceptance task. It launches a test-owned editor through Porthole, types two lines, saves a PNG and closes that editor. The operator is approving this run's own pending surface grants; the script waits for them. Do not approve permissions yourself, read credential files, print environment variables or tokens, change any other application, or launch further agents. If the script fails, report the failure and wait. If it passes, report the result and remain available for further work. Do not end this Codex session.
"@
$task | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $Workspace 'request.md')
$daemon = $existingDaemon
if (-not $daemon) {
    $daemon = Start-Process -FilePath (Join-Path $PortholeBinDirectory 'portholed.exe') -WorkingDirectory $Workspace -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $RunDirectory 'daemon.stdout.log') -RedirectStandardError (Join-Path $RunDirectory 'daemon.stderr.log')
}
if ($daemon.SessionId -ne $sessionId) { throw 'Porthole started in a different Windows session' }
$identity = $null
try {
    $ready = $false
    for ($i=0; $i -lt 30; $i++) {
        try { $info = Invoke-PortholeJson GET '/info'; if ($info.Status -eq 200) { $ready=$true; break } } catch { }
        Start-Sleep -Milliseconds 200
    }
    if (-not $ready) { throw 'Porthole did not become ready' }
    $created = Invoke-PortholeJson POST '/agent-identities' @{display_name="Beaufort $Server/$Session"}
    if ($created.Status -ne 200 -and $created.Status -ne 201) { throw "Identity creation failed with HTTP $($created.Status)" }
    $identity = $created.Body
    $state = @{agent_id=$identity.agent_id; portholed_pid=$daemon.Id; portholed_started=$daemon.StartTime.ToUniversalTime().ToString('o'); windows_session=$sessionId; created_utc=[DateTime]::UtcNow.ToString('o')}
    $state | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath $statePath
    $consoleLauncher = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
    $request = @{kind=@{type='process'; app=$consoleLauncher; args=@('-NoProfile','-File',(Join-Path $PSScriptRoot 'console-entry.ps1'),'-RunDirectory',$RunDirectory); cwd=$Workspace; env=@{PORTHOLE_AGENT_TOKEN=$identity.token; PORTHOLE_AGENT_ID=$identity.agent_id; PARITY_RUN_DIR=$RunDirectory; CLEAT_RUNTIME_DIR=$config.runtime; CLEAT_DAEMON=$Server}}; require_fresh_surface=$true; timeout_ms=10000}
    $launch = Invoke-PortholeJson POST '/launches' $request $identity.token
    if ($launch.Status -eq 403) {
        $pending = Invoke-PortholeJson GET '/agent-permissions/requests'
        $allowed = @($pending.Body | Where-Object { $_.agent_id -eq $identity.agent_id -and $_.status -eq 'pending' -and $_.description.operation.kind -eq 'launch' -and $_.description.operation.application -eq $consoleLauncher })
        if ($allowed.Count -ne 1) { throw 'Expected exactly one console-launch permission request' }
        $approval = Invoke-PortholeJson POST "/agent-permissions/requests/$($allowed[0].request_id)/approve" @{duration=@{type='persistent'}; target=$allowed[0].target; actions=@($allowed[0].actions)}
        if ($approval.Status -ne 200) { throw 'Launch grant approval failed' }
        $launch = Invoke-PortholeJson POST '/launches' $request $identity.token
    }
    if ($launch.Status -lt 200 -or $launch.Status -ge 300) {
        # Errors have no request environment. Preserve native diagnostics for recovery.
        $launch.Body | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $RunDirectory 'launch-error.json')
        throw "Console launch failed with HTTP $($launch.Status); inspect launch-error.json and any test-owned process before retrying"
    }
    $launch.Body | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $RunDirectory 'terminal-launch.json')
    Write-Output "STARTED: $Server/$Session; identity $($identity.agent_id); surface $($launch.Body.surface_id); Session $sessionId"
} catch {
    if ($identity) {
        try {
            $revoked = Invoke-PortholeJson POST "/agent-identities/$($identity.agent_id)/revoke" @{}
            if ($revoked.Status -lt 200 -or $revoked.Status -ge 300) { throw "HTTP $($revoked.Status)" }
        } catch { Write-Warning "Identity cleanup failed; explicitly revoke $($identity.agent_id): $_" }
    }
    throw
} finally {
    # Credentials exist only in the request and child process environments.
    $request=$null; $created=$null; $identity=$null
}
