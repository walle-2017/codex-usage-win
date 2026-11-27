param(
    [string]$OutputDir = "dist",
    [string]$PackageVersion = "",
    [Parameter(Mandatory = $true)][ValidateSet("Mark80", "MarkOthers", "Fit80")][string]$ProbeVariant
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

if ([string]::IsNullOrWhiteSpace($PackageVersion)) {
    $cargo = Get-Content -Raw (Join-Path $repoRoot "Cargo.toml")
    $match = [regex]::Match(
        $cargo,
        '(?m)^version\s*=\s*"(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)"'
    )
    if (-not $match.Success) {
        throw "Unable to read package version from Cargo.toml."
    }
    $PackageVersion = "{0}.{1}.{2}.0" -f `
        $match.Groups["major"].Value, `
        $match.Groups["minor"].Value, `
        $match.Groups["patch"].Value
}

if ($PackageVersion -notmatch '^\d+\.\d+\.\d+\.0$') {
    throw "MSIX version must use four numeric parts and end in .0. Actual: $PackageVersion"
}

$exePath = Join-Path $repoRoot "target\release\codex-usage-win.exe"
if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
    throw "Release executable not found: $exePath. Run 'cargo build --release' first."
}

$sourceIcon = Join-Path $repoRoot "docs\assets\icon.png"
if (-not (Test-Path -LiteralPath $sourceIcon -PathType Leaf)) {
    throw "Source icon not found: $sourceIcon"
}

$makeAppx = Get-ChildItem `
    -Path "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" `
    -ErrorAction SilentlyContinue |
    Sort-Object FullName -Descending |
    Select-Object -First 1

if (-not $makeAppx) {
    throw "makeappx.exe was not found in the Windows 10 SDK."
}

$outputRoot = if ([System.IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir
} else {
    Join-Path $repoRoot $OutputDir
}
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null

$workRoot = Join-Path $repoRoot "target\msix"
$stage = Join-Path $workRoot "stage"
Remove-Item -Recurse -Force $workRoot -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $stage "Assets") | Out-Null

Copy-Item -LiteralPath $exePath -Destination (Join-Path $stage "codex-usage-win.exe")

$manifestTemplate = Get-Content -Raw (Join-Path $PSScriptRoot "AppxManifest.xml")
$manifest = $manifestTemplate.Replace("__PACKAGE_VERSION__", $PackageVersion).Replace("__PROBE_VARIANT__", $ProbeVariant)
[System.IO.File]::WriteAllText(
    (Join-Path $stage "AppxManifest.xml"),
    $manifest,
    [System.Text.UTF8Encoding]::new($false)
)

Add-Type -AssemblyName System.Drawing

function Write-ScaledPng {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)][int]$Width,
        [Parameter(Mandatory = $true)][int]$Height,
        [switch]$OpaqueBlackBackground
    )

    $sourceImage = [System.Drawing.Image]::FromFile($Source)
    try {
        $bitmap = New-Object System.Drawing.Bitmap($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $bitmap.SetResolution(96, 96)
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                if ($OpaqueBlackBackground) {
                    $graphics.Clear([System.Drawing.Color]::Black)
                    $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceOver
                } else {
                    $graphics.Clear([System.Drawing.Color]::Transparent)
                    $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
                }
                $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.DrawImage(
                    $sourceImage,
                    [System.Drawing.Rectangle]::new(0, 0, $Width, $Height)
                )
            }
            finally {
                $graphics.Dispose()
            }

            $bitmap.Save($Destination, [System.Drawing.Imaging.ImageFormat]::Png)
        }
        finally {
            $bitmap.Dispose()
        }
    }
    finally {
        $sourceImage.Dispose()
    }
}

$assets = Join-Path $stage "Assets"

# Scale-qualified assets serve high-DPI Shell surfaces, including Search.
# Always resize directly from the canonical source, not the 44px fallback.
foreach ($scale in @(100, 125, 150, 200, 250, 300, 400)) {
    foreach ($logo in @(@{Name="Square44x44Logo"; Base=44}, @{Name="Square150x150Logo"; Base=150}, @{Name="StoreLogo"; Base=50})) {
        $size = [int][Math]::Ceiling($logo.Base * $scale / 100.0)
        $name = "$($logo.Name).scale-${scale}.png"
        Write-ScaledPng -Source $sourceIcon -Destination (Join-Path $assets $name) -Width $size -Height $size -OpaqueBlackBackground:($logo.Name -eq "StoreLogo")
    }
}

# Target-size variants are selected by the Windows Shell for taskbar and Start.
# Unplated variants preserve the application's own rounded transparent artwork.
foreach ($size in @(16, 20, 24, 30, 32, 36, 40, 44, 48, 60, 64, 72, 80, 96, 256)) {
    foreach ($suffix in @("", "_altform-unplated", "_altform-lightunplated")) {
        $name = "Square44x44Logo.targetsize-${size}${suffix}.png"
        Write-ScaledPng -Source $sourceIcon -Destination (Join-Path $assets $name) -Width $size -Height $size
    }
}

# DIAGNOSTIC ONLY: isolate encoding, marker geometry, and chroma.
python (Join-Path $repoRoot "scripts\search_geometry_probe.py") $assets $ProbeVariant (Join-Path $outputRoot "search-geometry-probe.json")
if ($LASTEXITCODE -ne 0) { throw "Diagnostic icon preparation failed." }

# Qualified visual assets require a Package Resource Index for Shell lookup.
$makePri = Join-Path $makeAppx.DirectoryName "makepri.exe"
if (-not (Test-Path -LiteralPath $makePri -PathType Leaf)) {
    throw "makepri.exe was not found alongside MakeAppx."
}
$priConfig = Join-Path $workRoot "priconfig.xml"
$priPath = Join-Path $stage "resources.pri"
& $makePri createconfig /cf $priConfig /dq en-US /o
if ($LASTEXITCODE -ne 0) { throw "MakePri createconfig failed." }

# Keep all qualifiers in the main PRI for a standalone MSIX, rather than
# generating separate resource-package indexes that Shell may not load.
[xml]$config = Get-Content -Raw -LiteralPath $priConfig
$packagingNodes = @($config.SelectNodes("//*[local-name()='packaging']"))
foreach ($node in $packagingNodes) {
    [void]$node.ParentNode.RemoveChild($node)
}
$config.Save($priConfig)

& $makePri new /pr $stage /cf $priConfig /of $priPath /o
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $priPath -PathType Leaf)) {
    throw "MakePri failed to generate resources.pri."
}
$splitIndexes = @(Get-ChildItem -LiteralPath $stage -Filter 'resources.*.pri' -File)
if ($splitIndexes.Count -ne 0) {
    throw "Standalone MSIX must contain a single resources.pri."
}

$packageName = "codex-usage-win_{0}_x64.msix" -f $PackageVersion
$packagePath = Join-Path $outputRoot $packageName
Remove-Item -Force $packagePath -ErrorAction SilentlyContinue

& $makeAppx.FullName pack /d $stage /p $packagePath /o /h SHA256
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx failed with exit code $LASTEXITCODE."
}

if (-not (Test-Path -LiteralPath $packagePath -PathType Leaf)) {
    throw "MSIX package was not created: $packagePath"
}

Write-Host "MSIX created: $packagePath"
Write-Host "Package version: $PackageVersion"
Write-Host "MakeAppx: $($makeAppx.FullName)"
