$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    & "$env:ProgramFiles\dotnet\dotnet.exe" restore ChannelChecks\ChannelChecks.csproj --locked-mode
    if ($LASTEXITCODE) { throw 'Channel check restore failed' }
    & "$env:ProgramFiles\dotnet\dotnet.exe" run --project ChannelChecks\ChannelChecks.csproj --no-restore
    if ($LASTEXITCODE) { throw 'Channel identity checks failed' }
    & "$env:ProgramFiles\dotnet\dotnet.exe" publish Helper\Helper.csproj -c Release -p:RestoreLockedMode=true -o publish\Helper
    if ($LASTEXITCODE) { throw 'WinUI publish failed' }
    & "$env:USERPROFILE\.cargo\bin\cargo.exe" build --manifest-path Worker\Cargo.toml --release --locked
    if ($LASTEXITCODE) { throw 'Rust build failed' }
    $workerDir = Join-Path $PSScriptRoot 'publish\Helper\Worker'
    New-Item -ItemType Directory -Force $workerDir | Out-Null
    Copy-Item -LiteralPath 'Worker\target\release\PortholeConsoleWorker.exe' -Destination $workerDir
    $root = (Resolve-Path 'publish\Helper').Path
    $files = @(Get-ChildItem -LiteralPath $root -File -Recurse | ForEach-Object {
        [ordered]@{ path = $_.FullName.Substring($root.Length + 1); sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    })
    $files | ConvertTo-Json | Set-Content -Encoding UTF8 'publish\files.json'
} finally { Pop-Location }
