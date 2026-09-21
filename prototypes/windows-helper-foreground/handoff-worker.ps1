param(
    [Parameter(Mandatory=$true)][string]$EventName,
    [Parameter(Mandatory=$true)][int]$HelperPid,
    [Parameter(Mandatory=$true)][string]$HelperStarted,
    [Parameter(Mandatory=$true)][int]$SessionId,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
    [switch]$CancellationProbe
)
# THROWAWAY elevated worker: current-session console handoff only, exits once.
$ErrorActionPreference = 'Stop'
$result = @{status='starting'; utc=[DateTime]::UtcNow.ToString('o')}
$gate = $null
try {
    if ($EventName -notmatch '^Local\\PortholeHelperPrototype-[a-f0-9]{32}$') { throw 'Invalid event name' }
    $ownSession = (Get-Process -Id $PID).SessionId
    if ($SessionId -eq 0 -or $SessionId -ne $ownSession) { throw 'Worker must use its own interactive session' }
    $helper = Get-Process -Id $HelperPid
    if ($helper.SessionId -ne $ownSession -or $helper.StartTime.ToUniversalTime().ToString('o') -ne $HelperStarted) { throw 'Helper identity changed' }
    if ($CancellationProbe) { throw 'UAC was approved during cancellation test; handoff is disabled in this mode' }
    $gate = [Threading.EventWaitHandle]::OpenExisting($EventName)
    $result.status = 'armed'
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'worker.json')
    if (-not $gate.WaitOne(30000)) { throw 'Helper did not grant handoff within 30 seconds' }
    $helper = Get-Process -Id $HelperPid
    if ($helper.SessionId -ne $ownSession -or $helper.StartTime.ToUniversalTime().ToString('o') -ne $HelperStarted) { throw 'Helper identity changed before handoff' }
    $result.handoff_utc = [DateTime]::UtcNow.ToString('o')
    $tscon = Join-Path $env:SystemRoot 'System32\tscon.exe'
    $result.output = @(& $tscon $ownSession /dest:console 2>&1 | ForEach-Object { "$_" })
    $result.exit_code = $LASTEXITCODE
    $result.status = if ($LASTEXITCODE -eq 0) { 'handed_off' } else { 'failed' }
} catch { $result.status='failed'; $result.error=$_.Exception.Message }
finally {
    if ($gate) { $gate.Dispose() }
    $result.completed_utc = [DateTime]::UtcNow.ToString('o')
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'worker.json')
}
