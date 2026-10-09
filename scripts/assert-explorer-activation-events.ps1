$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($term in @(
    'CODEX_TASKBAR_EXPLORER_EVENTS_EXPERIMENT',
    'explorer_activation_events_enabled',
    'SetWinEventHook',
    'EVENT_SYSTEM_FOREGROUND_DIAGNOSTIC',
    'EVENT_SYSTEM_MENUSTART_DIAGNOSTIC',
    'EVENT_SYSTEM_MENUEND_DIAGNOSTIC',
    'EVENT_OBJECT_FOCUS_DIAGNOSTIC',
    'on_explorer_activation_event',
    'log_explorer_activation_snapshot',
    'GetGUIThreadInfo',
    'native_interop::unhook_win_event'
)) {
    if ($source -notmatch [regex]::Escape($term)) {
        throw "Missing Explorer event diagnostic: $term"
    }
}
$callback = [regex]::Match($source, '(?s)unsafe\s+extern\s+"system"\s+fn\s+on_explorer_activation_event\s*\(.*?\n\}')
if (-not $callback.Success) { throw 'Missing Explorer activation callback.' }
foreach ($forbidden in @('SetForegroundWindow\s*\(', 'SetParent\s*\(', 'AttachThreadInput\s*\(', 'SendInput\s*\(')) {
    if ($callback.Value -match $forbidden) {
        throw "Explorer WinEvent callback must remain read-only: $forbidden"
    }
}
if ($source -notmatch 'for hook in explorer_activation_hooks\s*\{\s*native_interop::unhook_win_event') {
    throw 'Explorer event hooks must be unregistered after the message loop.'
}
if ($source -notmatch 'diagnose::is_enabled\(\)\s*&&\s*std::env::var_os\("CODEX_TASKBAR_EXPLORER_EVENTS_EXPERIMENT"\)') {
    throw 'Explorer event tracing must require --diagnose and opt-in.'
}
Write-Host 'PASS: Explorer activation event tracing is opt-in and read-only.'
