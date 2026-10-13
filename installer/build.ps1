# Builds the release agent and its MSI.
#   ./installer/build.ps1
# Output: dist/ScreenShotCM-<version>.msi
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "ScreenShotCM $version"

& "$root/scripts/cargo.ps1" build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$payload = Join-Path $root 'target/installer-payload'
Remove-Item -Recurse -Force $payload -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $payload | Out-Null
Copy-Item target/release/screenshotcm.exe $payload
Copy-Item THIRD_PARTY_LICENSES.md $payload
Copy-Item LICENSE (Join-Path $payload "LICENSE.txt")

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$msi = Join-Path $dist "ScreenShotCM-$version.msi"

# WiX 5.0.2 (MS-RL). 6 and 7 require the paid Open Source Maintenance Fee.
$wix = Get-Command wix -ErrorAction SilentlyContinue
if (-not $wix) { throw 'WiX 5 not found: dotnet tool install --global wix --version 5.0.2' }
$wixVersion = (& wix --version) -replace '\+.*$', ''
if (-not $wixVersion.StartsWith('5.')) { throw "WiX $wixVersion found; this installer is built with WiX 5.0.2" }

& wix build installer/ScreenShotCM.wxs -arch x64 `
    -d "Version=$version" -d "Payload=$payload" -d "Assets=$root/assets" `
    -o $msi
if ($LASTEXITCODE -ne 0) { throw 'wix build failed' }

# A fixed name too, for the release's permanent "latest" link:
# releases/latest/download/ScreenShotCM.msi
Copy-Item $msi (Join-Path $dist 'ScreenShotCM.msi') -Force

$hash = (Get-FileHash $msi -Algorithm SHA256).Hash.ToLower()
"$hash  $(Split-Path $msi -Leaf)" | Set-Content "$msi.sha256" -Encoding ascii
Write-Host "Built $msi ($([math]::Round((Get-Item $msi).Length / 1KB)) KB)"
Write-Host "SHA-256 $hash"
