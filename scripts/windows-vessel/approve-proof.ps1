param([Parameter(Mandatory=$true)][string]$RunDirectory, [int]$Seconds=45)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'pipe-client.ps1')
$state = Get-Content (Join-Path $RunDirectory 'state.json') -Raw | ConvertFrom-Json
$config = Get-Content (Join-Path $RunDirectory 'config.json') -Raw | ConvertFrom-Json
$deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
do {
    if (Test-Path (Join-Path $RunDirectory 'agent-desktop-result.json')) { Get-Content (Join-Path $RunDirectory 'agent-desktop-result.json'); exit 0 }
    $pending = Invoke-PortholeJson GET '/agent-permissions/requests'
    if ($pending.Status -ne 200) { throw 'Cannot read approval requests' }
    foreach ($item in @($pending.Body | Where-Object { $_.agent_id -eq $state.agent_id -and $_.status -eq 'pending' })) {
        $isFixtureLaunch = $item.description.operation.kind -eq 'launch' -and $item.description.operation.application -eq $config.fixture
        $isFixtureSurface = $item.description.surface.app_name -eq 'desktop_fixture.exe' -and $item.description.surface.title -eq 'Porthole #117 - test-owned editor'
        if (-not $isFixtureLaunch -and -not $isFixtureSurface) { throw "Unexpected request $($item.request_id); operator review required" }
        if (@($item.actions | Where-Object { $_ -notin @('manage','drive','observe') }).Count) { throw 'Unexpected requested capability' }
        $approved = Invoke-PortholeJson POST "/agent-permissions/requests/$($item.request_id)/approve" @{duration=@{type='until_surface_gone'}; target=$item.target; actions=@($item.actions)}
        if ($approved.Status -ne 200) { throw "Approval failed for $($item.request_id)" }
        Write-Output "Approved $($item.request_id): $($item.actions -join ',') for this run's fixture"
    }
    Start-Sleep -Milliseconds 500
} while ([DateTime]::UtcNow -lt $deadline)
Write-Output 'Approval window ended; agent result not yet present.'
