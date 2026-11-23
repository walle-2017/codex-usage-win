$ErrorActionPreference = 'Stop'
$root = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$tests = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\tests.rs')
if ($root -notmatch '(?m)^#\[cfg\(test\)\]\s*\nmod tests;' -or
    $root -match '(?m)^mod tests \{' -or
    $tests -notmatch 'fn quick_appearance_menu_tracks_explicit_user_choices\(' -or
    $tests -notmatch 'fn low_quota_alert_is_deduplicated_until_reset_window_changes\(' -or
    ([regex]::Matches($tests, '#\[test\]')).Count -lt 70) {
    throw 'Window regression tests must remain in src/window/tests.rs and preserve existing coverage.'
}
Write-Output 'PASS: window event engine and isolated regression tests retain their module boundaries.'
