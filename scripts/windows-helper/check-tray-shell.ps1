param(
    [Parameter(Mandatory=$true)][int]$HelperPid,
    [Parameter(Mandatory=$true)][int[]]$WorkloadPids,
    [Parameter(Mandatory=$true)][string]$EvidencePath
)

# Native acceptance of keyboard access and taskbar recovery for a running helper.
# Restart only the Explorer process that owns this session's taskbar.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TrayShellKeys {
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
}
'@

function Find-Element([string]$Name, $Type) {
    $condition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    return @([System.Windows.Automation.AutomationElement]::RootElement.FindAll(
        [System.Windows.Automation.TreeScope]::Descendants, $condition) |
        Where-Object { $_.Current.ControlType -eq $Type } | Select-Object -First 1)[0]
}

function Find-Icon {
    $icon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
    if (-not $icon) {
        $overflow = Find-Element 'Show Hidden Icons' ([System.Windows.Automation.ControlType]::Button)
        if (-not $overflow) { throw 'Hidden-icons button missing' }
        $overflow.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 300
        $icon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
    }
    if (-not $icon) { throw 'Porthole notification-area icon missing' }
    return $icon
}

function Assert-Process([int]$ProcessId, [string]$StartedUtc) {
    $process = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($process.StartTime.ToUniversalTime().ToString('o') -ne $StartedUtc) {
        throw "Process $ProcessId restarted"
    }
}

$helper = Get-Process -Id $HelperPid -ErrorAction Stop
if ($helper.Path -ne 'C:\Program Files\PortholeHelper\PortholeHelper.exe') {
    throw 'Expected the installed development helper'
}
$session = $helper.SessionId
$helperStart = $helper.StartTime.ToUniversalTime().ToString('o')
$workloads = @($WorkloadPids | ForEach-Object {
    $process = Get-Process -Id $_ -ErrorAction Stop
    if ($process.SessionId -ne $session) { throw "Workload $_ is in another session" }
    [pscustomobject]@{ pid=$process.Id; name=$process.ProcessName; started_utc=$process.StartTime.ToUniversalTime().ToString('o') }
})
$icon = Find-Icon
$icon.SetFocus()
[TrayShellKeys]::keybd_event(13, 0, 0, [UIntPtr]::Zero)
[TrayShellKeys]::keybd_event(13, 0, 2, [UIntPtr]::Zero)
$flyout = $null
for ($attempt = 0; $attempt -lt 30; $attempt++) {
    Start-Sleep -Milliseconds 100
    $candidate = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Window)
    if ($candidate -and $candidate.Current.ProcessId -eq $HelperPid -and -not $candidate.Current.IsOffscreen) {
        $flyout = $candidate
        break
    }
}
if (-not $flyout) { throw 'Enter on focused tray icon did not open helper flyout' }
$action = @($flyout.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    [System.Windows.Automation.Condition]::TrueCondition) |
    Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and
        $_.Current.Name -like 'Disconnect RDP*' } | Select-Object -First 1)[0]
if (-not $action -or -not $action.Current.IsKeyboardFocusable) {
    throw 'Handoff action is missing or not keyboard focusable'
}
$action.SetFocus()
[TrayShellKeys]::keybd_event(27, 0, 0, [UIntPtr]::Zero)
[TrayShellKeys]::keybd_event(27, 0, 2, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 250
if ([TrayShellKeys]::IsWindowVisible([IntPtr]$flyout.Current.NativeWindowHandle)) {
    throw 'Escape did not dismiss keyboard-opened flyout'
}

$root = [System.Windows.Automation.AutomationElement]::RootElement
$taskbar = @($root.FindAll([System.Windows.Automation.TreeScope]::Children,
    [System.Windows.Automation.Condition]::TrueCondition) |
    Where-Object { $_.Current.ClassName -eq 'Shell_TrayWnd' -and $_.Current.ProcessId -gt 0 } |
    Select-Object -First 1)[0]
if (-not $taskbar) { throw 'Taskbar owner not found' }
$shell = Get-Process -Id $taskbar.Current.ProcessId -ErrorAction Stop
if ($shell.Path -ine (Join-Path $env:SystemRoot 'explorer.exe') -or $shell.SessionId -ne $session) {
    throw 'Taskbar owner is not this session Windows Explorer'
}
$shellPid = $shell.Id
$shellWindows = @($root.FindAll([System.Windows.Automation.TreeScope]::Children,
    [System.Windows.Automation.Condition]::TrueCondition) |
    Where-Object { $_.Current.ProcessId -eq $shellPid })
$unexpected = @($shellWindows | Where-Object { $_.Current.ClassName -notin @(
    'Shell_TrayWnd','Progman','WorkerW','TopLevelWindowForOverflowXamlIsland') })
if ($unexpected.Count) { throw 'Taskbar Explorer owns another top-level window; refusing restart' }

try {
    Stop-Process -Id $shellPid -ErrorAction Stop
    $newTaskbar = $null
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        Start-Sleep -Milliseconds 200
        $newTaskbar = @($root.FindAll([System.Windows.Automation.TreeScope]::Children,
            [System.Windows.Automation.Condition]::TrueCondition) |
            Where-Object { $_.Current.ClassName -eq 'Shell_TrayWnd' -and $_.Current.ProcessId -ne $shellPid } |
            Select-Object -First 1)[0]
        if ($newTaskbar) { break }
    }
    if (-not $newTaskbar) {
        Start-Process -FilePath (Join-Path $env:SystemRoot 'explorer.exe')
        for ($attempt = 0; $attempt -lt 50; $attempt++) {
            Start-Sleep -Milliseconds 200
            $newTaskbar = @($root.FindAll([System.Windows.Automation.TreeScope]::Children,
                [System.Windows.Automation.Condition]::TrueCondition) |
                Where-Object { $_.Current.ClassName -eq 'Shell_TrayWnd' -and $_.Current.ProcessId -ne $shellPid } |
                Select-Object -First 1)[0]
            if ($newTaskbar) { break }
        }
    }
    if (-not $newTaskbar) { throw 'Explorer taskbar did not recover' }
    Assert-Process $HelperPid $helperStart
    foreach ($item in $workloads) { Assert-Process $item.pid $item.started_utc }
    $icon = $null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        try { $icon = Find-Icon; break } catch { Start-Sleep -Milliseconds 200 }
    }
    if (-not $icon) { throw 'Helper icon did not return after Explorer restart' }
    $icon.SetFocus()
    [TrayShellKeys]::keybd_event(13, 0, 0, [UIntPtr]::Zero)
    [TrayShellKeys]::keybd_event(13, 0, 2, [UIntPtr]::Zero)
    $reopened = $null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 100
        $candidate = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Window)
        if ($candidate -and $candidate.Current.ProcessId -eq $HelperPid -and -not $candidate.Current.IsOffscreen) {
            $reopened = $candidate
            break
        }
    }
    if (-not $reopened) { throw 'Restored icon did not open the original helper' }
    [TrayShellKeys]::keybd_event(27, 0, 0, [UIntPtr]::Zero)
    [TrayShellKeys]::keybd_event(27, 0, 2, [UIntPtr]::Zero)
    $result = [ordered]@{
        status='PASS'; observed_utc=[DateTime]::UtcNow.ToString('o')
        helper_pid=$HelperPid; helper_started_utc=$helperStart
        explorer_before_pid=$shellPid; explorer_after_pid=$newTaskbar.Current.ProcessId
        icon_name=$icon.Current.Name; keyboard_enter_opened=$true
        handoff_keyboard_focusable=$true; escape_dismissed=$true
        restored_icon_opened_original_helper=$true; workloads_unchanged=$workloads
    }
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $EvidencePath -Encoding UTF8
    $result | ConvertTo-Json -Depth 5 -Compress
} finally {
    $liveTaskbar = @($root.FindAll([System.Windows.Automation.TreeScope]::Children,
        [System.Windows.Automation.Condition]::TrueCondition) |
        Where-Object { $_.Current.ClassName -eq 'Shell_TrayWnd' })
    if (-not $liveTaskbar.Count) { Start-Process -FilePath (Join-Path $env:SystemRoot 'explorer.exe') }
}
