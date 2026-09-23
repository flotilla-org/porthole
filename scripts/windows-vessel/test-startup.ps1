param([Parameter(Mandatory=$true)][string]$PortholeExecutable)
$ErrorActionPreference = 'Stop'
$PortholeExecutable = (Get-Item -LiteralPath $PortholeExecutable).FullName
$daemons = @(Get-Process portholed -ErrorAction SilentlyContinue)
if ($daemons.Count -ne 1 -or $daemons[0].Path -ne $PortholeExecutable -or $daemons[0].SessionId -ne (Get-Process -Id $PID).SessionId) { throw 'Test requires the existing GUI Porthole daemon at the requested path in this Windows session' }
$daemon = $daemons[0]
$started = $daemon.StartTime.ToUniversalTime().ToString('o')
$root = Join-Path ([IO.Path]::GetTempPath()) ('porthole-startup-test-' + [Guid]::NewGuid().ToString('N'))
$stateDirectory = Join-Path $root "state [spaces] & 'quotes'"
$taskName = 'Porthole startup test ' + [Guid]::NewGuid().ToString('N')
$registration = @{PortholeExecutable=$PortholeExecutable; StateDirectory=$stateDirectory; TaskName=$taskName}
$register = Join-Path $PSScriptRoot 'register-startup.ps1'
$state = $null
try {
    & $register @registration
    $xml = Export-ScheduledTask -TaskName $taskName
    & $register @registration
    if ((Export-ScheduledTask -TaskName $taskName) -cne $xml) { throw 'Idempotent registration changed the task' }
    try {
        & $register -PortholeExecutable $PortholeExecutable -StateDirectory (Join-Path $root 'different') -TaskName $taskName
        throw 'Expected conflicting configuration rejection'
    } catch { if ($_.Exception.Message -notmatch 'occupied') { throw } }
    if ((Export-ScheduledTask -TaskName $taskName) -cne $xml) { throw 'Rejected registration changed the task' }
    Start-ScheduledTask -TaskName $taskName
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        Start-Sleep -Milliseconds 200
        $statePath = Join-Path $stateDirectory 'startup.json'
        if (Test-Path -LiteralPath $statePath) {
            try { $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json } catch { continue }
            if ($state.status -eq 'ready') { break }
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $state -or $state.status -ne 'ready') { throw 'Scheduled supervisor did not become ready' }
    if (-not $state.reused -or $state.portholed_pid -ne $daemon.Id -or $state.portholed_started -ne $started -or $state.windows_session -ne $daemon.SessionId) {
        throw 'Scheduled startup replaced the daemon or used the wrong GUI session'
    }
    Start-ScheduledTask -TaskName $taskName
    Start-Sleep -Seconds 2
    $again = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if ($again.supervisor_pid -ne $state.supervisor_pid -or $again.supervisor_started -ne $state.supervisor_started) { throw 'Repeated start replaced the supervisor' }
    if ((Get-ScheduledTask -TaskName $taskName).State -ne 'Running') { throw 'Task supervision did not remain running' }
    & $register @registration -Remove
    if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) { throw 'Task registration remains' }
    $daemon.Refresh()
    if ($daemon.HasExited -or $daemon.StartTime.ToUniversalTime().ToString('o') -ne $started) { throw 'Unregistration stopped or replaced the daemon' }
    # Exercise failure reporting through the real supervisor with a failing
    # readiness collaborator. Reuse the existing daemon; do not launch another.
    $failureScripts = Join-Path $root 'failure-scripts'
    [IO.Directory]::CreateDirectory($failureScripts) | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'startup-entry.ps1') -Destination (Join-Path $failureScripts 'startup-entry.ps1')
    'function Invoke-PortholeJson { throw "test readiness unavailable" }' | Set-Content -LiteralPath (Join-Path $failureScripts 'pipe-client.ps1') -Encoding UTF8
    $failureState = Join-Path $root 'failure-state'
    try {
        & (Join-Path $failureScripts 'startup-entry.ps1') -PortholeExecutable $PortholeExecutable -StateDirectory $failureState
        throw 'Expected readiness failure'
    } catch { if ($_.Exception.Message -notmatch 'did not become ready') { throw } }
    $failed = Get-Content -LiteralPath (Join-Path $failureState 'startup.json') -Raw | ConvertFrom-Json
    if ($failed.status -ne 'failed' -or $failed.error -notmatch 'did not become ready' -or $failed.portholed_pid -ne $daemon.Id) {
        throw 'Readiness failure lost its error or daemon identity'
    }
    Write-Output 'PASS: failed readiness records its reason without replacing the daemon'
    Write-Output "PASS: idempotent registration, collision refusal, GUI reuse, duplicate prevention and safe removal; evidence: $stateDirectory"
} finally {
    if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) { & $register @registration -Remove }
    # This test reused a daemon. Stop only its own verified supervisor process,
    # never a task process tree or the daemon/agent it observed.
    if ($state) {
        $supervisor = Get-Process -Id $state.supervisor_pid -ErrorAction SilentlyContinue
        if ($supervisor -and $supervisor.StartTime.ToUniversalTime().ToString('o') -eq $state.supervisor_started) {
            Stop-Process -Id $supervisor.Id
        }
    }
}
