$ErrorActionPreference = 'Stop'

$Files = @(
    'README.md',
    'README.zh-CN.md',
    'PRIVACY.md',
    'SECURITY.md',
    'docs/MAINTENANCE.md',
    'docs/installation.md',
    'docs/troubleshooting.md'
)

foreach ($File in $Files) {
    if (-not (Test-Path -LiteralPath $File -PathType Leaf)) {
        throw "Required user-facing document is missing: $File"
    }
}

$Joined = ($Files | ForEach-Object { Get-Content -Raw -LiteralPath $_ }) -join "`n"

foreach ($Forbidden in @(
    'walle-2017/codex-usage-monitor',
    'README-FORK.md',
    '.github/animation.gif',
    'upstream-ray/codex-usage-monitor/releases/latest'
)) {
    if ($Joined.Contains($Forbidden)) {
        throw "Stale documentation text remains: $Forbidden"
    }
}

$Readme = Get-Content -Raw -LiteralPath 'README.md'
$ReadmeZh = Get-Content -Raw -LiteralPath 'README.zh-CN.md'
$Maintenance = Get-Content -Raw -LiteralPath 'docs/MAINTENANCE.md'
$Troubleshooting = Get-Content -Raw -LiteralPath 'docs/troubleshooting.md'

if (-not $Readme.Contains('https://github.com/walle-2017/codex-usage-win/releases/latest')) {
    throw 'README.md must point to the renamed repository release page.'
}
if (-not $ReadmeZh.Contains('https://github.com/walle-2017/codex-usage-win/releases/latest')) {
    throw 'README.zh-CN.md must point to the renamed repository release page.'
}
if (-not $Maintenance.Contains('## Settings schema') -or
    -not $Maintenance.Contains('current schema') -or
    -not $Maintenance.Contains('remove unknown fields')) {
    throw 'Maintenance docs must describe current-schema settings repair without per-version migrations.'
}
if (-not $Maintenance.Contains('github-update') -or -not $Maintenance.Contains('--features store')) {
    throw 'Maintenance docs must describe standalone and Store update channels.'
}
if (-not $Readme.Contains('vCURRENT --> vLATEST') -or -not $ReadmeZh.Contains('v当前版本 --> v最新版本')) {
    throw 'Primary READMEs must document the discovered version menu hint.'
}
if (-not $Joined.Contains('--diagnose') -or -not $Troubleshooting.Contains('codex-usage-win.log.1')) {
    throw 'User-facing docs must document opt-in diagnostics and rotation.'
}

# Documentation reset: retain an empty changelog file but do not expose obsolete notes.
if (-not (Test-Path -LiteralPath 'CHANGELOG.md' -PathType Leaf) -or
    (Get-Item -LiteralPath 'CHANGELOG.md').Length -ne 0) {
    throw 'CHANGELOG.md must exist and remain empty after the audit reset.'
}
if (Test-Path -LiteralPath 'CONTRIBUTING.md' -PathType Leaf) {
    throw 'The retired CONTRIBUTING.md document must not be present.'
}
if ($Readme.Contains('(CHANGELOG.md)') -or $ReadmeZh.Contains('(CHANGELOG.md)')) {
    throw 'README links must not direct users to the empty changelog.'
}
if ($Readme -notmatch 'resvg' -or $ReadmeZh -notmatch 'resvg' -or
    $Readme -match 'Direct2D anti-aliased vector icons' -or
    $ReadmeZh -match 'Direct2D 抗锯齿矢量图标') {
    throw 'READMEs must accurately document resvg-based embedded SVG rasterization.'
}
$Security = Get-Content -Raw -LiteralPath 'SECURITY.md'
if ($Security.Contains('preserved in '+[char]96+'CHANGELOG.md')) {
    throw 'Security policy must not refer to notes in the cleared changelog.'
}

Write-Output 'PASS: documentation matches the cleaned Codex Usage Win repository and Store/GitHub channels.'
exit 0
