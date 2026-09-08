param(
  [Parameter(Position = 0)]
  [string] $Command,
  [Parameter(Position = 1)]
  [string] $InputDirectory,
  [Parameter(Position = 2)]
  [string] $OutputDirectory
)

$ErrorActionPreference = "Stop"

if ($Command -ne "build" -or [string]::IsNullOrWhiteSpace($InputDirectory) -or [string]::IsNullOrWhiteSpace($OutputDirectory)) {
  throw "usage: entrypoint.ps1 build INPUT OUTPUT"
}

function Read-TokamakValue([string] $Name) {
  $path = Join-Path $InputDirectory "metadata/$Name"
  return [System.IO.File]::ReadAllText($path).Trim()
}

$output = [System.IO.Path]::GetFullPath($OutputDirectory)
$appName = Read-TokamakValue "app-name"
$appHost = Read-TokamakValue "host"
$devEndpoint = $null
$devSessionToken = $null
$devEndpointPath = Join-Path $InputDirectory "metadata/dev-endpoint"
$devSessionTokenPath = Join-Path $InputDirectory "metadata/dev-session-token"
if (Test-Path $devEndpointPath) {
  $devEndpoint = [System.IO.File]::ReadAllText($devEndpointPath).Trim()
}
if (Test-Path $devSessionTokenPath) {
  $devSessionToken = [System.IO.File]::ReadAllText($devSessionTokenPath).Trim()
}
if ([string]::IsNullOrWhiteSpace($devEndpoint) -xor [string]::IsNullOrWhiteSpace($devSessionToken)) {
  throw "development endpoint and session token must be provided together"
}
$app = Join-Path $output "app"

if (Test-Path $output) {
  Remove-Item -Recurse -Force $output
}
New-Item -ItemType Directory -Force -Path $app | Out-Null
Copy-Item (Join-Path $InputDirectory "app/*") $app -Recurse -Force
Copy-Item (Join-Path $InputDirectory "runtime/tokamak-shell-windows.exe") (Join-Path $output "$appName.exe")
$icon = Join-Path $InputDirectory "icons/windows/AppIcon.ico"
if (Test-Path -LiteralPath $icon -PathType Leaf) {
  Copy-Item -LiteralPath $icon (Join-Path $output "AppIcon.ico")
}

$config = [ordered]@{
  name = $appName
  host = $appHost
}
if (-not [string]::IsNullOrWhiteSpace($devEndpoint)) {
  $config.devEndpoint = $devEndpoint
  $config.devSessionToken = $devSessionToken
}
$config = $config | ConvertTo-Json
$encoding = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText((Join-Path $output "tokamak.json"), $config, $encoding)
