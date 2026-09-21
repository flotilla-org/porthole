param(
    [Parameter(Mandatory=$true)][string]$PortholeExecutable,
    [Parameter(Mandatory=$true)][string]$StateDirectory
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'pipe-client.ps1')
$PortholeExecutable = (Get-Item -LiteralPath $PortholeExecutable).FullName
$StateDirectory = [IO.Path]::GetFullPath($StateDirectory)
$sessionId = (Get-Process -Id $PID).SessionId
if ($sessionId -eq 0 -or -not (Get-Process explorer -ErrorAction SilentlyContinue | Where-Object SessionId -eq $sessionId)) {
    throw 'Porthole startup requires the existing interactive GUI login'
}
[IO.Directory]::CreateDirectory($StateDirectory) | Out-Null
$mutex = [Threading.Mutex]::new($false, 'Local\PortholeVesselStartup')
$locked = $false
try {
    try { $locked = $mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $locked = $true }
    if (-not $locked) { throw 'Another Porthole startup supervisor is already running' }
    $existing = @(Get-Process portholed -ErrorAction SilentlyContinue)
    $reused = $existing.Count -gt 0
    if ($reused) {
        if ($existing.Count -ne 1 -or $existing[0].SessionId -ne $sessionId -or $existing[0].Path -ne $PortholeExecutable) {
            throw 'An unrelated or different-session Porthole daemon already exists; refusing replacement'
        }
        $daemon = $existing[0]
    } else {
        $daemon = Start-Process -FilePath $PortholeExecutable -WorkingDirectory (Split-Path $PortholeExecutable) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $StateDirectory 'portholed.stdout.log') -RedirectStandardError (Join-Path $StateDirectory 'portholed.stderr.log')
    }
    $started = $daemon.StartTime.ToUniversalTime().ToString('o')
    $state = @{
        supervisor_pid=$PID; supervisor_started=(Get-Process -Id $PID).StartTime.ToUniversalTime().ToString('o')
        portholed_pid=$daemon.Id; portholed_started=$started; executable=$PortholeExecutable
        windows_session=$sessionId; reused=$reused; status='starting'; ready_utc=$null
    }
    $state | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $StateDirectory 'startup.json') -Encoding UTF8
    $ready = $false
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $daemon.Refresh()
        if ($daemon.HasExited) { throw 'Porthole exited during startup; inspect startup logs' }
        try { if ((Invoke-PortholeJson GET '/info').Status -eq 200) { $ready = $true; break } } catch { }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $ready) { throw 'Porthole did not become ready; no replacement will be started' }
    $state.status = 'ready'
    $state.ready_utc = [DateTime]::UtcNow.ToString('o')
    $state | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $StateDirectory 'startup.json') -Encoding UTF8
    # Keep the scheduled action alive: IgnoreNew then prevents duplicate task
    # instances. Never restart a failed daemon or implicitly launch an agent.
    $daemon.WaitForExit()
    $state.status = 'exited'
    $state | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $StateDirectory 'startup.json') -Encoding UTF8
} catch {
    if ($state) {
        $state.status = 'failed'
        $state | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $StateDirectory 'startup.json') -Encoding UTF8
    }
    throw
} finally {
    if ($locked) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
