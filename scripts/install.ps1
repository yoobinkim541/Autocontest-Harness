# Install `ach`, the harness core that its agents call, into this checkout on Windows.
#
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1
#
# There is no prebuilt Windows binary: this builds with cargo (Rust from https://rustup.rs and the
# MSVC build tools). The binary goes to <checkout>\bin\ach.exe and nowhere else: it is not put on
# your PATH. Sessions the harness starts get it on theirs. Run it again after `git pull` to upgrade.
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) {
    $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (-not (Test-Path $cargo)) { Write-Error 'cargo not found: install Rust from https://rustup.rs'; exit 1 }
}

Write-Output 'building from source'
Push-Location $root
try {
    & $cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}

$bin = Join-Path $root 'bin'
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item -Force (Join-Path $root 'target\release\ach.exe') (Join-Path $bin 'ach.exe')

Write-Output "installed: $bin\ach.exe"
& (Join-Path $bin 'ach.exe') --version
Write-Output "next: open your coding agent in $root and ask it to read INIT.md and set up the harness"
