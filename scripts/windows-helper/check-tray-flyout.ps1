param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$EvidencePath,
    [string]$ScreenshotPath
)

# Exercise Explorer's actual notification-area button and the helper's WinUI
# flyout. This deliberately does not start a console handoff.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class FlyoutKeyboard {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X; public int Y; }
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
}
'@
$child = Start-Process -FilePath $Executable -PassThru -WindowStyle Hidden
try {
    Start-Sleep -Milliseconds 600
    if ($child.HasExited) { throw 'Helper exited during launch' }
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
        Start-Sleep -Milliseconds 250
        $icon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
    }
    if (-not $icon) { throw 'Porthole notification-area icon missing' }
    $iconBounds = $icon.Current.BoundingRectangle
    $previousForeground = [FlyoutKeyboard]::GetForegroundWindow()
    $previous = [FlyoutKeyboard+Point]::new()
    if (-not [FlyoutKeyboard]::GetCursorPos([ref]$previous)) { throw 'Cannot read cursor position' }
    if (-not [FlyoutKeyboard]::SetCursorPos([int]($iconBounds.X + $iconBounds.Width / 2), [int]($iconBounds.Y + $iconBounds.Height / 2))) {
        throw 'Cannot point at Porthole icon'
    }
    Start-Sleep -Milliseconds 80
    [FlyoutKeyboard]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    [FlyoutKeyboard]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    $flyout = $null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 100
        $candidate = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Window)
        if ($candidate -and $candidate.Current.ProcessId -eq $child.Id -and -not $candidate.Current.IsOffscreen) {
            $flyout = $candidate
            break
        }
    }
    if (-not $flyout) { throw 'Tray click did not reveal the helper flyout' }
    $bounds = $flyout.Current.BoundingRectangle
    if ([FlyoutKeyboard]::GetForegroundWindow().ToInt64() -ne $flyout.Current.NativeWindowHandle) {
        throw "Tray click opened the flyout without activating it: foreground=$([FlyoutKeyboard]::GetForegroundWindow().ToInt64()) flyout=$($flyout.Current.NativeWindowHandle)"
    }
    if ($bounds.Width -gt 500 -or $bounds.Height -gt 450 -or $bounds.Width -lt 300 -or $bounds.Height -lt 250) {
        throw "Unexpected flyout size: $($bounds.Width)x$($bounds.Height)"
    }
    if ([Math]::Abs(($bounds.X + $bounds.Width / 2) - ($iconBounds.X + $iconBounds.Width / 2)) -gt 300) {
        throw 'Flyout is not near its tray icon'
    }
    if ($bounds.X -lt $iconBounds.X + $iconBounds.Width -and
        $bounds.X + $bounds.Width -gt $iconBounds.X -and
        $bounds.Y -lt $iconBounds.Y + $iconBounds.Height -and
        $bounds.Y + $bounds.Height -gt $iconBounds.Y) {
        throw "Flyout overlaps its tray icon: flyout=$bounds icon=$iconBounds"
    }
    $action = @($flyout.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition) |
        Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and
            $_.Current.Name -like 'Disconnect RDP*' } | Select-Object -First 1)[0]
    if (-not $action) { throw 'Handoff action missing from flyout' }
    if ($ScreenshotPath) {
        Add-Type -AssemblyName System.Drawing
        $bitmap = [System.Drawing.Bitmap]::new([int]$bounds.Width, [int]$bounds.Height)
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.CopyFromScreen([int]$bounds.X, [int]$bounds.Y, 0, 0, $bitmap.Size)
            } finally { $graphics.Dispose() }
            $bitmap.Save($ScreenshotPath, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $bitmap.Dispose() }
    }
    $action.SetFocus()
    $hwnd = [IntPtr]$flyout.Current.NativeWindowHandle
    $foregroundBeforeEscape = [FlyoutKeyboard]::GetForegroundWindow().ToInt64()
    [FlyoutKeyboard]::keybd_event(27, 0, 0, [UIntPtr]::Zero)
    [FlyoutKeyboard]::keybd_event(27, 0, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    if ([FlyoutKeyboard]::IsWindowVisible($hwnd)) { throw "Escape did not dismiss the flyout; foreground=$foregroundBeforeEscape flyout=$($hwnd.ToInt64())" }
    if ($previousForeground -ne [IntPtr]::Zero) {
        $secondIcon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
        if (-not $secondIcon) {
            $overflow = Find-Element 'Show Hidden Icons' ([System.Windows.Automation.ControlType]::Button)
            if (-not $overflow) { throw 'Hidden-icons button missing on second opening' }
            $overflow.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
            Start-Sleep -Milliseconds 250
            $secondIcon = Find-Element 'Porthole helper' ([System.Windows.Automation.ControlType]::Button)
        }
        if (-not $secondIcon) { throw 'Porthole icon missing on second opening' }
        $secondBounds = $secondIcon.Current.BoundingRectangle
        [FlyoutKeyboard]::SetCursorPos([int]($secondBounds.X + $secondBounds.Width / 2), [int]($secondBounds.Y + $secondBounds.Height / 2)) | Out-Null
        Start-Sleep -Milliseconds 80
        [FlyoutKeyboard]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
        [FlyoutKeyboard]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
        for ($attempt = 0; $attempt -lt 10 -and -not [FlyoutKeyboard]::IsWindowVisible($hwnd); $attempt++) {
            Start-Sleep -Milliseconds 100
        }
        if (-not [FlyoutKeyboard]::IsWindowVisible($hwnd)) { throw 'Second tray click did not reopen the flyout' }
        if (-not [FlyoutKeyboard]::SetForegroundWindow($previousForeground)) { throw 'Cannot focus the previous window' }
        Start-Sleep -Milliseconds 250
        if ([FlyoutKeyboard]::IsWindowVisible($hwnd)) { throw 'Flyout stayed open after focus moved elsewhere' }
    }
    $result = [ordered]@{
        status = 'PASS'
        observed_utc = [DateTime]::UtcNow.ToString('o')
        helper_pid = $child.Id
        icon = @{x=$iconBounds.X;y=$iconBounds.Y;width=$iconBounds.Width;height=$iconBounds.Height}
        flyout = @{x=$bounds.X;y=$bounds.Y;width=$bounds.Width;height=$bounds.Height}
        handoff_action = $action.Current.Name
        escape_dismissed = $true
        focus_loss_dismissed = $previousForeground -ne [IntPtr]::Zero
    }
    $result | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $EvidencePath -Encoding UTF8
    $result | ConvertTo-Json -Depth 4 -Compress
} finally {
    if ($previous) { [FlyoutKeyboard]::SetCursorPos($previous.X, $previous.Y) | Out-Null }
    if (-not $child.HasExited) { Stop-Process -Id $child.Id }
}
