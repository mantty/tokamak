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

# Put the Microsoft C++ linker, from Visual Studio or its Build Tools, and
# the Windows libraries it links on this process's paths. A Visual Studio
# developer shell for x64 already has them.
function Use-MicrosoftLinker {
  if ($env:VSCMD_ARG_TGT_ARCH -eq "x64") {
    return
  }
  $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
  $installation = $null
  if (Test-Path -LiteralPath $vswhere) {
    $installation = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  }
  if ([string]::IsNullOrWhiteSpace($installation)) {
    throw "Windows builds link the tokamak runtime with the Microsoft C++ build tools, which are not installed. Install Visual Studio or Visual Studio Build Tools with the `"Desktop development with C++`" workload."
  }
  $vcvars = Join-Path $installation "VC\Auxiliary\Build\vcvars64.bat"
  foreach ($line in (& cmd.exe /d /c "`"$vcvars`" >nul && set")) {
    if ($line -match '^([^=]+)=(.*)$') {
      Set-Item -Path "env:$($Matches[1])" -Value $Matches[2]
    }
  }
}

# Link the shell and runtime into the app's executable, exporting the entry
# points of the runtime parts the app links.
function Invoke-RuntimeLink([string] $Executable) {
  Use-MicrosoftLinker
  $runtime = Join-Path $InputDirectory "runtime/TokamakRuntime"
  $libraries = [System.IO.File]::ReadAllText((Join-Path $runtime "link-libraries")).Trim() -split '\s+'
  $exports = @([System.IO.File]::ReadAllLines((Join-Path $InputDirectory "metadata/exported-symbols")) | ForEach-Object { "/EXPORT:$_" })
  $importLibrary = Join-Path $InputDirectory "runtime.lib"
  & link.exe /NOLOGO "/OUT:$Executable" /SUBSYSTEM:WINDOWS /ENTRY:wWinMainCRTStartup /OPT:REF /OPT:ICF /INCREMENTAL:NO /NXCOMPAT /DYNAMICBASE /HIGHENTROPYVA "/IMPLIB:$importLibrary" (Join-Path $runtime "tokamak.lib") @libraries @exports
  if ($LASTEXITCODE -ne 0) {
    throw "linking the tokamak runtime failed with status $LASTEXITCODE"
  }
}

$output = [System.IO.Path]::GetFullPath($OutputDirectory)
$appName = Read-TokamakValue "app-name"
$appSlug = Read-TokamakValue "app-slug"
$identifier = Read-TokamakValue "identifier"
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
Invoke-RuntimeLink (Join-Path $output "$appSlug.exe")
$icon = Join-Path $InputDirectory "icons/windows/AppIcon.ico"
if (Test-Path -LiteralPath $icon -PathType Leaf) {
  Copy-Item -LiteralPath $icon (Join-Path $output "AppIcon.ico")
}

$config = [ordered]@{
  name = $appName
  slug = $appSlug
  identifier = $identifier
  host = $appHost
}
if (Test-Path (Join-Path $InputDirectory "metadata/version")) {
  $config.version = Read-TokamakValue "version"
}
if (-not [string]::IsNullOrWhiteSpace($devEndpoint)) {
  $config.devEndpoint = $devEndpoint
  $config.devSessionToken = $devSessionToken
}
$config = $config | ConvertTo-Json
$encoding = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText((Join-Path $output "tokamak.json"), $config, $encoding)
