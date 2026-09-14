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

$Cargo = Get-Content -Raw -LiteralPath 'Cargo.toml'
$Lock = Get-Content -Raw -LiteralPath 'Cargo.lock'
$Updater = Get-Content -Raw -LiteralPath 'src/updater.rs'
$Install = Get-Content -Raw -LiteralPath 'scripts/install.ps1'
$Uninstall = Get-Content -Raw -LiteralPath 'scripts/uninstall.ps1'
$ReleaseWorkflow = Get-Content -Raw -LiteralPath '.github/workflows/release.yml'
$CiBuild = Get-Content -Raw -LiteralPath '.github/workflows/ci.yml'

Assert-Contains $Cargo 'name = "codex-usage-win"' 'Cargo package must be named codex-usage-win.'
Assert-Contains $Cargo 'version = "1.0.5"' 'Branding release must be version 1.0.5.'
Assert-Contains $Cargo 'description = "Codex Usage Win"' 'Cargo description must use Codex Usage Win.'
Assert-Contains $Cargo 'ProductName = "Codex Usage Win"' 'Windows ProductName must use Codex Usage Win.'
Assert-Contains $Cargo 'FileDescription = "Codex Usage Win"' 'Windows FileDescription must use Codex Usage Win.'
Assert-Contains $Cargo 'OriginalFilename = "codex-usage-win.exe"' 'Windows OriginalFilename must use codex-usage-win.exe.'
Assert-Contains $Cargo 'InternalName = "CodexUsageWin"' 'Windows InternalName must use CodexUsageWin.'
Assert-Contains $Lock 'name = "codex-usage-win"' 'Cargo.lock root package must use codex-usage-win.'
Assert-Contains $Lock 'version = "1.0.5"' 'Cargo.lock root package must be version 1.0.5.'

Assert-Contains $Updater 'const EXE_ASSET_NAME: &str = "codex-usage-win.exe";' 'Updater executable asset must use codex-usage-win.exe.'
Assert-Contains $Updater 'const CHECKSUM_ASSET_NAME: &str = "codex-usage-win.exe.sha256";' 'Updater checksum asset must use codex-usage-win.exe.sha256.'
Assert-Contains $Updater '--codex-usage-win-updated-to=' 'Updater must emit the new one-shot success argument.'
Assert-Contains $Updater '--codex-usage-updated-to=' 'Updater must still accept the v1.0.3 legacy success argument during migration.'

Assert-Contains $Install "Programs\CodexUsageWin" 'Installer must use the CodexUsageWin install directory.'
Assert-Contains $Install "codex-usage-win.exe" 'Installer must install codex-usage-win.exe.'
Assert-Contains $Install "Codex Usage Win.lnk" 'Installer shortcuts must use Codex Usage Win.'
Assert-Contains $Install "Uninstall\CodexUsageWin" 'Installer uninstall registration must use CodexUsageWin.'
Assert-Contains $Install "DisplayName -Value 'Codex Usage Win'" 'Installed app display name must be Codex Usage Win.'
Assert-Contains $Install "Shortcut.Description = 'Codex Usage Win'" 'Shortcut description must be Codex Usage Win.'
Assert-Contains $Install 'CodexUsageWin-Installer' 'Installer User-Agent must use the new identity.'

Assert-Contains $Uninstall "Programs\CodexUsageWin" 'Uninstaller must target the CodexUsageWin install directory.'
Assert-Contains $Uninstall "codex-usage-win.exe" 'Uninstaller must target codex-usage-win.exe.'
Assert-Contains $Uninstall "Codex Usage Win.lnk" 'Uninstaller shortcuts must use Codex Usage Win.'
Assert-Contains $Uninstall "Uninstall\CodexUsageWin" 'Uninstaller registry key must use CodexUsageWin.'
Assert-Contains $Uninstall "Write-Output 'Codex Usage Win was uninstalled.'" 'Uninstaller message must use Codex Usage Win.'
Assert-Contains $Uninstall '$SettingsDirectory = Join-Path $env:APPDATA ''CodexUsage''' 'Existing settings directory must remain CodexUsage for upgrade compatibility.'

Assert-Contains $ReleaseWorkflow 'RELEASE_ASSET_NAME: codex-usage-win.exe' 'Release workflow asset name must be codex-usage-win.exe.'
Assert-Contains $ReleaseWorkflow 'target/release/codex-usage-win.exe' 'Release workflow must package the renamed binary.'
Assert-Contains $ReleaseWorkflow 'dist/codex-usage-win.exe.sha256' 'Release workflow must publish the renamed checksum.'
Assert-Contains $CiBuild 'target/release/codex-usage-win.exe' 'CI build must package the renamed binary.'
Assert-Contains $CiBuild 'name: codex-usage-win-windows-x64' 'CI build artifact must use the new file identity.'

$LocalizationFiles = Get-ChildItem -LiteralPath 'src/localization' -Filter '*.rs' | Where-Object { $_.Name -ne 'mod.rs' }
foreach ($File in $LocalizationFiles) {
    $Text = Get-Content -Raw -LiteralPath $File.FullName
    Assert-Contains $Text 'window_title: "Codex Usage Win"' "$($File.Name) window title must use Codex Usage Win."
    Assert-Contains $Text 'update_title: "Codex Usage Win' "$($File.Name) update notification title must start with Codex Usage Win."
}

foreach ($Path in @('Cargo.toml', 'scripts/install.ps1', 'scripts/uninstall.ps1')) {
    $Text = Get-Content -Raw -LiteralPath $Path
    Assert-NotContains $Text 'Codex Usage.lnk' "$Path still contains the old shortcut name."
}

if (-not (Test-Path -LiteralPath 'src/icons/icon.ico' -PathType Leaf)) {
    throw 'src/icons/icon.ico is required.'
}
if ((Get-Item -LiteralPath 'src/icons/icon.ico').Length -lt 1024) {
    throw 'src/icons/icon.ico is unexpectedly small.'
}

Write-Output 'Branding contract verified: Codex Usage Win / codex-usage-win / v1.0.5.'
exit 0
