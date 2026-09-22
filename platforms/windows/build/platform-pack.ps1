param(
  [Parameter(Position = 0)]
  [string] $Command,
  [Parameter(Position = 1)]
  [string] $Target,
  [Parameter(Position = 2)]
  [string] $RustTarget,
  [Parameter(Position = 3)]
  [string] $Output
)

$ErrorActionPreference = "Stop"

if ($Command -ne "build" -or $Target -ne "windows-x64" -or $RustTarget -ne "x86_64-pc-windows-msvc" -or [string]::IsNullOrWhiteSpace($Output)) {
  throw "usage: platform-pack.ps1 build windows-x64 x86_64-pc-windows-msvc OUTPUT"
}

$workspace = (Get-Location).Path
$output = [System.IO.Path]::GetFullPath($Output)

& cargo build --package windows-shell --release --target $RustTarget
if ($LASTEXITCODE -ne 0) {
  throw "Windows app shell build failed with status $LASTEXITCODE"
}

$executable = Join-Path $workspace "target/$RustTarget/release/tokamak-shell-windows.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
  throw "Windows app shell was not produced: $executable"
}

if (Test-Path -LiteralPath $output) {
  Remove-Item -Recurse -Force $output
}
New-Item -ItemType Directory -Force -Path (Join-Path $output "bin"), (Join-Path $output "build") | Out-Null
Copy-Item $executable (Join-Path $output "bin/tokamak-shell-windows.exe")
Copy-Item (Join-Path $workspace "platforms/windows/build/entrypoint.ps1") (Join-Path $output "build/entrypoint.ps1")
