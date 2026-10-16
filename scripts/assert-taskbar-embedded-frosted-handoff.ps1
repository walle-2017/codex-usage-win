Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$start = $source.IndexOf('fn try_taskbar_replica_handoff(')
$end = $source.IndexOf('fn attach_to_taskbar_window(', $start)
if ($start -lt 0 -or $end -le $start) { throw 'Replica handoff not found.' }
$handoff = $source.Substring($start, $end - $start)

foreach ($term in @(
    'let embedded_blur_handoff = frosted && embedded_frosted_experiment_enabled()',
    's.composition_blur_active',
    'if frosted && !embedded_blur_handoff',
    'taskbar embedded frosted replica handoff begin',
    'render_layered();',
    'sync_blur_backdrop_zorder(next);',
    'GetAncestor(blur, GA_PARENT) == taskbar.hwnd',
    'IsWindowVisible(blur).as_bool()',
    'let shown = after_render_ok && backdrop_ready',
    'taskbar embedded frosted replica backdrop ready=',
    'taskbar embedded frosted replica rollback: original backdrop restored',
    'sync_blur_backdrop_zorder(source_hwnd);',
    'taskbar embedded frosted replica handoff success'
)) {
    if (-not $handoff.Contains($term)) { throw "Missing embedded-frosted handoff guard: $term" }
}
foreach ($forbidden in @('SetParent\s*\(', 'AttachThreadInput\s*\(', 'SetForegroundWindow\s*\(')) {
    if ($handoff -match $forbidden) {
        throw "Existing frosted HWND must not be reparented or activated: $forbidden"
    }
}
$ensure = [regex]::Match($source, '(?s)fn\s+ensure_blur_backdrop\(.*?\n\}')
if (-not $ensure.Success -or
    -not $ensure.Value.Contains('if parent != target') -or
    -not $ensure.Value.Contains('destroy_blur_backdrop();')) {
    throw 'Blur backdrop must be recreated under the new taskbar rather than reparented.'
}
if ($handoff.IndexOf('s.hwnd = SendHwnd::from_hwnd(source_hwnd)') -gt
    $handoff.IndexOf('sync_blur_backdrop_zorder(source_hwnd);')) {
    throw 'Rollback must restore source state before rebuilding its backdrop.'
}
Write-Host 'PASS: embedded frosted replica moves via fixed taskbar children with guarded backdrop recreation.'
