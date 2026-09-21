# Isolated process/path and error-preservation checks; no desktop or daemon needed.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'child-command.ps1')
$root = Join-Path ([IO.Path]::GetTempPath()) ('vessel-test-' + [Guid]::NewGuid().ToString('N'))
$run = Join-Path $root "space [brackets] & %USERNAME% 'apostrophe'"
$oldToken = $env:PORTHOLE_AGENT_TOKEN
try {
    [IO.Directory]::CreateDirectory($run) | Out-Null
    $child = Join-Path $run 'child.ps1'
    @'
param([string]$RunDirectory)
[IO.File]::WriteAllText((Join-Path $RunDirectory 'received.txt'), $RunDirectory)
'@ | Set-Content -LiteralPath $child -Encoding UTF8
    # Exercise the actual cmd.exe boundary used by Cleat, including a trailing slash.
    $expected = $run + '\'
    $command = New-VesselChildCommand $child $expected
    $process = Start-Process -FilePath $env:ComSpec -ArgumentList ('/D /C ' + $command) -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0 -or [IO.File]::ReadAllText((Join-Path $run 'received.txt')) -cne $expected) { throw 'Child command changed a literal path' }
    Write-Output 'PASS: child command preserves special characters and trailing slash through cmd.exe.'

    $fake = Join-Path $run 'fake-porthole.ps1'
    @'
if ($args[0] -eq 'launch') {
    $global:LASTEXITCODE = 0
    '{"surface_id":"test-only","confidence":"strong","surface_was_preexisting":false}'
} elseif ($args[0] -eq 'close') {
    $global:LASTEXITCODE = 1
    'cleanup-failure'
} else {
    $global:LASTEXITCODE = 1
    'primary-failure'
}
'@ | Set-Content -LiteralPath $fake -Encoding UTF8
    @{porthole=$fake; fixture='test-only'} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $run 'config.json') -Encoding UTF8
    $env:PORTHOLE_AGENT_TOKEN = 'test-only-not-a-credential'
    $failure = $null
    $warnings = @()
    try { & (Join-Path $PSScriptRoot 'desktop-proof.ps1') -RunDirectory $run -WarningVariable warnings -WarningAction SilentlyContinue }
    catch { $failure = $_.Exception.Message }
    if ($failure -notmatch 'primary-failure' -or $failure -match 'cleanup-failure') { throw "Original failure was lost: $failure" }
    if (($warnings -join ' ') -notmatch 'cleanup-failure') { throw 'Cleanup failure was not reported' }
    if (Test-Path -LiteralPath (Join-Path $run 'agent-desktop-result.json')) { throw 'Failed proof wrote a passing result' }
    Write-Output 'PASS: primary failure survives failed cleanup, which is reported separately.'
} finally {
    $env:PORTHOLE_AGENT_TOKEN = $oldToken
    $resolved = [IO.Path]::GetFullPath($root)
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolved -Leaf) -notlike 'vessel-test-*') { throw 'Unexpected test cleanup path' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
