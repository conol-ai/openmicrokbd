[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $FirmwareBin,

    [string] $OutputDir = "dist",

    [ValidateSet("aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc")]
    [string] $Target
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $PSCommandPath
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDir ".."))
$appRoot = Join-Path $repoRoot "app"
$firmwarePath = [System.IO.Path]::GetFullPath($FirmwareBin)
$outputPath = if ([System.IO.Path]::IsPathRooted($OutputDir)) {
    [System.IO.Path]::GetFullPath($OutputDir)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repoRoot $OutputDir))
}

if (-not (Test-Path -LiteralPath $firmwarePath -PathType Leaf)) {
    throw "Firmware image does not exist: $firmwarePath"
}

if (-not $Target) {
    $hostLine = rustc -vV | Select-String '^host: (.+)$'
    if (-not $hostLine) {
        throw "Cannot determine the active Rust host target"
    }
    $Target = $hostLine.Matches[0].Groups[1].Value
}
if ($Target -notin @("aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc")) {
    throw "Unsupported Windows target: $Target"
}

$appMetadata = cargo metadata --manifest-path (Join-Path $appRoot "Cargo.toml") --locked --no-deps --format-version 1 | ConvertFrom-Json
$appPackage = $appMetadata.packages | Where-Object name -EQ "openmicro-app" | Select-Object -First 1
if (-not $appPackage) {
    throw "Cannot determine the OpenMicro app version"
}
$appVersion = $appPackage.version

$firmwareMetadata = cargo metadata --manifest-path (Join-Path $repoRoot "fw\Cargo.toml") --locked --no-deps --format-version 1 | ConvertFrom-Json
$firmwarePackage = $firmwareMetadata.packages | Where-Object name -EQ "openmicro-fw" | Select-Object -First 1
if (-not $firmwarePackage) {
    throw "Cannot determine the OpenMicro firmware version"
}
$firmwareVersion = $firmwarePackage.version
$architecture = if ($Target.StartsWith("aarch64-")) { "aarch64" } else { "x86_64" }
$packageName = "OpenMicro-$appVersion-windows-$architecture"
$stageRoot = Join-Path $outputPath $packageName
$archivePath = Join-Path $outputPath "$packageName.zip"

Push-Location $appRoot
try {
    $previousRustFlags = $env:RUSTFLAGS
    $env:RUSTFLAGS = if ($previousRustFlags) {
        "$previousRustFlags -C target-feature=+crt-static"
    } else {
        "-C target-feature=+crt-static"
    }
    cargo build --release --locked --target $Target --bin openmicro-app
    if ($LASTEXITCODE -ne 0) {
        throw "The Windows release build failed"
    }
} finally {
    $env:RUSTFLAGS = $previousRustFlags
    Pop-Location
}

$builtExecutable = Join-Path $appRoot "target\$Target\release\openmicro-app.exe"
if (-not (Test-Path -LiteralPath $builtExecutable -PathType Leaf)) {
    throw "Release executable was not produced: $builtExecutable"
}

New-Item -ItemType Directory -Force -Path $outputPath | Out-Null
if (Test-Path -LiteralPath $stageRoot) {
    $resolvedStage = [System.IO.Path]::GetFullPath($stageRoot)
    if (-not $resolvedStage.StartsWith($outputPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to replace a staging directory outside the output directory"
    }
    Remove-Item -LiteralPath $resolvedStage -Recurse -Force
}
if (Test-Path -LiteralPath $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}

$firmwareDir = Join-Path $stageRoot "firmware"
$licensesDir = Join-Path $stageRoot "licenses"
New-Item -ItemType Directory -Force -Path $firmwareDir, $licensesDir | Out-Null
Copy-Item -LiteralPath $builtExecutable -Destination (Join-Path $stageRoot "OpenMicro.exe")
Copy-Item -LiteralPath $firmwarePath -Destination (Join-Path $firmwareDir "openmicro-fw.bin")
Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $licensesDir "OpenMicro-LICENSE.txt")
Copy-Item -LiteralPath (Join-Path $appRoot "resources\simple-icons.LICENSE.md") -Destination (Join-Path $licensesDir "SimpleIcons-LICENSE.md")

$firmwareHash = (Get-FileHash -LiteralPath $firmwarePath -Algorithm SHA256).Hash.ToLowerInvariant()
$firmwareManifest = [ordered]@{
    version = $firmwareVersion
    sha256 = $firmwareHash
} | ConvertTo-Json
Set-Content -LiteralPath (Join-Path $firmwareDir "manifest.json") -Value $firmwareManifest -Encoding utf8NoBOM

$readme = @"
OpenMicro $appVersion for Windows $architecture

1. Extract this entire folder before running the app.
2. Run OpenMicro.exe. The app, firmware folder, and licenses must stay together.
3. Start the firmware update so the keyboard enters DFU mode. If OpenMicro then
   shows DFU driver setup, open Zadig, enable Options > List All Devices, choose
   STM32 BOOTLOADER (0483:df11), select WinUSB, and install/replace the driver.

Configuration is stored in your Windows user profile. The portable folder may be moved without losing profiles.
"@
Set-Content -LiteralPath (Join-Path $stageRoot "README-Windows.txt") -Value $readme -Encoding utf8NoBOM

Compress-Archive -LiteralPath $stageRoot -DestinationPath $archivePath -CompressionLevel Optimal
if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
    throw "Windows package was not created"
}

Write-Output $archivePath
