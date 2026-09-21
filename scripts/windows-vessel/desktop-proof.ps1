param([Parameter(Mandatory=$true)][string]$RunDirectory)
$ErrorActionPreference = 'Stop'
$config = Get-Content -LiteralPath (Join-Path $RunDirectory 'config.json') -Raw | ConvertFrom-Json
if (-not $env:PORTHOLE_AGENT_TOKEN) { throw 'Agent tool process did not inherit its Porthole token' }
$surface = $null
$keepSurface = $false
$pendingPath = Join-Path $RunDirectory 'agent-desktop-pending.json'
$log = Join-Path $RunDirectory 'agent-desktop-commands.txt'
function Invoke-DesktopCommand([string[]]$Arguments) {
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    do {
        $ErrorActionPreference = 'Continue'
        $output = & $config.porthole @Arguments 2>&1
        $code = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        $text = ($output | ForEach-Object { "$_" }) -join "`n"
        if ($code -eq 0) {
            Add-Content -LiteralPath $log -Value ("porthole " + ($Arguments -join ' ') + ' -> ' + $text)
            return $text
        }
        if ($text -notmatch 'agent_permission_needed' -or [DateTime]::UtcNow -gt $deadline) { throw "Porthole command failed: $text" }
        Start-Sleep -Milliseconds 500
    } while ($true)
}
try {
    if (Test-Path -LiteralPath $pendingPath) {
        $pending = Get-Content -LiteralPath $pendingPath -Raw | ConvertFrom-Json
        if ($pending.agent_id -ne $env:PORTHOLE_AGENT_ID) { throw 'Pending surface belongs to a different agent run' }
        $surface = $pending.surface_id
    } else {
        $launch = Invoke-DesktopCommand @('launch','--app',$config.fixture,'--require-fresh-surface','--json') | ConvertFrom-Json
        $surface = $launch.surface_id
        if ($launch.confidence -ne 'strong' -or $launch.surface_was_preexisting) { throw 'No fresh correlated window' }
    }
    Invoke-DesktopCommand @('focus',$surface) | Out-Null
    Invoke-DesktopCommand @('text',$surface,'Beaufort: a Cleat-hosted Codex agent typed this.') | Out-Null
    Invoke-DesktopCommand @('key',$surface,'--key','Enter') | Out-Null
    Invoke-DesktopCommand @('text',$surface,'Porthole token inherited; native Windows input and PNG.') | Out-Null
    $png = Join-Path $RunDirectory 'agent-visible-input.png'
    Invoke-DesktopCommand @('screenshot',$surface,'--out',$png) | Out-Null
    Invoke-DesktopCommand @('close',$surface) | Out-Null
    $surface = $null
    if (Test-Path -LiteralPath $pendingPath) { Remove-Item -LiteralPath $pendingPath }
    @{result='PASS'; caller_pid=$PID; windows_session=(Get-Process -Id $PID).SessionId; agent_id=$env:PORTHOLE_AGENT_ID; png=$png; sha256=(Get-FileHash $png -Algorithm SHA256).Hash; completed_utc=[DateTime]::UtcNow.ToString('o')} |
        ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $RunDirectory 'agent-desktop-result.json')
    Write-Output 'PASS: real Porthole launch/input/screenshot/close from the coding agent.'
} catch {
    if ($surface -and $_.Exception.Message -match 'system_permission_needed') {
        $keepSurface = $true
        @{agent_id=$env:PORTHOLE_AGENT_ID; surface_id=$surface; reason=$_.Exception.Message} |
            ConvertTo-Json | Set-Content -Encoding UTF8 $pendingPath
        Write-Output 'BLOCKED: activate the test-owned editor in the Windows GUI, then rerun this script to resume the same surface.'
    }
    throw
} finally {
    if ($surface -and -not $keepSurface) { Invoke-DesktopCommand @('close',$surface) | Out-Null }
}
