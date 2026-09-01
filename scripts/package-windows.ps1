param(
  [string]$Version = "0.4.0",
  [ValidateSet("stable", "beta", "dev")]
  [string]$Channel = "stable",
  [string]$OutputDir = "",
  [switch]$PrepareOnly,
  [switch]$PackagePrepared,
  [switch]$UseExistingBuild
)

$ErrorActionPreference = "Stop"

function Resolve-CodeCaddieZig {
  $RequiredVersion = "0.16.0"
  $Candidates = @()
  if ($env:NATIVE_SDK_ZIG) {
    $Candidates += $env:NATIVE_SDK_ZIG
  } else {
    $PathZig = Get-Command zig -CommandType Application -ErrorAction SilentlyContinue
    if ($PathZig) { $Candidates += $PathZig.Source }
    $NativeHome = if ($env:NATIVE_SDK_HOME) {
      $env:NATIVE_SDK_HOME
    } elseif ($env:USERPROFILE) {
      Join-Path $env:USERPROFILE ".native"
    }
    if ($NativeHome) {
      $ManagedZig = Join-Path $NativeHome "toolchains\zig-$RequiredVersion\zig"
      $Candidates += "$ManagedZig.exe", $ManagedZig
    }
  }
  foreach ($Candidate in $Candidates | Select-Object -Unique) {
    try {
      $ActualVersion = (& $Candidate version 2>$null).Trim()
      if ($LASTEXITCODE -eq 0 -and $ActualVersion -eq $RequiredVersion) {
        return $Candidate
      }
    } catch {
      # Try the next Native SDK-compatible location before reporting one
      # actionable toolchain error below.
    }
  }
  throw "Zig $RequiredVersion is required. Install it on PATH or set NATIVE_SDK_ZIG to the exact executable."
}

if ($PrepareOnly -and $PackagePrepared) { throw "PrepareOnly and PackagePrepared are mutually exclusive" }
if ($PackagePrepared -and $UseExistingBuild) { throw "PackagePrepared and UseExistingBuild are mutually exclusive" }
if ($UseExistingBuild -and ($env:CI -ne "true" -or $Channel -ne "dev")) {
  throw "UseExistingBuild is restricted to dev packaging on CI"
}
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$AppName = if ($Channel -eq "dev") { "CodeCaddie Dev" } else { "CodeCaddie" }
$ReleaseChannel = $Channel -ne "dev"
if (-not $OutputDir) {
  $OutputDir = if ($Channel -eq "dev") { "dist\local\windows" } else { "dist\windows" }
}
if (-not [IO.Path]::IsPathRooted($OutputDir)) { $OutputDir = Join-Path $Root $OutputDir }
$Output = Join-Path $OutputDir $AppName
$Archive = Join-Path $OutputDir "CodeCaddie-$Version-Windows-x64.zip"
$Msi = Join-Path $OutputDir "CodeCaddie-$Version-Windows-x64.msi"
$BuildNumber = if ($env:CODECADDIE_BUILD_NUMBER) { $env:CODECADDIE_BUILD_NUMBER } else { (git rev-list --count HEAD) }
$MsiBuildNumber = if ($env:CODECADDIE_MSI_BUILD_NUMBER) { $env:CODECADDIE_MSI_BUILD_NUMBER } else { $BuildNumber }
$CommitSha = if ($env:CODECADDIE_COMMIT_SHA) { $env:CODECADDIE_COMMIT_SHA } else { (git rev-parse HEAD) }

$env:CODECADDIE_BUILD_NUMBER = $BuildNumber
$env:CODECADDIE_COMMIT_SHA = $CommitSha
if ($ReleaseChannel) {
  foreach ($Required in @(
    "CODECADDIE_GITHUB_REPOSITORY_ID",
    "CODECADDIE_WINDOWS_PUBLISHER"
  )) {
    if (-not [Environment]::GetEnvironmentVariable($Required)) {
      throw "stable packaging requires $Required"
    }
  }
  if ($env:CODECADDIE_GITHUB_REPOSITORY_ID -notmatch '^[1-9][0-9]*$') {
    throw "CODECADDIE_GITHUB_REPOSITORY_ID must be a positive numeric repository ID"
  }
}

Set-Location $Root
if (-not $PackagePrepared) {
  if (-not $UseExistingBuild) {
    & cargo build --release --package codecaddie-core --locked
    if ($LASTEXITCODE -ne 0) { throw "Rust release build failed" }
    & pnpm exec native check apps/desktop --strict
    if ($LASTEXITCODE -ne 0) { throw "Native SDK validation failed" }
    $Zig = Resolve-CodeCaddieZig
    Push-Location "apps\desktop"
    try {
      & $Zig build test --summary all "-Dchannel=$Channel" -Dcpu=baseline -j1
      if ($LASTEXITCODE -ne 0) { throw "serialized native test warm-up failed" }
      & $Zig build --summary all -Doptimize=ReleaseFast "-Dchannel=$Channel" -Dtrace=off -Dstrip=true -Dcpu=baseline -j1
      if ($LASTEXITCODE -ne 0) { throw "serialized native release build failed" }
    } finally {
      Pop-Location
    }
  }
  $BuildInputs = @(
    "target\release\codecaddie-core.exe",
    "target\release\codecaddie-updater.exe",
    "apps\desktop\zig-out\bin\codecaddie.exe"
  )
  $BuildInputHashes = @{}
  foreach ($RequiredBuild in $BuildInputs) {
    if (-not (Test-Path $RequiredBuild -PathType Leaf)) {
      throw "existing Windows build is missing: $RequiredBuild"
    }
    $BuildInputHashes[$RequiredBuild] = (Get-FileHash -Algorithm SHA256 $RequiredBuild).Hash
  }
  node scripts/check-native-credential-boundary.mjs --binary apps\desktop\zig-out\bin\codecaddie.exe --platform windows
  if ($LASTEXITCODE) { throw "native client credential boundary failed" }

  if (Test-Path $Output) { Remove-Item -Recurse -Force $Output }
  New-Item -ItemType Directory -Force $OutputDir | Out-Null
  Push-Location "apps\desktop"
  $PackageExitCode = 0
  try {
    & "..\..\node_modules\.bin\native.cmd" package --target windows --output $Output --binary "zig-out\bin\codecaddie.exe" --assets assets
    $PackageExitCode = $LASTEXITCODE
  } finally {
    Pop-Location
  }
  if ($PackageExitCode -ne 0) { throw "Native SDK failed to package the Windows application" }
  foreach ($RequiredBuild in $BuildInputs) {
    $CurrentHash = (Get-FileHash -Algorithm SHA256 $RequiredBuild).Hash
    if ($CurrentHash -ne $BuildInputHashes[$RequiredBuild]) {
      throw "Native SDK packaging modified an existing Windows build: $RequiredBuild"
    }
  }
  Copy-Item "target\release\codecaddie-core.exe" (Join-Path $Output "bin\codecaddie-core.exe")
  Copy-Item "target\release\codecaddie-updater.exe" (Join-Path $Output "bin\codecaddie-updater.exe")
  Copy-Item "LICENSE" $Output
  Copy-Item "THIRD_PARTY_NOTICES.md" $Output
  Copy-Item "docs\licenses\APACHE-2.0.txt" $Output
  Copy-Item "docs\licenses\GEIST-OFL.txt" $Output
  Copy-Item "docs\licenses\IBM-PLEX-OFL.txt" $Output
  if (!(Test-Path "docs\licenses\RUST-DEPENDENCY-LICENSES.md" -PathType Leaf)) {
    throw "checked-in Rust dependency license bundle is missing"
  }
  Copy-Item "docs\licenses\RUST-DEPENDENCY-LICENSES.md" $Output

  # CLI launcher for coding agents (`codecaddie mcp`). It lives in cli\ rather
  # than bin\ so PATH resolution cannot collide with the desktop codecaddie.exe.
  $Cli = Join-Path $Output "cli"
  New-Item -ItemType Directory -Force -Path $Cli | Out-Null
  Set-Content -Path (Join-Path $Cli "codecaddie.cmd") -Encoding ascii -Value "@echo off`r`n`"%~dp0..\bin\codecaddie-core.exe`" %*"
  @{
    channel = $Channel
    version = $Version
    build = $BuildNumber
    commit = $CommitSha
    appId = if ($Channel -eq "dev") { "org.codecaddie.desktop.dev" } else { "org.codecaddie.desktop" }
  } | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $Output "build-info.json")
} elseif (-not (Test-Path (Join-Path $Output "bin\codecaddie.exe"))) {
  throw "prepared Windows payload is missing at $Output"
}

if ($PrepareOnly) {
  Write-Output $Output
  exit 0
}

if ($ReleaseChannel) {
  if ($env:CODECADDIE_WINDOWS_OPEN_SOURCE_SIGNING_APPROVED -ne "1") {
    throw "stable and beta Windows packaging is disabled until approved open-source Authenticode signing is available"
  }
  foreach ($Binary in @("codecaddie-core.exe", "codecaddie-updater.exe", "codecaddie.exe")) {
    & signtool verify /pa /all /tw (Join-Path $Output "bin\$Binary")
    if ($LASTEXITCODE -ne 0) { throw "$Binary must be signed before MSI packaging" }
  }
}

if (Test-Path $Archive) { Remove-Item -Force $Archive }
Compress-Archive -Path "$Output\*" -DestinationPath $Archive

if ($ReleaseChannel) {
  $Wix = Get-Command wix -ErrorAction SilentlyContinue
  if (-not $Wix) { throw "WiX Toolset 7.0.0 is required for stable MSI packaging" }
  $WixVersion = (& wix --version).Trim()
  if (-not $WixVersion.StartsWith("7.0.0")) { throw "WiX 7.0.0 is required; found $WixVersion" }
  if (Test-Path $Msi) { Remove-Item -Force $Msi }
  $WixSource = Join-Path $OutputDir "CodeCaddie.generated.wxs"
  node "scripts\generate-wix.mjs" --payload $Output --version $Version --build $MsiBuildNumber --output $WixSource
  & wix build $WixSource `
    -arch x64 `
    -bindpath "Payload=$Output" `
    -out $Msi
  if ($LASTEXITCODE -ne 0) { throw "WiX failed to create the MSI" }
  & wix msi validate $Msi
  if ($LASTEXITCODE -ne 0) { throw "WiX MSI validation failed" }
}
Write-Output $Output
Write-Output $Archive
if (Test-Path $Msi) { Write-Output $Msi }
