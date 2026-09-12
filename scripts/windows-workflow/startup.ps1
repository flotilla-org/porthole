param(
    [Parameter(Mandatory)][ValidateSet('Install', 'Start', 'Status', 'Remove')][string]$Action,
    [string]$DaemonPath
)
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$taskName = 'Porthole-' + $identity.User.Value
$description = 'Porthole desktop daemon for ' + $identity.User.Value
$existing = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
if ($existing -and $existing.Description -ne $description) {
    throw "Refusing to modify an unrecognized task: $taskName"
}
switch ($Action) {
    'Install' {
        if (!$DaemonPath) { throw 'Install requires -DaemonPath to a built portholed.exe.' }
        $exe = (Resolve-Path -LiteralPath $DaemonPath).Path
        if ([IO.Path]::GetFileName($exe) -ne 'portholed.exe') { throw 'Expected portholed.exe.' }
        if ($existing.State -eq 'Running') {
            if ($existing.Actions.Count -eq 1 -and $existing.Actions[0].Execute -eq $exe -and $existing.Principal.LogonType -eq 'Interactive' -and $existing.Principal.RunLevel -eq 'Limited') {
                Write-Output "Already registered and running: $taskName"
                return
            }
            throw 'Task is running with different settings. Remove it before updating its registration.'
        }
        $principal = New-ScheduledTaskPrincipal -UserId $identity.Name -LogonType Interactive -RunLevel Limited
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User $identity.Name
        $taskAction = New-ScheduledTaskAction -Execute $exe -WorkingDirectory (Split-Path -Parent $exe)
        $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        Register-ScheduledTask -TaskName $taskName -Description $description -Action $taskAction -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
        Write-Output "Registered $taskName for the existing user's GUI login. Use Start to run it now."
    }
    'Start' {
        if (!$existing) { throw 'Install the task first.' }
        Start-ScheduledTask -TaskName $taskName
    }
    'Remove' {
        if ($existing) {
            Stop-ScheduledTask -TaskName $taskName
            Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
        }
    }
}
if ($Action -ne 'Remove') {
    Get-ScheduledTask -TaskName $taskName | Select-Object TaskName, State, @{Name='User';Expression={$_.Principal.UserId}}, @{Name='LogonType';Expression={$_.Principal.LogonType}}, @{Name='RunLevel';Expression={$_.Principal.RunLevel}}
}
