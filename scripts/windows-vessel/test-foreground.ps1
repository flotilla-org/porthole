param(
    [Parameter(Mandatory=$true)][string]$ServerExecutable,
    [Parameter(Mandatory=$true)][string]$FixtureExecutable,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
    [ValidateRange(2,100)][int]$Iterations = 6
)
$ErrorActionPreference = 'Stop'
$ServerExecutable = (Resolve-Path -LiteralPath $ServerExecutable).Path
$FixtureExecutable = (Resolve-Path -LiteralPath $FixtureExecutable).Path
$EvidenceDirectory = [IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath $EvidenceDirectory) { throw 'Use a fresh evidence directory' }
New-Item -ItemType Directory -Path $EvidenceDirectory | Out-Null
$suffix = [guid]::NewGuid().ToString('N')
$server = $null
try {
    $server = Start-Process -FilePath $ServerExecutable -ArgumentList $suffix -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $EvidenceDirectory 'stdout.log') -RedirectStandardError (Join-Path $EvidenceDirectory 'stderr.log')
    $started = $server.StartTime.ToUniversalTime().ToString('o')
    @{portholed_pid=$server.Id; portholed_started=$started} | ConvertTo-Json | Set-Content (Join-Path $EvidenceDirectory 'state.json')
    @{fixture=$FixtureExecutable} | ConvertTo-Json | Set-Content (Join-Path $EvidenceDirectory 'config.json')
    . (Join-Path $PSScriptRoot 'pipe-client.ps1')
    $ready = $false
    foreach ($attempt in 1..20) {
        try { $info = Invoke-PortholeJson GET '/info' -PipeName "porthole-foreground-$suffix"; if ($info.Status -eq 200) { $ready=$true; break } } catch { Start-Sleep -Milliseconds 100 }
    }
    if (-not $ready) { throw 'Test server did not become ready' }
    & powershell.exe -NoProfile -File (Join-Path $PSScriptRoot 'foreground-proof.ps1') -RunDirectory $EvidenceDirectory -PipeName "porthole-foreground-$suffix" -EvidencePath (Join-Path $EvidenceDirectory 'result.json') -InterveningInput -SeparateInputProcess -Iterations $Iterations
    $proofExit = $LASTEXITCODE
} finally {
    if ($server) {
        $live = Get-Process -Id $server.Id -ErrorAction SilentlyContinue
        if ($live -and $live.Path -eq $ServerExecutable -and $live.StartTime.ToUniversalTime().ToString('o') -eq $started) {
            Stop-Process -Id $live.Id
            $live.WaitForExit()
        }
    }
}
exit $proofExit
