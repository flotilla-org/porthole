param(
    [Parameter(Mandatory=$true)][int]$HelperPid,
    [Parameter(Mandatory=$true)][string]$EvidencePath
)

# Native shell check: hide the installed WinUI window, then reopen it through
# the real notification-area button exposed by Explorer UI Automation.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$helper = Get-Process -Id $HelperPid -ErrorAction Stop
if ($helper.Path -ne 'C:\Program Files\PortholeHelper\PortholeHelper.exe') {
    throw 'Expected the installed development helper'
}
if ($helper.MainWindowHandle -ne 0) {
    if (-not $helper.CloseMainWindow()) { throw 'Cannot close helper status window' }
    Start-Sleep -Milliseconds 400
}
$helper.Refresh()
if ($helper.HasExited -or $helper.MainWindowHandle -ne 0) { throw 'Helper did not hide in the notification area' }

$root = [System.Windows.Automation.AutomationElement]::RootElement
function Find-Button([string]$Name) {
    $condition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    return @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition) |
        Where-Object { $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button } |
        Select-Object -First 1)[0]
}
function Invoke-Button($Button) {
    if (-not $Button) { throw 'Notification-area button missing' }
    $Button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}

$icon = Find-Button 'Porthole helper'
if (-not $icon) {
    Invoke-Button (Find-Button 'Show Hidden Icons')
    Start-Sleep -Milliseconds 300
    $icon = Find-Button 'Porthole helper'
}
if (-not $icon) { throw 'Porthole helper icon not found in the notification area' }
Invoke-Button $icon
for ($attempt = 0; $attempt -lt 20; $attempt++) {
    Start-Sleep -Milliseconds 100
    $helper.Refresh()
    if ($helper.MainWindowHandle -ne 0) { break }
}
if ($helper.MainWindowHandle -eq 0 -or $helper.MainWindowTitle -ne 'Porthole helper') {
    throw 'Tray icon did not reopen the helper status window'
}
$result = [ordered]@{
    status = 'PASS'
    observed_utc = [DateTime]::UtcNow.ToString('o')
    helper_pid = $helper.Id
    helper_started_utc = $helper.StartTime.ToUniversalTime().ToString('o')
    icon_name = $icon.Current.Name
    window_title = $helper.MainWindowTitle
    window_handle = $helper.MainWindowHandle.ToInt64()
}
$result | ConvertTo-Json | Set-Content -LiteralPath $EvidencePath -Encoding UTF8
if (-not $helper.CloseMainWindow()) { throw 'Reopened helper did not close' }
Start-Sleep -Milliseconds 300
$helper.Refresh()
if ($helper.HasExited -or $helper.MainWindowHandle -ne 0) { throw 'Helper did not return to the notification area' }
$result | ConvertTo-Json -Compress
