Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Contains {
    param([string]$Text, [string]$Needle, [string]$Message)
    if (-not $Text.Contains($Needle)) { throw $Message }
}

function Assert-NotContains {
    param([string]$Text, [string]$Needle, [string]$Message)
    if ($Text.Contains($Needle)) { throw $Message }
}

$ReleaseWorkflow = Get-Content -Raw -LiteralPath '.github/workflows/release.yml'
$Updater = Get-Content -Raw -LiteralPath 'src/updater.rs'
$Window = Get-Content -Raw -LiteralPath 'src/window.rs'

# Future releases publish only the current Codex Usage Win asset names.
Assert-NotContains $ReleaseWorkflow 'dist/codex-usage.exe' 'Release workflow must not publish the legacy codex-usage.exe alias.'
Assert-NotContains $ReleaseWorkflow 'dist/codex-usage.exe.sha256' 'Release workflow must not publish the legacy codex-usage.exe.sha256 alias.'
Assert-Contains $ReleaseWorkflow 'dist/codex-usage-win.exe' 'Release workflow must publish codex-usage-win.exe.'
Assert-Contains $ReleaseWorkflow 'dist/codex-usage-win.exe.sha256' 'Release workflow must publish codex-usage-win.exe.sha256.'

# Release asset 404 is a recoverable download failure.
Assert-Contains $Updater 'ManualUpdateRequired { version: String }' 'Updater must expose a manual-update result with the discovered version.'
Assert-Contains $Updater 'ureq::Error::Status(404, _)' 'Updater must classify release asset HTTP 404 separately.'
Assert-Contains $Updater 'UpdateOutcome::ManualUpdateRequired' 'Updater must convert a missing release asset into manual-update flow.'

# Missing asset handling must provide a link to GitHub Releases.
Assert-Contains $Window 'const IDM_OPEN_RELEASES' 'Context menu must define a GitHub Releases command.'
Assert-Contains $Window 'https://github.com/walle-2017/codex-usage-win/releases' 'Context menu must target the repository Releases page.'
Assert-Contains $Window 'ShellExecuteW' 'GitHub Releases command must open in the default browser.'
Assert-Contains $Window 'ManualUpdateRequired { version }' 'UI must handle the manual-update result explicitly.'
Assert-Contains $Window 'updater::UpdateError::AssetMissing' 'UI error matching must remain exhaustive for the internal missing-asset error.'
Assert-Contains $Window '对应的发布文件不存在' 'Simplified Chinese notification must explain missing release assets.'
Assert-Contains $Window 'GitHub Releases' 'Context menu must expose GitHub Releases.'

Write-Output 'PASS: release asset errors fall back to manual GitHub Releases update.'
exit 0
