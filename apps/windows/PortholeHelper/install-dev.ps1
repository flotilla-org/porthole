# Development-only installer. Run this reviewed script elevated; production needs
# a signed installer. The input manifest is a build inventory, not a trust root.
param([switch]$Update)
$ErrorActionPreference = 'Stop'
Start-Transcript -Path (Join-Path $PSScriptRoot 'publish\install.log') -Force | Out-Null
trap { Write-Host ($_ | Out-String); Stop-Transcript | Out-Null; exit 1 }
$admin = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $admin.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Installation requires elevation' }
$destination = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'PortholeHelper'
if (Test-Path -LiteralPath $destination) {
    if (-not $Update) { throw "Installation already exists: $destination" }
    if (Get-Process PortholeHelper -ErrorAction SilentlyContinue) { throw 'Close the prototype before updating' }
    if ((Get-Item -LiteralPath $destination).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Unexpected installation reparse point' }
    $previous = Get-Content -Raw (Join-Path $destination 'files.json') | ConvertFrom-Json
    foreach ($file in $previous) {
        $oldPath = [IO.Path]::GetFullPath((Join-Path $destination $file.path))
        if (-not $oldPath.StartsWith($destination + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid installed manifest path' }
        if ((Get-FileHash -LiteralPath $oldPath -Algorithm SHA256).Hash -ne $file.sha256) { throw "Existing installation changed: $($file.path)" }
    }
}
$source = (Resolve-Path (Join-Path $PSScriptRoot 'publish\Helper')).Path
$files = Get-Content -Raw (Join-Path $PSScriptRoot 'publish\files.json') | ConvertFrom-Json
foreach ($file in $files) {
    $full = [IO.Path]::GetFullPath((Join-Path $source $file.path))
    if (-not $full.StartsWith($source + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid manifest path' }
    if ((Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash -ne $file.sha256) { throw "Build changed: $($file.path)" }
}
New-Item -ItemType Directory -Force -Path $destination | Out-Null
foreach ($file in $files) {
    $target = Join-Path $destination $file.path
    New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($target)) | Out-Null
    Copy-Item -LiteralPath (Join-Path $source $file.path) -Destination $target
    if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $file.sha256) { throw "Installed hash mismatch: $($file.path)" }
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'publish\files.json') -Destination (Join-Path $destination 'files.json')
Stop-Transcript | Out-Null
