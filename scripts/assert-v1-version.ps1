$ErrorActionPreference = 'Stop'

$ExpectedVersion = '1.0.5'

$CargoToml = Get-Content -Raw -LiteralPath 'Cargo.toml'
$PackageMatch = [regex]::Match(
    $CargoToml,
    '(?ms)^\[package\]\s*.*?^version\s*=\s*"([^"]+)"'
)
if (-not $PackageMatch.Success) {
    throw 'Unable to locate the package version in Cargo.toml.'
}
if ($PackageMatch.Groups[1].Value -ne $ExpectedVersion) {
    throw "Cargo.toml package version must be $ExpectedVersion, found $($PackageMatch.Groups[1].Value)."
}

$CargoLock = Get-Content -Raw -LiteralPath 'Cargo.lock'
$LockMatch = [regex]::Match(
    $CargoLock,
    '(?ms)^\[\[package\]\]\s*\r?\nname\s*=\s*"codex-usage-win"\s*\r?\nversion\s*=\s*"([^"]+)"'
)
if (-not $LockMatch.Success) {
    throw 'Unable to locate codex-usage-win in Cargo.lock.'
}
if ($LockMatch.Groups[1].Value -ne $ExpectedVersion) {
    throw "Cargo.lock codex-usage-win version must be $ExpectedVersion, found $($LockMatch.Groups[1].Value)."
}

$WindowSource = Get-Content -Raw -LiteralPath 'src/window.rs'
if ($WindowSource -notmatch 'env!\("CARGO_PKG_VERSION"\)') {
    throw 'The version menu must derive its value from CARGO_PKG_VERSION.'
}
if ($WindowSource -notmatch 'format!\("v\{\}"\s*,\s*env!\("CARGO_PKG_VERSION"\)\)') {
    throw 'The version menu must render the package version with a v prefix.'
}

$BuildSource = Get-Content -Raw -LiteralPath 'build.rs'
if ($BuildSource -notmatch 'env!\("CARGO_PKG_VERSION"\)') {
    throw 'Windows executable version metadata must derive from CARGO_PKG_VERSION.'
}

$OldProductVersion = '1.9' + '.1'
$OldProductVersionHits = git grep -n -F $OldProductVersion -- . ':(exclude)Cargo.lock' ':(exclude)scripts/assert-v1-version.ps1' 2>$null
if ($LASTEXITCODE -eq 0 -and $OldProductVersionHits) {
    throw "Old product version $OldProductVersion remains in tracked files:`n$($OldProductVersionHits -join "`n")"
}
if ($LASTEXITCODE -notin @(0, 1)) {
    throw 'git grep failed while checking for stale product version strings.'
}

Write-Output "Version contract verified: v$ExpectedVersion"
exit 0
