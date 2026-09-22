param(
  [string] $InstallRoot = (Join-Path $HOME ".local")
)

$ErrorActionPreference = "Stop"
$repository = "mantty/tokamak"
$api = "https://api.github.com/repos/$repository"
$targets = @(
  "android-arm64",
  "ios-arm64",
  "ios-simulator-arm64",
  "ios-simulator-x64",
  "macos-arm64",
  "macos-x64",
  "windows-x64"
)

if ($env:OS -ne "Windows_NT" -or -not [Environment]::Is64BitOperatingSystem) {
  throw "tokamak requires 64-bit Windows"
}
if ([string]::IsNullOrWhiteSpace($InstallRoot)) {
  throw "InstallRoot must not be empty"
}
if (-not (Get-Command tar -ErrorAction SilentlyContinue)) {
  throw "tar is required"
}

$apiHeaders = @{
  Accept = "application/vnd.github+json"
  "X-GitHub-Api-Version" = "2022-11-28"
}
$githubToken = if (-not [string]::IsNullOrWhiteSpace($env:GH_TOKEN)) {
  $env:GH_TOKEN
} else {
  $env:GITHUB_TOKEN
}
if (-not [string]::IsNullOrWhiteSpace($githubToken)) {
  $apiHeaders.Authorization = "Bearer $githubToken"
}

$releases = @(Invoke-RestMethod -Uri "$api/releases?per_page=1" -Headers $apiHeaders)
if ($releases.Count -eq 0) {
  throw "no tokamak release was found"
}
$tag = $releases[0].tag_name
if ($tag -notmatch "^[A-Za-z0-9._-]+$") {
  throw "unsupported release tag: $tag"
}

$releaseUrl = "https://github.com/$repository/releases/download/$tag"
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) "tokamak-install-$([guid]::NewGuid())"
$stagedCli = $null
$stagedPlatformPacks = $null

try {
  $cliDirectory = Join-Path $temporary "cli"
  $platformPacks = Join-Path $temporary "platform-packs"
  New-Item -ItemType Directory -Force -Path $cliDirectory, $platformPacks | Out-Null

  Write-Output "Downloading tokamak $tag for windows-x64..."
  $cliArchive = Join-Path $temporary "tokamak-cli-windows-x64.zip"
  Invoke-WebRequest -UseBasicParsing -Uri "$releaseUrl/tokamak-cli-windows-x64.zip" -OutFile $cliArchive
  Expand-Archive -LiteralPath $cliArchive -DestinationPath $cliDirectory
  $cli = Join-Path $cliDirectory "tok.exe"
  if (-not (Test-Path -LiteralPath $cli -PathType Leaf)) {
    throw "CLI archive does not contain tok.exe"
  }

  foreach ($target in $targets) {
    Write-Output "Downloading platform pack $target..."
    $archive = Join-Path $temporary "tokamak-platform-pack-$target.tar.gz"
    $destination = Join-Path $platformPacks $target
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Invoke-WebRequest -UseBasicParsing -Uri "$releaseUrl/tokamak-platform-pack-$target.tar.gz" -OutFile $archive
    & tar -xzf $archive -C $destination
    if ($LASTEXITCODE -ne 0) {
      throw "failed to extract $target"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $destination "platform-pack.json") -PathType Leaf)) {
      throw "$target archive does not contain platform-pack.json"
    }
  }

  $InstallRoot = [System.IO.Path]::GetFullPath($InstallRoot)
  $binDirectory = Join-Path $InstallRoot "bin"
  $shareDirectory = Join-Path $InstallRoot "share/tokamak"
  $platformPackDirectory = Join-Path $shareDirectory "platform-packs"
  New-Item -ItemType Directory -Force -Path $binDirectory, $shareDirectory | Out-Null

  $stagedCli = Join-Path $binDirectory ".tokamak-install-$PID.exe"
  $stagedPlatformPacks = Join-Path $shareDirectory ".platform-packs-install-$PID"
  Copy-Item -LiteralPath $cli -Destination $stagedCli
  Copy-Item -LiteralPath $platformPacks -Destination $stagedPlatformPacks -Recurse
  Remove-Item -LiteralPath $platformPackDirectory -Recurse -Force -ErrorAction SilentlyContinue
  Move-Item -LiteralPath $stagedPlatformPacks -Destination $platformPackDirectory
  $stagedPlatformPacks = $null
  $installedCli = Join-Path $binDirectory "tok.exe"
  Remove-Item -LiteralPath $installedCli -Force -ErrorAction SilentlyContinue
  Move-Item -LiteralPath $stagedCli -Destination $installedCli
  $stagedCli = $null

  Write-Output ""
  Write-Output "Installed tokamak $tag in $binDirectory"
  Write-Output "Installed platform packs in $platformPackDirectory"
  if (@($env:Path -split [System.IO.Path]::PathSeparator) -notcontains $binDirectory) {
    Write-Output "Add $binDirectory to PATH, then open a new terminal."
  }
  Write-Output "Run tok targets to verify the installation."
} finally {
  Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
  if ($stagedCli) {
    Remove-Item -LiteralPath $stagedCli -Force -ErrorAction SilentlyContinue
  }
  if ($stagedPlatformPacks) {
    Remove-Item -LiteralPath $stagedPlatformPacks -Recurse -Force -ErrorAction SilentlyContinue
  }
}
