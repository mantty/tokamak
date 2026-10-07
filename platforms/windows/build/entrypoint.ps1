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
  $runtime = Join-Path $pack "lib/TokamakRuntime"
  $libraries = [System.IO.File]::ReadAllText((Join-Path $runtime "link-libraries")).Trim() -split '\s+'
  $exports = @([System.IO.File]::ReadAllLines((Join-Path $InputDirectory "metadata/exported-symbols")) | ForEach-Object { "/EXPORT:$_" })
  $importLibrary = Join-Path $InputDirectory "runtime.lib"
  & link.exe /NOLOGO "/OUT:$Executable" /SUBSYSTEM:WINDOWS /ENTRY:wWinMainCRTStartup /OPT:REF /OPT:ICF /INCREMENTAL:NO /NXCOMPAT /DYNAMICBASE /HIGHENTROPYVA "/IMPLIB:$importLibrary" (Join-Path $runtime "tokamak.lib") @libraries @exports
  if ($LASTEXITCODE -ne 0) {
    throw "linking the tokamak runtime failed with status $LASTEXITCODE"
  }
}

# The CLI runs the entrypoint from the pack root.
$pack = (Get-Location).Path
$output = [System.IO.Path]::GetFullPath($OutputDirectory)
$appName = Read-TokamakValue "app-name"
$appSlug = Read-TokamakValue "app-slug"
$appHost = Read-TokamakValue "host"
$icon = $env:TOKAMAK_WINDOWS_ICON
if ($icon -and (-not (Test-Path -LiteralPath $icon -PathType Leaf) -or [System.IO.Path]::GetExtension($icon) -ne ".ico")) {
  throw "windows.icon must be an .ico file: $icon"
}
$app = Join-Path $output "app"

if (Test-Path $output) {
  Remove-Item -Recurse -Force $output
}
New-Item -ItemType Directory -Force -Path $app | Out-Null
Copy-Item (Join-Path $InputDirectory "app/*") $app -Recurse -Force
Invoke-RuntimeLink (Join-Path $output "$appSlug.exe")
if ($icon) {
  Copy-Item -LiteralPath $icon (Join-Path $output "AppIcon.ico")
}

$config = [ordered]@{
  name = $appName
  slug = $appSlug
  host = $appHost
}
if (Test-Path (Join-Path $InputDirectory "metadata/dev-endpoint")) {
  $config.devEndpoint = Read-TokamakValue "dev-endpoint"
  $config.devSessionToken = Read-TokamakValue "dev-session-token"
}
$config = $config | ConvertTo-Json
$encoding = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText((Join-Path $output "tokamak.json"), $config, $encoding)
