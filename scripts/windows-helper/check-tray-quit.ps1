param(
    [Parameter(Mandatory=$true)][int]$HelperPid,
    [Parameter(Mandatory=$true)][int[]]$WorkloadPids,
    [Parameter(Mandatory=$true)][string]$EvidencePath
)

# Native shell check: use the real notification-area context menu to quit only
# the installed helper, then verify separately owned workload processes live on.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TrayMenuMouse {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X; public int Y; }
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
'@
$helper = Get-Process -Id $HelperPid -ErrorAction Stop
if ($helper.Path -ne 'C:\Program Files\PortholeHelper\PortholeHelper.exe') {
    throw 'Expected the installed development helper'
}
$workloads = @($WorkloadPids | ForEach-Object {
    $process = Get-Process -Id $_ -ErrorAction Stop
    [pscustomobject]@{pid=$process.Id;name=$process.ProcessName;started_utc=$process.StartTime.ToUniversalTime().ToString('o')}
})
$root = [System.Windows.Automation.AutomationElement]::RootElement
function Find-Element([string]$Name, $Type) {
    $condition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    return @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition) |
        Where-Object { $_.Current.ControlType -eq $Type } | Select-Object -First 1)[0]
}
$icon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
if (-not $icon) {
    $overflow = Find-Element 'Show Hidden Icons' ([System.Windows.Automation.ControlType]::Button)
    if (-not $overflow) { throw 'Hidden-icons button missing' }
    $overflow.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 300
    $icon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
}
if (-not $icon) { throw 'Porthole notification-area icon missing' }
$bounds = $icon.Current.BoundingRectangle
$previous = [TrayMenuMouse+Point]::new()
if (-not [TrayMenuMouse]::GetCursorPos([ref]$previous)) { throw 'Cannot read cursor position' }
try {
    if (-not [TrayMenuMouse]::SetCursorPos([int]($bounds.X + $bounds.Width / 2), [int]($bounds.Y + $bounds.Height / 2))) {
        throw 'Cannot point at Porthole icon'
    }
    Start-Sleep -Milliseconds 80
    [TrayMenuMouse]::mouse_event(8, 0, 0, 0, [UIntPtr]::Zero)
    [TrayMenuMouse]::mouse_event(16, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 300
} finally { [TrayMenuMouse]::SetCursorPos($previous.X, $previous.Y) | Out-Null }
$open = Find-Element 'Show Porthole helper' ([System.Windows.Automation.ControlType]::MenuItem)
$quit = Find-Element 'Quit helper' ([System.Windows.Automation.ControlType]::MenuItem)
if (-not $open -or -not $quit -or $open.Current.ProcessId -ne $HelperPid -or $quit.Current.ProcessId -ne $HelperPid) {
    throw 'Expected Porthole context menu missing'
}
$quit.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep -Milliseconds 500
if (Get-Process -Id $HelperPid -ErrorAction SilentlyContinue) { throw 'Quit did not end the helper' }
foreach ($item in $workloads) {
    $process = Get-Process -Id $item.pid -ErrorAction Stop
    if ($process.StartTime.ToUniversalTime().ToString('o') -ne $item.started_utc) {
        throw "Workload process changed: $($item.pid)"
    }
}
$result = [ordered]@{
    status = 'PASS'
    observed_utc = [DateTime]::UtcNow.ToString('o')
    helper_pid = $HelperPid
    icon_name = 'Porthole helper'
    context_menu = @('Show Porthole helper', 'Quit helper')
    helper_exited = $true
    workloads_unchanged = $workloads
}
$result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $EvidencePath -Encoding UTF8
$result | ConvertTo-Json -Depth 5 -Compress
