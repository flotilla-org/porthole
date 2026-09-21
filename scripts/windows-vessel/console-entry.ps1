param([Parameter(Mandatory=$true)][string]$RunDirectory)
$ErrorActionPreference = 'Stop'
# Porthole gives launched children NUL standard streams. Conhost interprets
# valid streams as ConPTY transport. ShellExecute starts the visible console
# with the normal Windows console handles instead, while this wrapper stays
# alive so Porthole can verify its descendant window and process birth times.
$terminalEntry = Join-Path $PSScriptRoot 'terminal-entry.ps1'
$arguments = '-- powershell.exe -NoProfile -File "' + $terminalEntry + '" -RunDirectory "' + $RunDirectory + '"'
$console = Start-Process -FilePath "$env:SystemRoot\System32\conhost.exe" -ArgumentList $arguments -WindowStyle Normal -PassThru
$console.WaitForExit()
