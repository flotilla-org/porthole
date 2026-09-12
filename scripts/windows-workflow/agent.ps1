param([Parameter(Mandatory)][string]$Directory)
$ErrorActionPreference = 'Stop'
$state = Get-Content -LiteralPath (Join-Path $Directory 'state.json') -Raw | ConvertFrom-Json
$identity = Get-Content -LiteralPath (Join-Path $Directory 'identity.json') -Raw | ConvertFrom-Json
$env:PORTHOLE_AGENT_TOKEN = $identity.token
Set-Location -LiteralPath $Directory
& $state.AgentCommand --no-alt-screen -s workspace-write -a on-request
exit $LASTEXITCODE
