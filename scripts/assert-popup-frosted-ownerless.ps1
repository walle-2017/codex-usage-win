Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$win = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')

foreach ($token in @(
    'CODEX_TASKBAR_FROSTED_OWNERLESS_EXPERIMENT',
    'fn popup_ownerless_frosted_experiment_enabled()',
    'popup_session && popup_ownerless_frosted_experiment_enabled()',
    'clear_popup_owner_if_present(backdrop_hwnd)',
    'clear_popup_owner_if_present(foreground_hwnd)',
    'let owner = if popup_ownerless_frosted_experiment_enabled()',
    '&& !popup_ownerless_frosted_experiment_enabled()'
)) {
    if (-not $win.Contains($token)) { throw "Missing ownerless frosted experiment contract: $token" }
}
if (-not $native.Contains('pub fn clear_popup_owner_if_present(hwnd: HWND) -> bool')) {
    throw 'Missing native popup owner clear helper.'
}
$predicate = [regex]::Match($win, '(?s)fn\s+popup_ownerless_frosted_experiment_enabled\(\).*?\n\}')
if (-not $predicate.Success -or
    -not $predicate.Value.Contains('diagnose::is_enabled()') -or
    -not $predicate.Value.Contains('taskbar_replica_enabled()')) {
    throw 'Ownerless experiment must be opt-in and constrained to diagnosis and replica mode.'
}
$binding = [regex]::Match($win, '(?s)fn\s+bind_popup_windows_to_taskbar_owner\(.*?\n\}')
if (-not $binding.Success -or
    $binding.Value.IndexOf('clear_popup_owner_if_present') -gt
    $binding.Value.IndexOf('native_interop::set_popup_owner(backdrop_hwnd')) -or
    $binding.Value -notmatch '(?s)popup_session && popup_ownerless_frosted_experiment_enabled\(\).*?return;') {
    throw 'Ownerless branch must return before taskbar owner assignments.'
}
$nativeClear = [regex]::Match($native, '(?s)pub fn clear_popup_owner_if_present\(.*?\n\}')
if (-not $nativeClear.Success) { throw 'Missing clear function body.' }
foreach ($forbidden in @('SetParent\s*\(', 'SetWindowPos\s*\(', 'SetForegroundWindow\s*\(', 'SendInput\s*\(')) {
    if ($nativeClear.Value -match $forbidden) {
        throw "Ownerless clear helper must not change parenting, position or activation: $forbidden"
    }
}
if ($nativeClear.Value -notmatch 'SetWindowLongPtrW\(hwnd, GWLP_HWNDPARENT, 0\)' -or
    $nativeClear.Value -notmatch 'before_raw == 0 && before_effective == HWND::default\(\)') {
    throw 'Ownerless clear must only remove an existing top-level owner.'
}
Write-Host 'PASS: frosted ownerless experiment retains existing taskbar behavior by default.'
