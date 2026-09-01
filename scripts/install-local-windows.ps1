param(
  [switch]$NoBuild,
  [switch]$NoLaunch,
  [switch]$Uninstall,
  [string]$Destination = "",
  [switch]$Help
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ($Help) {
  Write-Output "usage: pnpm install:local -- [--no-build] [--no-launch] [--uninstall] [--destination C:\absolute\path]"
  exit 0
}
if (-not $Destination) { $Destination = Join-Path $env:LOCALAPPDATA "Programs\CodeCaddie Dev" }
if (-not [IO.Path]::IsPathRooted($Destination) -or
    [IO.Path]::GetPathRoot($Destination) -eq $Destination -or
    (Split-Path $Destination -Leaf) -ne "CodeCaddie Dev") {
  throw "destination must be an absolute directory named CodeCaddie Dev, not a drive root"
}

$Source = Join-Path $Root "dist\local\windows\CodeCaddie Dev"
$Data = Join-Path $env:APPDATA "CodeCaddie Dev"
$Shortcut = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\CodeCaddie Dev.lnk"
$Executable = Join-Path $Destination "bin\codecaddie.exe"

function Stop-CodeCaddieDev {
  $processes = Get-Process -Name codecaddie -ErrorAction SilentlyContinue | Where-Object {
    try { $_.Path -eq $Executable } catch { $false }
  }
  foreach ($process in $processes) {
    $null = $process.CloseMainWindow()
    if (-not $process.WaitForExit(5000)) { throw "CodeCaddie Dev is still running; quit it and retry" }
  }
}

function Remove-Associations {
  Remove-Item "HKCU:\Software\Classes\codecaddie-dev" -Recurse -Force -ErrorAction SilentlyContinue
  # Invitation associations from earlier builds are removed if present.
  Remove-Item "HKCU:\Software\Classes\.codecaddie-dev-invite" -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item "HKCU:\Software\Classes\CodeCaddieDev.Invitation" -Recurse -Force -ErrorAction SilentlyContinue
}

if ($Uninstall) {
  Stop-CodeCaddieDev
  if (Test-Path $Destination) { Remove-Item -Recurse -Force $Destination }
  Remove-Item $Shortcut -Force -ErrorAction SilentlyContinue
  Remove-Associations
  Write-Output "Removed $Destination"
  Write-Output "Preserved developer data at $Data"
  exit 0
}

foreach ($tool in @("node", "pnpm", "cargo", "git")) {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "missing prerequisite: $tool" }
}

if (-not $NoBuild) {
  $Version = (Get-Content (Join-Path $Root "package.json") | ConvertFrom-Json).version
  & (Join-Path $Root "scripts\package-windows.ps1") -Version "$Version-dev" -Channel dev
}
if (-not (Test-Path (Join-Path $Source "bin\codecaddie.exe")) -or
    -not (Test-Path (Join-Path $Source "bin\codecaddie-core.exe")) -or
    -not (Test-Path (Join-Path $Source "bin\codecaddie-updater.exe"))) {
  throw "local package is incomplete; run without --no-build"
}

Stop-CodeCaddieDev
$Parent = Split-Path -Parent $Destination
New-Item -ItemType Directory -Force $Parent | Out-Null
$Staging = Join-Path $Parent ".CodeCaddie Dev.staging.$PID"
$Backup = Join-Path $Parent ".CodeCaddie Dev.backup.$PID"
Remove-Item $Staging, $Backup -Recurse -Force -ErrorAction SilentlyContinue
Copy-Item $Source $Staging -Recurse
if (-not (Test-Path (Join-Path $Staging "bin\codecaddie.exe"))) { throw "staged application is incomplete" }
try {
  if (Test-Path $Destination) { Move-Item $Destination $Backup }
  Move-Item $Staging $Destination
  Remove-Item $Backup -Recurse -Force -ErrorAction SilentlyContinue
} catch {
  Remove-Item $Staging -Recurse -Force -ErrorAction SilentlyContinue
  if ((Test-Path $Backup) -and -not (Test-Path $Destination)) { Move-Item $Backup $Destination }
  throw
}

$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($Shortcut)
$link.TargetPath = $Executable
$link.WorkingDirectory = Split-Path -Parent $Executable
$link.Description = "CodeCaddie developer build"
$link.Save()

New-Item "HKCU:\Software\Classes\codecaddie-dev\shell\open\command" -Force | Out-Null
Set-ItemProperty "HKCU:\Software\Classes\codecaddie-dev" -Name "URL Protocol" -Value ""
Set-ItemProperty "HKCU:\Software\Classes\codecaddie-dev" -Name "(default)" -Value "URL:CodeCaddie Dev"
Set-ItemProperty "HKCU:\Software\Classes\codecaddie-dev\shell\open\command" -Name "(default)" -Value ('"{0}" "%1"' -f $Executable)

Write-Output "Installed $Destination"
Write-Output "Developer data: $Data"
if (-not $NoLaunch) { Start-Process $Executable }
