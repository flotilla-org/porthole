param([Parameter(Mandatory=$true)][string]$RunDirectory)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'child-command.ps1')
$config = Get-Content -LiteralPath (Join-Path $RunDirectory 'config.json') -Raw | ConvertFrom-Json
$env:CLEAT_RUNTIME_DIR = $config.runtime
$env:CLEAT_DAEMON = $config.server
$env:PATH = (Split-Path $config.cleat) + ';' + $env:PATH
if (-not $env:PORTHOLE_AGENT_TOKEN) { throw 'Porthole token did not reach the launched terminal' }
$entry = Join-Path $PSScriptRoot 'agent-entry.ps1'
$childCommand = New-VesselChildCommand $entry $RunDirectory
$launch = & $config.cleat --server $config.server launch $config.session --cmd $childCommand --cwd $config.workspace --size 120x40 --json
if ($LASTEXITCODE -ne 0) { throw 'Cleat agent launch failed' }
$launch | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $RunDirectory 'cleat-launch.json')
& $config.cleat --server $config.server attach $config.session --no-create --identity beaufort-local
exit $LASTEXITCODE
