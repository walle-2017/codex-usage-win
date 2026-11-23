Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Join-Path $PSScriptRoot '..'
$install = Get-Content -Raw (Join-Path $root 'scripts/install.ps1')
$installDocs = Get-Content -Raw (Join-Path $root 'docs/installation.md')
$readme = Get-Content -Raw (Join-Path $root 'README.md')
$readmeZh = Get-Content -Raw (Join-Path $root 'README.zh-CN.md')
$maintenance = Get-Content -Raw (Join-Path $root 'docs/MAINTENANCE.md')
$icons = Get-Content -Raw (Join-Path $root 'docs/settings-icons-svg.md')
$packaging = Get-Content -Raw (Join-Path $root 'packaging/README.md')
$generator = Get-Content -Raw (Join-Path $root 'scripts/generate_icons.py')
$store = Get-Content -Raw (Join-Path $root '.github/workflows/store-msix.yml')
if (-not $install.Contains("Programs\CodexUsageWin") -or
    -not $installDocs.Contains("Programs\CodexUsageWin") -or
    -not $installDocs.Contains("Uninstall\CodexUsageWin") -or
    $installDocs -match 'Programs\\CodexUsage(?!Win)' -or
    $installDocs -match 'Uninstall\\CodexUsage(?!Win)') {
    throw 'Installation documentation must match the actual installer identity.'
}
if ($readme -match 'Adaptive taskbar layouts' -or
    $readmeZh -match '自适应三种' -or
    $readme -notmatch 'Default and Minimal taskbar layouts' -or
    $readmeZh -notmatch '默认和极简两种可选') {
    throw 'READMEs must expose two actual selectable taskbar layouts.'
}
if ($maintenance -match 'feature/vertical-taskbar-support' -or
    $icons -notmatch 'white coverage mask|untinted coverage mask' -and
    $icons -notmatch 'coverage mask') {
    throw 'Maintainer notes or SVG cache documentation are stale.'
}
if ($packaging -notmatch 'cargo build --release --no-default-features --features store' -or
    $store -notmatch "assets/\*\*" -or
    $store -match 'Version="1\.0\.0\.0"' -or
    $store -notmatch 'steps\.package_version\.outputs\.msix_version') {
    throw 'MSIX packaging docs, embedded assets triggers or version handling are inconsistent.'
}
if ($generator -notmatch 'DOC_ICON_DIR' -or
    $generator -match 'codex-usage-icon\.png') {
    throw 'Icon generator must output the actual documented icon asset.'
}
$required = @('src/icons/icon.ico', 'src/icons/256x256.png', 'docs/assets/icon.png')
foreach ($file in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $root $file) -PathType Leaf)) {
        throw "Missing referenced runtime/documentation icon: $file"
    }
}
Write-Output 'PASS: audited documentation, dynamic Store packaging, and retained icon assets are consistent.'
