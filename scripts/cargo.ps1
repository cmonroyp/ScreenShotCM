# Runs cargo with llvm-mingw on PATH for this process only.
#   ./scripts/cargo.ps1 build --release
#   ./scripts/cargo.ps1 test
# LLVM_MINGW overrides where llvm-mingw was unpacked.
$ErrorActionPreference = 'Stop'
$llvm = if ($env:LLVM_MINGW) { $env:LLVM_MINGW } else { Join-Path $env:LOCALAPPDATA 'Programs\llvm-mingw' }
if (-not (Test-Path (Join-Path $llvm 'bin\x86_64-w64-mingw32-clang.exe'))) {
    throw "llvm-mingw not found at $llvm. Unpack a release from https://github.com/mstorsjo/llvm-mingw/releases there, or set LLVM_MINGW."
}
$env:PATH = "$llvm\bin;$env:USERPROFILE\.cargo\bin;$env:PATH"
& cargo @args
exit $LASTEXITCODE
