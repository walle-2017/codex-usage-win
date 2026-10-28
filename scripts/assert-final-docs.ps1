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

Write-Output 'PASS: documentation matches the cleaned Codex Usage Win repository and Store/GitHub channels.'
exit 0
