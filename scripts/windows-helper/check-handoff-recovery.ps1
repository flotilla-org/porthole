param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][int]$InstalledHelperPid,
    [Parameter(Mandatory=$true)][int[]]$WorkloadPids,
    [Parameter(Mandatory=$true)][string]$EvidencePath
)

# Exercise recovery in a test-owned source build. No worker is launched and no
# handoff action is invoked. Restore the installed helper and its journal even
# if UI inspection fails.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class RecoveryTrayMouse {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X; public int Y; }
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
'@

$installedPath = 'C:\Program Files\PortholeHelper\PortholeHelper.exe'
$installed = Get-Process -Id $InstalledHelperPid -ErrorAction Stop
if ($installed.Path -ne $installedPath) { throw 'Expected the installed development helper' }
if ((Get-Process PortholeConsoleWorker -ErrorAction SilentlyContinue)) { throw 'A handoff worker is running' }
$workloads = @($WorkloadPids | ForEach-Object {
    $process = Get-Process -Id $_ -ErrorAction Stop
    [pscustomobject]@{ pid=$process.Id; started_utc=$process.StartTime.ToUniversalTime().ToString('o') }
})
$journal = Join-Path $env:LOCALAPPDATA 'Porthole\helper\handoff.json'
$original = if (Test-Path -LiteralPath $journal) { [IO.File]::ReadAllBytes($journal) } else { $null }
$archivesBefore = @(Get-ChildItem -LiteralPath (Split-Path $journal) -Filter 'handoff-before-reconciliation-*.json' -ErrorAction SilentlyContinue |
    ForEach-Object FullName)
$testArchive = $null
$testHelper = $null
$duplicateHelper = $null
$dummyWorker = $null
$dummyDir = Join-Path $env:TEMP ('porthole-recovery-' + [Guid]::NewGuid().ToString('N'))
$dummyPath = Join-Path $dummyDir 'PortholeConsoleWorker.exe'
$previous = [RecoveryTrayMouse+Point]::new()
if (-not [RecoveryTrayMouse]::GetCursorPos([ref]$previous)) { throw 'Cannot read pointer position' }

function Find-Element([string]$Name, $Type) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $condition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    return @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition) |
        Where-Object { $_.Current.ControlType -eq $Type } | Select-Object -First 1)[0]
}

try {
    & (Join-Path $PSScriptRoot 'check-tray-quit.ps1') -HelperPid $InstalledHelperPid -WorkloadPids $WorkloadPids -EvidencePath (Join-Path $env:TEMP 'porthole-recovery-pretest-quit.json') | Out-Null
    $synthetic = [ordered]@{
        status='handoff_outcome_unknown'; unresolved=$true
        error='Synthetic test state; no transfer occurred'; utc=[DateTime]::UtcNow.ToString('o')
        worker_pid=$null; worker_started=$null
    }
    $synthetic | ConvertTo-Json | Set-Content -LiteralPath $journal -Encoding UTF8
    $testHelper = Start-Process -FilePath $Executable -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 700
    if ($testHelper.HasExited) { throw 'Test helper exited during launch' }
    $duplicateHelper = Start-Process -FilePath $Executable -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 300
    if (-not $duplicateHelper.HasExited) { throw 'Second helper instance stayed running' }

    $icon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
    if (-not $icon) {
        $overflow = Find-Element 'Show Hidden Icons' ([System.Windows.Automation.ControlType]::Button)
        if (-not $overflow) { throw 'Hidden-icons button missing' }
        $overflow.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 250
        $icon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
    }
    if (-not $icon) { throw 'Test helper tray icon missing' }
    $bounds = $icon.Current.BoundingRectangle
    [RecoveryTrayMouse]::SetCursorPos([int]($bounds.X + $bounds.Width / 2), [int]($bounds.Y + $bounds.Height / 2)) | Out-Null
    Start-Sleep -Milliseconds 80
    [RecoveryTrayMouse]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    [RecoveryTrayMouse]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    $flyout = $null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 100
        $candidate = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Window)
        if ($candidate -and $candidate.Current.ProcessId -eq $testHelper.Id -and -not $candidate.Current.IsOffscreen) {
            $flyout = $candidate
            break
        }
    }
    if (-not $flyout) { throw 'Test helper flyout did not open' }
    $handoff = @($flyout.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition) |
        Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and
            $_.Current.Name -like 'Disconnect RDP*' } | Select-Object -First 1)[0]
    if (-not $handoff -or $handoff.Current.IsEnabled) { throw 'Uncertain outcome did not disable handoff' }
    $inspect = Find-Element 'Inspect previous handoff' ([System.Windows.Automation.ControlType]::Button)
    if (-not $inspect -or -not $inspect.Current.IsEnabled) { throw 'Inspection action missing' }
    New-Item -ItemType Directory -Path $dummyDir | Out-Null
    Copy-Item -LiteralPath (Join-Path $env:SystemRoot 'System32\ping.exe') -Destination $dummyPath
    $dummyWorker = Start-Process -FilePath $dummyPath -ArgumentList '-n 30 127.0.0.1' -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 150
    if ($dummyWorker.HasExited -or -not (Get-Process PortholeConsoleWorker -ErrorAction SilentlyContinue)) {
        throw 'Test-owned worker stand-in did not stay running'
    }
    $inspect.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 150
    $blockedAck = Find-Element 'Acknowledge inspection and allow another handoff' ([System.Windows.Automation.ControlType]::Button)
    if (($blockedAck -and -not $blockedAck.Current.IsOffscreen) -or $handoff.Current.IsEnabled) {
        throw 'Running worker did not block acknowledgement'
    }
    Stop-Process -Id $dummyWorker.Id
    $dummyWorker.WaitForExit()
    $inspect.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 200
    $acknowledge = Find-Element 'Acknowledge inspection and allow another handoff' ([System.Windows.Automation.ControlType]::Button)
    if (-not $acknowledge -or -not $acknowledge.Current.IsEnabled) { throw 'Acknowledgement not offered after safe inspection' }
    if ($handoff.Current.IsEnabled) { throw 'Inspection alone re-enabled handoff' }
    $acknowledge.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 200
    $record = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    if ($record.status -ne 'handoff_reconciled' -or $record.unresolved -or -not $handoff.Current.IsEnabled) {
        throw 'Explicit acknowledgement did not re-arm handoff'
    }
    $archive = @(Get-ChildItem -LiteralPath (Split-Path $journal) -Filter 'handoff-before-reconciliation-*.json' |
        Where-Object { $_.FullName -notin $archivesBefore })
    if ($archive.Count -ne 1) { throw 'Previous journal was not preserved exactly once' }
    $testArchive = $archive[0].FullName
    $result = [ordered]@{
        status='PASS'; observed_utc=[DateTime]::UtcNow.ToString('o')
        test_helper_pid=$testHelper.Id; duplicate_helper_rejected=$true
        handoff_disabled_before_inspection=$true
        running_worker_blocked_acknowledgement=$true; inspection_did_not_rearm=$true
        explicit_acknowledgement_rearmed=$true
        previous_journal_archived=$true; worker_launched=$false; transfer_invoked=$false
        workloads_unchanged=$workloads
    }
    $result | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $EvidencePath -Encoding UTF8
    $result | ConvertTo-Json -Depth 4 -Compress
} finally {
    [RecoveryTrayMouse]::SetCursorPos($previous.X, $previous.Y) | Out-Null
    if ($dummyWorker -and -not $dummyWorker.HasExited) { Stop-Process -Id $dummyWorker.Id }
    if (Test-Path -LiteralPath $dummyPath) { Remove-Item -LiteralPath $dummyPath }
    if (Test-Path -LiteralPath $dummyDir) { Remove-Item -LiteralPath $dummyDir }
    if ($duplicateHelper -and -not $duplicateHelper.HasExited) { Stop-Process -Id $duplicateHelper.Id }
    if ($testHelper -and -not $testHelper.HasExited) { Stop-Process -Id $testHelper.Id }
    if ($original) { [IO.File]::WriteAllBytes($journal, $original) }
    elseif (Test-Path -LiteralPath $journal) { Remove-Item -LiteralPath $journal }
    if ($testArchive -and (Test-Path -LiteralPath $testArchive)) { Remove-Item -LiteralPath $testArchive }
    if (-not (Get-Process PortholeHelper -ErrorAction SilentlyContinue | Where-Object Path -eq $installedPath)) {
        Start-Process -FilePath $installedPath -WindowStyle Hidden | Out-Null
    }
    foreach ($item in $workloads) {
        $process = Get-Process -Id $item.pid -ErrorAction Stop
        if ($process.StartTime.ToUniversalTime().ToString('o') -ne $item.started_utc) {
            throw "Workload process changed: $($item.pid)"
        }
    }
}
