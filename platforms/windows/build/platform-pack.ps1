param(
  [Parameter(Position = 0)]
  [string] $Command,
  [Parameter(Position = 1)]
  [string] $Target,
  [Parameter(Position = 2)]
  [string] $Output
)

$ErrorActionPreference = "Stop"

if ($Command -ne "build" -or $Target -ne "windows-x64" -or [string]::IsNullOrWhiteSpace($Output)) {
  throw "usage: platform-pack.ps1 build windows-x64 OUTPUT"
}

$workspace = (Get-Location).Path
$rustTarget = "x86_64-pc-windows-msvc"
$output = [System.IO.Path]::GetFullPath($Output)

$libraries = Join-Path $workspace "target/$rustTarget/release/tokamak-link-libraries"
& cargo rustc --package windows-shell --release --target $rustTarget --lib --crate-type staticlib -- --print "native-static-libs=$libraries"
if ($LASTEXITCODE -ne 0) {
  throw "Windows app shell build failed with status $LASTEXITCODE"
}

$library = Join-Path $workspace "target/$rustTarget/release/windows_shell.lib"
if (-not (Test-Path -LiteralPath $library -PathType Leaf) -or -not (Test-Path -LiteralPath $libraries -PathType Leaf)) {
  throw "Windows app shell library was not produced: $library"
}

if (Test-Path -LiteralPath $output) {
  Remove-Item -Recurse -Force $output
}
$runtime = Join-Path $output "lib/TokamakRuntime"
New-Item -ItemType Directory -Force -Path $runtime, (Join-Path $output "build") | Out-Null
# App builds link the shell and runtime with these system libraries.
Copy-Item $library (Join-Path $runtime "tokamak.lib")
Copy-Item $libraries (Join-Path $runtime "link-libraries")
Copy-Item (Join-Path $workspace "platforms/windows/build/entrypoint.ps1") (Join-Path $output "build/entrypoint.ps1")
