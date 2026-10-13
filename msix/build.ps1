# Builds the Microsoft Store package: dist/ScreenShotCM-<version>.msix
#
#   ./msix/build.ps1 -IdentityName <Package/Identity/Name> `
#                    -Publisher "<Package/Identity/Publisher>" `
#                    -PublisherDisplayName "<Publisher display name>"
#
# The three values come from Partner Center → the app → Product management →
# Product identity. Without them the package gets local test values, good
# for checking it builds and installs, not for submitting. The Store signs
# the package itself: upload it unsigned.
#
# makeappx.exe is in Microsoft.Windows.SDK.BuildTools (nuget.org), unpacked
# under %LOCALAPPDATA%\Programs\windows-sdk-buildtools; SDK_BUILDTOOLS
# overrides that.
param(
    [string]$IdentityName = 'ScreenShotCM.Local',
    [string]$Publisher = 'CN=ScreenShotCM Local',
    [string]$PublisherDisplayName = 'ScreenShotCM'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$tools = if ($env:SDK_BUILDTOOLS) { $env:SDK_BUILDTOOLS } else { Join-Path $env:LOCALAPPDATA 'Programs\windows-sdk-buildtools' }
$makeappx = Get-ChildItem $tools -Recurse -Filter makeappx.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.DirectoryName -like '*\x64' } | Select-Object -First 1
if (-not $makeappx) { throw "makeappx.exe not found under $tools" }

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
# The Store wants four parts and the last one 0.
$packageVersion = "$version.0"
Write-Host "ScreenShotCM $version → MSIX $packageVersion ($IdentityName)"

& "$root/scripts/cargo.ps1" build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$stage = Join-Path $root 'target/msix'
if (Test-Path $stage) { Get-ChildItem $stage | Remove-Item -Recurse -Force }
New-Item -ItemType Directory -Force (Join-Path $stage 'Assets') | Out-Null
Copy-Item target/release/screenshotcm.exe $stage
Copy-Item THIRD_PARTY_LICENSES.md $stage

$p = Start-Process (Join-Path $stage 'screenshotcm.exe') -ArgumentList '--export-store-assets', (Join-Path $stage 'Assets') -Wait -PassThru
if ($p.ExitCode -ne 0) { throw 'exporting the store assets failed' }

$manifest = Get-Content msix/AppxManifest.xml -Raw
$manifest = $manifest.Replace('{{IdentityName}}', $IdentityName).
    Replace('{{Publisher}}', [Security.SecurityElement]::Escape($Publisher)).
    Replace('{{PublisherDisplayName}}', [Security.SecurityElement]::Escape($PublisherDisplayName)).
    Replace('{{Version}}', $packageVersion)
[IO.File]::WriteAllText((Join-Path $stage 'AppxManifest.xml'), $manifest, (New-Object Text.UTF8Encoding $false))

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$msix = Join-Path $dist "ScreenShotCM-$version.msix"
& $makeappx.FullName pack /d $stage /p $msix /o
if ($LASTEXITCODE -ne 0) { throw 'makeappx pack failed' }
Write-Host "Built $msix ($([math]::Round((Get-Item $msix).Length / 1KB)) KB)"
