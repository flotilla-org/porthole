param([string]$Executable = (Join-Path $PSScriptRoot 'publish\Helper\PortholeHelper.exe'))
$ErrorActionPreference = 'Stop'
$child = Start-Process -FilePath $Executable -PassThru
try {
    if ($child.WaitForExit(5000)) { throw "Helper exited during launch: $($child.ExitCode)" }
    $status = Get-Content -Raw (Join-Path $env:LOCALAPPDATA 'Porthole\helper\status.json') | ConvertFrom-Json
    if ($status.helper_pid -ne $child.Id -or $status.status -ne 'ready' -or $status.helper_elevated) { throw 'Fresh unelevated ready status missing' }
    $child.Refresh()
    if ($child.MainWindowHandle -ne 0) { throw 'Flyout should start hidden' }
    Write-Output "PASS: unelevated helper starts in the notification area, PID $($child.Id)"
} finally {
    if (-not $child.HasExited) {
        Stop-Process -Id $child.Id
        if (-not $child.WaitForExit(3000)) { throw 'Test-owned helper did not exit' }
    }
}
