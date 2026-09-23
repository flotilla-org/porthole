param(
    [Parameter(Mandatory=$true)][string]$PortholeExecutable,
    [string]$StateDirectory = (Join-Path $env:LOCALAPPDATA 'Porthole\startup'),
    [string]$TaskName = ('Porthole GUI - ' + [Security.Principal.WindowsIdentity]::GetCurrent().User.Value),
    [switch]$Remove
)
$ErrorActionPreference = 'Stop'
if ($TaskName -match '[\\/\*\?\[\]]') { throw 'TaskName must be a literal name in the root task folder' }
$PortholeExecutable = [IO.Path]::GetFullPath($PortholeExecutable)
if (-not $Remove) { $PortholeExecutable = (Get-Item -LiteralPath $PortholeExecutable).FullName }
if ([IO.Path]::GetFileName($PortholeExecutable) -ne 'portholed.exe') { throw 'Expected portholed.exe' }
$StateDirectory = [IO.Path]::GetFullPath($StateDirectory)
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$description = "Porthole Windows vessel startup v1; owner=$sid"
$shell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$entry = Join-Path $PSScriptRoot 'startup-entry.ps1'
# Encode literal paths through the Windows command-line boundary. No tokens or
# agent credentials belong in this task's arguments or configuration.
$invocation = "& '" + $entry.Replace("'", "''") + "' -PortholeExecutable '" + $PortholeExecutable.Replace("'", "''") + "' -StateDirectory '" + $StateDirectory.Replace("'", "''") + "'"
$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($invocation))
$arguments = "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -EncodedCommand $encoded"
$existing = @(Get-ScheduledTask -TaskPath '\' | Where-Object TaskName -eq $TaskName)
if ($existing.Count -gt 0) {
    $task = $existing[0]
    $principalSid = $task.Principal.UserId
    if ($principalSid -notlike 'S-1-*') { $principalSid = ([Security.Principal.NTAccount]$principalSid).Translate([Security.Principal.SecurityIdentifier]).Value }
    if ($task.Description -ne $description -or $principalSid -ne $sid -or
        @($task.Actions).Count -ne 1 -or $task.Actions[0].Execute -ne $shell -or
        $task.Actions[0].Arguments -cne $arguments -or $task.Actions[0].WorkingDirectory -ne $PSScriptRoot) {
        throw 'Task name is occupied by unrelated or differently configured work; refusing to change it'
    }
    if ($Remove) {
        # Unregister only. Do not Stop-ScheduledTask or terminate its process tree.
        Unregister-ScheduledTask -TaskName $TaskName -TaskPath '\' -Confirm:$false
        Write-Output "REMOVED registration: $TaskName; running processes were not terminated"
        return
    }
    $triggerSid = $task.Triggers[0].UserId
    if ($triggerSid -and $triggerSid -notlike 'S-1-*') { $triggerSid = ([Security.Principal.NTAccount]$triggerSid).Translate([Security.Principal.SecurityIdentifier]).Value }
    if ($task.Principal.LogonType -ne 'Interactive' -or $task.Principal.RunLevel -ne 'Limited' -or
        @($task.Triggers).Count -ne 1 -or $task.Triggers[0].CimClass.CimClassName -ne 'MSFT_TaskLogonTrigger' -or
        $triggerSid -ne $sid -or
        $task.Settings.MultipleInstances -ne 'IgnoreNew' -or $task.Settings.ExecutionTimeLimit -ne 'PT0S' -or
        $task.Settings.DisallowStartIfOnBatteries -or $task.Settings.StopIfGoingOnBatteries -or $task.Settings.RunOnlyIfIdle -or
        -not $task.Settings.Enabled -or -not $task.Triggers[0].Enabled) {
        throw 'Owned task settings drifted; inspect and explicitly remove the registration before replacing it'
    }
    Write-Output "REUSED registration: $TaskName"
    return
}
if ($Remove) { Write-Output "ABSENT registration: $TaskName"; return }
$action = New-ScheduledTaskAction -Execute $shell -Argument $arguments -WorkingDirectory $PSScriptRoot
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $sid
$principal = New-ScheduledTaskPrincipal -UserId $sid -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -DontStopOnIdleEnd
$definition = New-ScheduledTask -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description $description
Register-ScheduledTask -TaskName $TaskName -TaskPath '\' -InputObject $definition | Out-Null
Write-Output "REGISTERED: $TaskName; agent launch remains explicit"
