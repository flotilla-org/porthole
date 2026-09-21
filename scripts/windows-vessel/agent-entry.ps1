param([Parameter(Mandatory=$true)][string]$RunDirectory)
$ErrorActionPreference = 'Stop'
$config = Get-Content -LiteralPath (Join-Path $RunDirectory 'config.json') -Raw | ConvertFrom-Json
if (-not $env:PORTHOLE_AGENT_TOKEN) { throw 'Porthole token did not reach the Cleat child' }
$env:PARITY_RUN_DIR = $RunDirectory
$env:PATH = (Split-Path $config.porthole) + ';' + (Split-Path $config.cleat) + ';' + $env:PATH
$process = Get-Process -Id $PID
@{pid=$PID; session_id=$process.SessionId; started=$process.StartTime.ToUniversalTime().ToString('o'); agent_id=$env:PORTHOLE_AGENT_ID; token_present=$true; continuity=[guid]::NewGuid().ToString()} |
    ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $RunDirectory 'agent-entry.json')
$prompt = 'You are the coding agent in a Windows Portholed Vessel acceptance run. Read request.md in this workspace and perform only that task. Never print environment variables or token values. Remain available for further work after reporting the result.'
try {
    & $config.codex --no-alt-screen -C $config.workspace -s danger-full-access -a never -c shell_environment_policy.inherit=all -c shell_environment_policy.ignore_default_excludes=true $prompt
    $agentExit = $LASTEXITCODE
} finally {
    # Local-trust operator command: revokes only this run's identity at run end.
    & $config.porthole agents revoke $env:PORTHOLE_AGENT_ID --json | Out-Null
}
exit $agentExit
