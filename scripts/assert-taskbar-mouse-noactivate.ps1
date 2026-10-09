$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')

$required = @(
    'CODEX_TASKBAR_MOUSE_NOACTIVATE_EXPERIMENT',
    'mouse_noactivate_experiment_enabled',
    'WM_MOUSEACTIVATE',
    'MA_NOACTIVATE',
    'cursor_is_on_drag_handle',
    'WM_ACTIVATEAPP',
    'WM_NCACTIVATE'
)
foreach ($term in $required) {
    if ($source -notmatch [regex]::Escape($term)) {
        throw "Missing mouse-noactivate experiment contract: $term"
    }
}
$handler = [regex]::Match($source, '(?s)WM_MOUSEACTIVATE\s*=>\s*\{(?<body>.*?)\n\s*WM_LBUTTONDOWN\s*=>')
if (-not $handler.Success) {
    throw 'Unable to locate WM_MOUSEACTIVATE handler before WM_LBUTTONDOWN.'
}
$body = $handler.Groups['body'].Value
if ($body -notmatch 'enabled\s*&&\s*embedded\s*&&\s*drag_handle') {
    throw 'MA_NOACTIVATE must remain limited to enabled embedded drag-handle clicks.'
}
if ($body -notmatch 'left_button' -or $body -notmatch 'DefWindowProcW') {
    throw 'Non-left-button or disabled behavior must remain unchanged.'
}
if ($source -notmatch 'diagnose::is_enabled\(\)' -or $source -notmatch 'mouse_noactivate_experiment_enabled\(\)') {
    throw 'Mouse activation experiment must be opt-in and diagnostics-scoped.'
}
Write-Host 'PASS: taskbar mouse-noactivate experiment contract.'
