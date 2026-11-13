$ErrorActionPreference = 'Stop'

$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
# Desktop Default->Minimal->Default must recompute reset labels when the
# effective popup preset changes, not only at the earlier settings-message step.
$floatingWidgetBlock = [regex]::Match(
    $window,
    '(?s)fn enable_floating_widget\(reason: &str, vertical: bool\)\s*\{.*?\n\}'
).Value
if ([string]::IsNullOrWhiteSpace($floatingWidgetBlock) -or
    [regex]::Matches($floatingWidgetBlock, 'sync_floating_preset_and_usage\(s, preset\)').Count -ne 2 -or
    $window -notmatch '(?s)fn sync_floating_preset_and_usage\(s: &mut AppState, preset: AppearancePreset\)\s*\{\s*if s\.effective_appearance_preset != preset\s*\{\s*s\.effective_appearance_preset = preset;\s*refresh_usage_texts\(s\);') {
    throw 'Desktop layout changes must regenerate usage reset timestamps after the effective preset changes.'
}
$style = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style.rs')
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
$composition = Get-Content -Raw (Join-Path $PSScriptRoot '..\native\composition_blur.cpp')
$controlPrimitives = Get-Content -Raw (Join-Path $PSScriptRoot '..\native\control_primitives.cpp')
$styleWindow = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window.rs')
$styleWindowRoot = $styleWindow
$navigationState = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\navigation_state.rs')
$styleWindow += [Environment]::NewLine + $navigationState
if ($styleWindowRoot -notmatch 'mod navigation_state;' -or
    $styleWindowRoot -notmatch 'use self::navigation_state::\*;' -or
    $navigationState -notmatch 'pub\(super\) fn set_section\(' -or
    $navigationState -notmatch 'pub\(super\) fn select_editor\(' -or
    $navigationState -notmatch 'request_discard_confirmation\(hwnd, PendingDiscardAction::SwitchSection\(section\)\)' -or
    $navigationState -notmatch 'sync_settings_window_composition\(hwnd, section\)' -or
    $navigationState -notmatch 'layout_settings_children\(hwnd\)' -or
    $navigationState -notmatch 'sync_numeric_edits\(' -or
    $navigationState -notmatch 'redraw_settings_window\(hwnd\)' -or
    $styleWindowRoot -match '(?m)^fn (?:set_section|select_editor)\(') {
    throw 'Settings navigation and editor selection must remain in navigation_state.rs.'
}
$interactiveFrames = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\interactive_frames.rs')
$styleWindow += [Environment]::NewLine + $interactiveFrames
if ($styleWindowRoot -notmatch 'mod interactive_frames;' -or
    $styleWindowRoot -notmatch 'use self::interactive_frames::\*;' -or
    $interactiveFrames -notmatch 'pub\(super\) fn mark_settings_paint_cause\(' -or
    $interactiveFrames -notmatch 'pub\(super\) fn settings_interactive_frame_due\(' -or
    $interactiveFrames -notmatch 'pub\(super\) fn queue_settings_interactive_frame\(' -or
    $interactiveFrames -notmatch 'SETTINGS_INTERACTIVE_FRAME_QUEUED' -or
    $interactiveFrames -notmatch 'SETTINGS_LAST_INTERACTIVE_FRAME' -or
    $interactiveFrames -notmatch 'PostMessageW\(' -or
    $interactiveFrames -notmatch 'Duration::from_millis\(16\)' -or
    $styleWindowRoot -match '(?m)^fn (?:mark_settings_paint_cause|settings_interactive_frame_due|queue_settings_interactive_frame)\(') {
    throw 'Settings interactive-frame scheduling must remain in interactive_frames.rs.'
}
# WinMM tick source is separated from UI-thread message dispatch.
$submenuFrameClock = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\submenu_frame_clock.rs')
$styleWindow += [Environment]::NewLine + $submenuFrameClock
if ($styleWindowRoot -notmatch 'mod submenu_frame_clock;' -or
    $styleWindowRoot -notmatch 'use self::submenu_frame_clock::\*;' -or
    $submenuFrameClock -notmatch 'pub\(super\) fn begin_submenu_frame_clock\(' -or
    $submenuFrameClock -notmatch 'pub\(super\) fn end_submenu_frame_clock\(' -or
    $submenuFrameClock -notmatch 'pub\(super\) unsafe fn drive_submenu_frame_tick\(' -or
    $submenuFrameClock -notmatch 'timeSetEvent\(' -or
    $submenuFrameClock -notmatch 'TIME_KILL_SYNCHRONOUS_FLAG' -or
    $submenuFrameClock -notmatch 'PostMessageW\(' -or
    $submenuFrameClock -notmatch 'timeKillEvent\(' -or
    $submenuFrameClock -notmatch 'SetTimer\(' -or
    $styleWindowRoot -match '(?m)^(?:unsafe )?fn (?:begin_submenu_frame_clock|end_submenu_frame_clock|drive_submenu_frame_tick)\(') {
    throw 'Submenu multimedia frame clock and fallback must remain in submenu_frame_clock.rs.'
}
$windowChrome = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\window_chrome.rs')
$styleWindow += [Environment]::NewLine + $windowChrome
if ($styleWindowRoot -notmatch 'mod window_chrome;' -or
    $styleWindowRoot -notmatch 'use self::window_chrome::\*;' -or
    $windowChrome -notmatch 'pub\(super\) fn set_settings_initial_dwm_cloak\(' -or
    $windowChrome -notmatch 'pub\(super\) fn apply_fixed_titlebar\(' -or
    $windowChrome -notmatch 'pub\(super\) fn settings_window_ex_style\(' -or
    $windowChrome -notmatch 'pub\(super\) fn settings_composition_for_section\(' -or
    $windowChrome -notmatch 'pub\(super\) fn sync_settings_window_composition\(') {
    throw 'Settings window chrome must remain isolated in window_chrome.rs.'
}
# Wheel and slider actions retain native EDIT updates, model synchronization and frame throttling.
$sliderInteraction = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\slider_interaction.rs')
$styleWindow += [Environment]::NewLine + $sliderInteraction
if ($styleWindowRoot -notmatch 'mod slider_interaction;' -or
    $styleWindowRoot -notmatch 'use self::slider_interaction::\*;' -or
    $sliderInteraction -notmatch 'pub\(super\) fn adjust_numeric_by_wheel\(' -or
    $sliderInteraction -notmatch 'pub\(super\) fn adjust_slider_by_wheel\(' -or
    $sliderInteraction -notmatch 'pub\(super\) fn update_slider\(' -or
    $sliderInteraction -notmatch 'queue_settings_interactive_frame\(hwnd, true\)' -or
    $sliderInteraction -notmatch 'send_parent\(WM_STYLE_SAVE, 0, 0\)' -or
    $styleWindowRoot -match '(?m)^fn (?:adjust_numeric_by_wheel|adjust_slider_by_wheel|update_slider)\(') {
    throw 'Numeric wheel and slider interactions must retain their native controls and 60 FPS frame schedule.'
}
# Native EDIT change handlers are separate from value synchronization.
# In particular a focused blur edit must be redrawn without rewriting its text.
$editorInput = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\editor_input.rs')
$styleWindow += [Environment]::NewLine + $editorInput
if ($styleWindowRoot -notmatch 'mod editor_input;' -or
    $styleWindowRoot -notmatch 'use self::editor_input::\*;' -or
    $editorInput -notmatch 'pub\(super\) fn update_color_from_hex_edit\(' -or
    $editorInput -notmatch 'pub\(super\) fn update_color_from_numeric_edit\(' -or
    $editorInput -notmatch 'pub\(super\) fn update_blur_from_numeric_edit\(' -or
    $editorInput -notmatch 'pub\(super\) fn update_corner_from_numeric_edit\(' -or
    $editorInput -notmatch 'queue_settings_interactive_frame\(hwnd, false\)' -or
    $editorInput -notmatch 'send_parent\(WM_STYLE_SAVE, 0, 0\)' -or
    $styleWindowRoot -match '(?m)^fn (?:update_color_from_hex_edit|update_color_from_numeric_edit|update_blur_from_numeric_edit|update_corner_from_numeric_edit)\(') {
    throw 'Native input handlers must stay in editor_input.rs without changing preview or focus behavior.'
}
# Desktop submenu's cached endpoint frames retain their GDI-backed blending and terminal refresh.
$submenuComposition = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\submenu_composition.rs')
$styleWindow += [Environment]::NewLine + $submenuComposition
if ($styleWindowRoot -notmatch 'mod submenu_composition;' -or
    $styleWindowRoot -notmatch 'use self::submenu_composition::\*;' -or
    $submenuComposition -notmatch 'pub\(super\) (?:unsafe )?fn paint_submenu_endpoint\(' -or
    $submenuComposition -notmatch 'pub\(super\) (?:unsafe )?fn composite_cached_submenu_card\(' -or
    $submenuComposition -notmatch 'pub\(super\) (?:unsafe )?fn finish_submenu_terminal_paint\(' -or
    $submenuComposition -notmatch 'pub\(super\) (?:unsafe )?fn can_reuse_coalesced_position_paint\(' -or
    $submenuComposition -notmatch 'native_interop::alpha_blend_cached_card\(' -or
    $submenuComposition -notmatch 'PostMessageW\(' -or
    $submenuComposition -notmatch 'painted_section == Some\(section\)' -or
    $styleWindowRoot -match '(?m)^(?:unsafe\s+)?fn (?:paint_submenu_endpoint|composite_cached_submenu_card|finish_submenu_terminal_paint|can_reuse_coalesced_position_paint)\(') {
    throw 'Cached submenu endpoint composition and safe retained-pixel reuse must remain in submenu_composition.rs.'
}
# Preset gallery caches pre-render both positions and keep deferred warmup outside live frame paints.
$presetGalleryCache = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\preset_gallery_cache.rs')
$styleWindow += [Environment]::NewLine + $presetGalleryCache
if ($styleWindowRoot -notmatch 'mod preset_gallery_cache;' -or
    $styleWindowRoot -notmatch 'use self::preset_gallery_cache::\*;' -or
    $presetGalleryCache -notmatch 'pub\(super\) (?:unsafe )?fn preset_gallery_cache_ready\(' -or
    $presetGalleryCache -notmatch 'pub\(super\) (?:unsafe )?fn build_preset_gallery_cache\(' -or
    $presetGalleryCache -notmatch 'pub\(super\) (?:unsafe )?fn request_preset_gallery_warmup\(' -or
    $presetGalleryCache -notmatch 'pub\(super\) (?:unsafe )?fn copy_cached_preset_gallery\(' -or
    $presetGalleryCache -notmatch 'PostMessageW\(hwnd, WM_SETTINGS_PREWARM_GALLERY' -or
    $presetGalleryCache -notmatch 'SettingsPaintBuffer::create\(' -or
    $styleWindowRoot -match '(?m)^(?:unsafe\s+)?fn (?:preset_gallery_cache_ready|build_preset_gallery_cache|request_preset_gallery_warmup|copy_cached_preset_gallery)\(') {
    throw 'Deferred preset gallery warmup and cached position rendering must remain in preset_gallery_cache.rs.'
}
# First settings show is DWM-cloaked until a synchronous parent/child paint,
# with an opaque erase only if no retained first frame exists yet.
$firstSettingsShow = [regex]::Match(
    $styleWindowRoot,
    '(?s)let first_frame_cloaked = set_settings_initial_dwm_cloak\(hwnd, true\);.*?let _ = SetForegroundWindow\(hwnd\);'
)
if (-not $firstSettingsShow.Success -or
    $firstSettingsShow.Value -notmatch 'ShowWindow\(hwnd, SW_SHOWNORMAL\)' -or
    $firstSettingsShow.Value -notmatch 'RDW_INVALIDATE \| RDW_ERASE \| RDW_ALLCHILDREN \| RDW_UPDATENOW' -or
    $firstSettingsShow.Value -notmatch 'set_settings_initial_dwm_cloak\(hwnd, false\)' -or
    $windowChrome -notmatch 'DWMWA_CLOAK' -or
    $styleWindowRoot -notmatch 'painted_section.get\(\).is_none\(\)' -or
    $styleWindowRoot -notmatch 'first complete settings frame must be rendered synchronously') {
    throw 'Settings must reveal only after an opaque, fully painted initial parent/child frame.'
}
# Closing the discard overlay must restore the RichEdit child before the file chooser.
if ($styleWindowRoot -notmatch '(?s)Some\(PendingDiscardAction::Json\(action\)\) => \{\s*//[^\n]*\n\s*//[^\n]*\n\s*//[^\n]*\n\s*layout_settings_children\(hwnd\);\s*redraw_settings_window\(hwnd\);\s*begin_json_action_confirmed\(hwnd, action\);' -or
    $styleWindowRoot -notmatch 'json_has_pending_changes\(s.json_dirty, s.json_import_pending_apply\)' -or
    $styleWindowRoot -notmatch 'json_import_pending_apply: false,') {
    throw 'Confirmed JSON replacement must restore RichEdit and preserve imported-pending state.'
}
# JSON file import must not inherit an old dirty warning, and Export is disabled until applied.
$jsonFileGuards = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_file_io.rs')
$jsonClickGuards = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\hit_testing.rs')
$jsonPaintGuards = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_paint.rs')
$jsonEditorGuards = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_editor_text.rs')
$jsonActionGuards = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_actions.rs')
$jsonSupportGuards = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_support.rs')
# Editing JSON invalidates the prior operation-success label and clickable
# import/export path immediately, but retains errors until live validation runs.
if ($jsonActionGuards -notmatch 'if json_success_status_is_stale_on_edit\(&s.json_status\) \{\s*s.json_status.clear\(\);\s*s.json_status_path = None;' -or
    $jsonSupportGuards -notmatch "status.starts_with\('✓'\)" -or
    $jsonSupportGuards -notmatch 'fn json_user_edit_clears_stale_success_statuses_but_not_errors\(') {
    throw 'JSON user edits must clear stale Reload, Format, Import and Export success statuses immediately.'
}
if ($jsonClickGuards -notmatch 'json_action_enabled\(action, json_dirty, json_import_pending_apply\)' -or
    $jsonPaintGuards -notmatch 'disabled_export = action == JsonAction::Export' -or
    $jsonPaintGuards -notmatch '● 已导入，待应用' -or
    $jsonFileGuards -notmatch 'write_json_editor\(&standard, status, false\)' -or
    $jsonFileGuards -notmatch 's.json_import_pending_apply = import_pending_apply;' -or
    $jsonFileGuards -notmatch 'json_action_enabled\(JsonAction::Export, s.json_dirty, s.json_import_pending_apply\)' -or
    $jsonEditorGuards -notmatch 's.json_import_pending_apply = false;' -or
    $jsonActionGuards -notmatch 'request_discard_confirmation\(hwnd, PendingDiscardAction::Json\(action\)\)' -or
    $jsonActionGuards -notmatch 'begin_json_action_confirmed\(hwnd, action\)' -or
    $jsonSupportGuards -notmatch 'fn json_actions_guard_export_and_confirm_destructive_reload_or_import\(' -or
    $styleWindowRoot -notmatch 'Some\(PendingDiscardAction::Json\(action\)\)') {
    throw 'JSON import, unsaved confirmation, Export-disable and pending Apply regression.'
}
# Configuration file dialog and cancellation behavior must remain in json_file_io.rs.
$jsonFileIo = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_file_io.rs')
$styleWindow += [Environment]::NewLine + $jsonFileIo
if ($styleWindowRoot -notmatch 'mod json_file_io;' -or
    $styleWindowRoot -notmatch 'use self::json_file_io::\*;' -or
    $jsonFileIo -notmatch 'pub\(super\) fn file_dialog\(' -or
    $jsonFileIo -notmatch 'pub\(super\) fn import_json_file\(' -or
    $jsonFileIo -notmatch 'pub\(super\) fn export_json_file\(' -or
    $styleWindowRoot -match '(?m)^fn (?:file_dialog|import_json_file|export_json_file)\(') {
    throw 'JSON Windows file pickers and import/export must remain in json_file_io.rs.'
}
# JSON action state and validation logic is independently modularized.
$jsonActions = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_actions.rs')
$styleWindow += [Environment]::NewLine + $jsonActions
if ($styleWindowRoot -notmatch 'mod json_actions;' -or
    $styleWindowRoot -notmatch 'use self::json_actions::\*;' -or
    $jsonActions -notmatch 'pub\(super\) fn begin_json_apply\(' -or
    $jsonActions -notmatch 'pub\(super\) fn begin_json_action\(' -or
    $jsonActions -notmatch 'pub\(super\) fn finish_pending_json_action\(' -or
    $jsonActions -notmatch 'pub\(super\) fn schedule_json_validation\(' -or
    $jsonActions -notmatch 'pub\(super\) fn update_json_validation_status\(' -or
    $jsonActions -notmatch 'pub\(super\) fn format_json_editor\(' -or
    $jsonActions -notmatch 'pub\(super\) fn apply_json_editor\(' -or
    $styleWindowRoot -match '(?m)^fn (?:begin_json_apply|begin_json_action|finish_pending_json_action|schedule_json_validation|update_json_validation_status|format_json_editor|apply_json_editor)\(') {
    throw 'JSON action orchestration, validation and apply helpers must remain in json_actions.rs.'
}
# Native editor value synchronization stays separate from repaint and input handlers.
$editorSync = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\editor_sync.rs')
$styleWindow += "\`n" + $editorSync
if ($styleWindowRoot -notmatch 'mod editor_sync;' -or
    $styleWindowRoot -notmatch 'use self::editor_sync::\*;' -or
    $editorSync -notmatch 'pub\(super\) fn sync_numeric_edits\(' -or
    $editorSync -notmatch 'pub\(super\) fn sync_blur_edit\(' -or
    $editorSync -notmatch 'pub\(super\) fn sync_corner_edit\(' -or
    $editorSync -notmatch 'pub\(super\) fn sync_active_style_into_editable\(' -or
    $editorSync -notmatch 'pub\(super\) fn sync_hex_edits\(' -or
    $editorSync -notmatch 'pub\(super\) fn normalize_hex_edit\(' -or
    $editorSync -notmatch 'set_edit_text_string\(' -or
    $editorSync -notmatch 's.syncing_hex_edits = false;' -or
    $styleWindowRoot -match '(?m)^fn (?:sync_numeric_edits|sync_blur_edit|sync_corner_edit|sync_active_style_into_editable|sync_hex_edits|normalize_hex_edit)\(') {
    throw 'Native edit synchronization must preserve state locks and input-message order in editor_sync.rs.'
}
# Native editor layout now has its own module; retain focus-safe child operations.
$editorLayout = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\editor_layout.rs')
$styleWindow += "\`n" + $editorLayout
if ($styleWindowRoot -notmatch 'mod editor_layout;' -or
    $styleWindowRoot -notmatch 'use self::editor_layout::\*;' -or
    $editorLayout -notmatch 'pub\(super\) unsafe fn single_line_edit_font_height\(' -or
    $editorLayout -notmatch 'pub\(super\) fn editor_layout_snapshot\(' -or
    $editorLayout -notmatch 'pub\(super\) fn release_editor_focus_before_layout\(' -or
    $editorLayout -notmatch 'pub\(super\) fn layout_numeric_edits\(' -or
    $editorLayout -notmatch 'pub\(super\) fn hex_layout_snapshot\(' -or
    $editorLayout -notmatch 'pub\(super\) fn layout_hex_edits\(' -or
    $styleWindowRoot -match '(?m)^(?:unsafe\s+)?fn (?:single_line_edit_font_height|editor_layout_snapshot|release_editor_focus_before_layout|layout_numeric_edits|hex_layout_snapshot|layout_hex_edits)\(') {
    throw 'Editor child layout must remain centralized and preserve font/focus safety.'
}
# Region and text-measurement helpers must stay in the shared geometry modules.
$layoutWindowHelpers = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\layout_geometry.rs')
$jsonWindowHelpers = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_geometry.rs')
$renderWindowHelpers = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\render_primitives.rs')
if ($layoutWindowHelpers -notmatch 'pub\(super\) fn settings_work_area_for_widget\(' -or
    $layoutWindowHelpers -notmatch 'pub\(super\) fn position_is_only_snapshot_change\(' -or
    $jsonWindowHelpers -notmatch 'pub\(super\) fn json_status_path_rect\(' -or
    $renderWindowHelpers -notmatch 'pub\(super\) fn text_width_px\(' -or
    $styleWindowRoot -match '(?m)^fn (?:settings_work_area_for_widget|position_is_only_snapshot_change|json_status_path_rect|text_width_px)\(') {
    throw 'Settings geometry/text helpers must be available from extracted modules.'
}
# Input hit testing and JSON export name defaults stay available after splitting.
$inputHitHelpers = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\input_events.rs')
$hitTesting = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\hit_testing.rs')
$styleWindow += "`n" + $hitTesting
$jsonNameHelpers = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_support.rs')
if ($hitTesting -notmatch 'pub\(super\) fn wheel_numeric_target_at\(' -or
    $hitTesting -notmatch 'pub\(super\) fn hit_target_at\(' -or
    $hitTesting -notmatch 'pub\(super\) fn slider_kind_at\(' -or
    $styleWindowRoot -notmatch 'mod hit_testing;' -or
    $styleWindowRoot -notmatch 'use self::hit_testing::\*;' -or
    $styleWindowRoot -match '(?m)^fn (?:hit_target_at|slider_kind_at)\(' -or
    $inputHitHelpers -match '(?m)^pub\(super\) fn wheel_numeric_target_at\(' -or
    $jsonNameHelpers -notmatch 'pub\(super\) fn export_default_filename\(' -or
    $jsonNameHelpers -notmatch 'codex-usage-win-config-' -or
    $styleWindowRoot -match '(?m)^fn (?:wheel_numeric_target_at|export_default_filename)\(') {
    throw 'Wheel numeric hit targets and export file names must remain unchanged after extraction.'
}
# Native editor messaging now lives in a dedicated module; retain all six helpers.
$editControls = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\edit_controls.rs')
$styleWindow += "`n" + $editControls
if ($styleWindowRoot -notmatch 'mod edit_controls;' -or
    $styleWindowRoot -notmatch 'use self::edit_controls::\*;' -or
    $editControls -notmatch 'pub\(super\) fn set_edit_text_string\(' -or
    $editControls -notmatch 'pub\(super\) fn set_edit_text\(' -or
    $editControls -notmatch 'pub\(super\) fn read_edit_value\(' -or
    $editControls -notmatch 'pub\(super\) fn read_edit_text\(' -or
    $editControls -notmatch 'pub\(super\) unsafe fn rich_set_selection\(' -or
    $editControls -notmatch 'pub\(super\) unsafe fn rich_set_selected_color\(' -or
    $styleWindowRoot -match '(?m)^\s*(?:unsafe\s+)?fn (?:set_edit_text_string|set_edit_text|read_edit_value|read_edit_text|rich_set_selection|rich_set_selected_color)\(') {
    throw 'Settings native edit helpers must remain centralized after modularization.'
}
# Direct typing must explicitly present the Settings slider and native
# EDIT without rewriting the in-progress text or moving its caret.
$typedBlurHandler = [regex]::Match(
    $editorInput,
    '(?s)pub\(super\) fn update_blur_from_numeric_edit\(\)\s*\{.*?\n\}'
).Value
if ([string]::IsNullOrWhiteSpace($typedBlurHandler) -or
    $typedBlurHandler -notmatch 'RDW_INVALIDATE \| RDW_ERASE \| RDW_UPDATENOW' -or
    $typedBlurHandler -notmatch 'queue_settings_interactive_frame\(hwnd, false\)' -or
    $typedBlurHandler -match '(?m)^\s*sync_blur_edit\(' -or
    $styleWindowRoot -notmatch 'for \(section, typed_value\) in \[\(Section::Panel, 27\), \(Section::Tooltip, 43\)\]') {
    throw 'Typed panel/tooltip blur must present the native edit and slider without replacing focused input text.'
}
# A wheel step on a native EDIT must update both the model and the actual
# child window text, even when the settings parent uses WS_EX_COMPOSITED.
$wheelTextSync = [regex]::Match(
    $sliderInteraction,
    '(?s)pub\(super\) fn adjust_numeric_by_wheel\(.*?\n\}'
).Value
if ([string]::IsNullOrWhiteSpace($wheelTextSync) -or
    $wheelTextSync -notmatch 'set_edit_text\(edit, next\)' -or
    $wheelTextSync -notmatch 'WheelNumericTarget::Blur => return,' -or
    $wheelTextSync -notmatch 'WheelNumericTarget::Corner => sync_corner_edit\(\)' -or
    $wheelTextSync -notmatch 'WheelNumericTarget::Color\(_\) => sync_numeric_edits\(\)' -or
    $wheelTextSync -notmatch 'RDW_INVALIDATE \| RDW_ERASE \| RDW_UPDATENOW' -or
    $styleWindowRoot -notmatch 'focused native EDIT text must match the blur slider/model') {
    throw 'Numeric wheel adjustments must synchronize and repaint the focused native edit after updating style.'
}
$editorGeometry = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\editor_geometry.rs')
# Native single-line EDIT has no vertical-text alignment API. The child
# must be centered inside the themed input background using real font metrics.
if ($editorGeometry -notmatch 'pub\(super\) fn centered_single_line_edit_rect\(' -or
    $styleWindowRoot -match '(?m)^fn centered_single_line_edit_rect\(' -or
    $editorLayout -notmatch 'fn single_line_edit_font_height\(' -or
    $editorLayout -notmatch 'WM_GETFONT' -or
    $editorLayout -notmatch 'GetTextMetricsW\(' -or
    $editorLayout -notmatch 'centered_single_line_edit_rect\(\s*numeric_edit_rect\(' -or
    $editorLayout -notmatch 'centered_single_line_edit_rect\(\s*blur_edit_rect\(' -or
    $editorLayout -notmatch 'centered_single_line_edit_rect\(\s*corner_edit_rect\(' -or
    $editorLayout -notmatch 'centered_single_line_edit_rect\(\s*hex_edit_rect\(' -or
    $styleWindowRoot -notmatch 'fn single_line_edits_stay_vertically_centered_across_font_metrics_and_dpi\(') {
    throw 'Settings single-line edit controls must center the native text window using measured font metrics across all input types.'
}
$styleWindow += "`n" + (Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\preset_preview.rs'))
$jsonStatus = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_status.rs')
$styleWindow += "`n" + $jsonStatus
if ($styleWindowRoot -notmatch 'mod json_status;' -or
    $styleWindowRoot -notmatch 'use self::json_status::\*;' -or
    $jsonStatus -notmatch 'pub\(super\) fn set_json_status\(' -or
    $jsonStatus -notmatch 'pub\(super\) fn set_json_status_with_path\(' -or
    $jsonStatus -notmatch 'pub\(super\) fn open_json_status_path\(' -or
    $jsonStatus -notmatch 'pub\(super\) fn start_json_error_shake\(' -or
    $jsonStatus -notmatch 'pub\(super\) fn advance_json_error_shake\(' -or
    $jsonStatus -notmatch 'pub\(super\) fn show_json_apply_validation_error\(' -or
    $styleWindowRoot -match '(?m)^fn (?:set_json_status|set_json_status_with_path|open_json_status_path|start_json_error_shake|advance_json_error_shake|show_json_apply_validation_error)\(') {
    throw 'JSON status and validation feedback helpers must stay in json_status.rs.'
}
$jsonEditorText = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_editor_text.rs')
$styleWindow += "`n" + $jsonEditorText
if ($styleWindowRoot -notmatch 'mod json_editor_text;' -or
    $styleWindowRoot -notmatch 'use self::json_editor_text::\*;' -or
    $jsonEditorText -notmatch 'pub\(super\) fn read_large_edit_text_raw\(' -or
    $jsonEditorText -notmatch 'pub\(super\) fn read_large_edit_text\(' -or
    $jsonEditorText -notmatch 'pub\(super\) fn syntax_highlight_json_editor\(' -or
    $jsonEditorText -notmatch 'pub\(super\) fn write_json_editor\(' -or
    $jsonEditorText -notmatch 'pub\(super\) fn refresh_json_editor_theme\(' -or
    $jsonEditorText -notmatch 'pub\(super\) fn reload_json_editor_from_snapshot\(' -or
    $jsonEditorText -notmatch 'pub\(super\) fn locate_json_error\(' -or
    $styleWindowRoot -match '(?m)^fn (?:read_large_edit_text_raw|read_large_edit_text|syntax_highlight_json_editor|write_json_editor|refresh_json_editor_theme|reload_json_editor_from_snapshot|locate_json_error)\(') {
    throw 'JSON RichEdit text, syntax and editor refresh helpers must remain in json_editor_text.rs.'
}
$jsonSupport = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_support.rs')
$styleWindow += "`n" + $jsonSupport
$colorCodec = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\color_codec.rs')
$styleWindow += "`n" + $colorCodec
if ($styleWindowRoot -notmatch 'mod color_codec;' -or
    $styleWindowRoot -notmatch 'pub use self::color_codec::\{decode_color_target, unpack_color\};' -or
    $colorCodec -notmatch 'pub fn decode_color_target\(' -or
    $colorCodec -notmatch 'pub fn unpack_color\(' -or
    $colorCodec -notmatch 'pub\(super\) fn encode_color_target\(' -or
    $colorCodec -notmatch 'pub\(super\) fn pack_color\(' -or
    $colorCodec -notmatch 'u32::from\(color.a\) << 24' -or
    $styleWindowRoot -match '(?m)^\s*(?:pub )?fn (?:decode_color_target|unpack_color|encode_color_target|pack_color)\(' -or
    $styleWindowRoot -notmatch 'fn settings_color_codec_round_trips_all_targets_and_rgba_channels\(') {
    throw 'Color target IDs and RGBA Win32 packing must remain stable and exported after modularization.'
}
$jsonScroll = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_scroll.rs')
$styleWindow += "`n" + $jsonScroll
$inputEvents = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\input_events.rs')
$styleWindow += "`n" + $inputEvents
$commandEvents = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\command_events.rs')
$styleWindow += "`n" + $commandEvents
$timerEvents = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\timer_events.rs')
$styleWindow += "`n" + $timerEvents
$lifecycleEvents = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\lifecycle_events.rs')
$styleWindow += "`n" + $lifecycleEvents
$renderPrimitives = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\render_primitives.rs')
$styleWindow += "`n" + $renderPrimitives
# Semantic icon mapping follows the shared native icon painter.
if ($renderPrimitives -notmatch 'pub\(super\) fn theme_icon\(' -or
    $renderPrimitives -notmatch 'pub\(super\) fn layout_icon\(' -or
    $renderPrimitives -notmatch 'pub\(super\) fn preset_icon\(' -or
    $renderPrimitives -notmatch 'pub\(super\) fn json_action_icon\(' -or
    $styleWindowRoot -match '(?m)^\s*fn (?:theme_icon|layout_icon|preset_icon|json_action_icon)\(' -or
    $styleWindowRoot -notmatch 'fn settings_icons_preserve_theme_layout_preset_and_json_mappings\(') {
    throw 'Settings icon mappings must be centralized without changing any theme, preset or JSON button icon.'
}
$jsonGeometry = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_geometry.rs')
$styleWindow += "`n" + $jsonGeometry
$jsonPaint = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\json_paint.rs')
$styleWindow += "`n" + $jsonPaint
# JSON child pane and its narrow scrollbar repaint must share json_paint.rs.
# Keep the retained tiny bitmap, clipped scrollbar frame, full-page fallback
# and the existing Windows HWND regression that exercises the buffered painter.
if ($jsonPaint -notmatch 'pub\(super\) unsafe fn paint_json_scrollbar_buffered\(' -or
    $jsonPaint -notmatch 'pub\(super\) unsafe fn paint_json_page_surface\(' -or
    $jsonPaint -notmatch 'JSON_SCROLLBAR_PAINT_BUFFER.with\(' -or
    $jsonPaint -notmatch 'SettingsPaintBuffer::create\(' -or
    $jsonPaint -notmatch 'if scrollbar_only \{' -or
    $jsonPaint -notmatch 'if !buffered \{' -or
    $jsonPaint -notmatch 'SetViewportOrgEx\(buffer.dc, -track.left, -track.top' -or
    $jsonPaint -notmatch 'paint_json_scrollbar_decoration\(' -or
    $styleWindowRoot -match '(?m)^\s*unsafe fn (?:paint_json_scrollbar_buffered|paint_json_page_surface)\(' -or
    $styleWindowRoot -notmatch 'paint_json_page_surface\(hwnd\);' -or
    $styleWindowRoot -notmatch 'the narrow scrollbar must support double-buffered painting') {
    throw 'JSON child-pane rendering must keep buffered scrollbar and fallback semantics when moved to json_paint.rs.'
}
# Unsaved JSON overlay painting belongs with the JSON page rendering;
# click hit-testing and the dialog action remain in the parent window.
if ($jsonPaint -notmatch 'pub\(super\) unsafe fn paint_discard_dialog\(' -or
    $jsonPaint -notmatch 'discard_dialog_button_rect\(hwnd, true\)' -or
    $jsonPaint -notmatch 'discard_dialog_button_rect\(hwnd, false\)' -or
    $jsonPaint -notmatch 'HitTarget::DiscardChanges' -or
    $jsonPaint -notmatch 'HitTarget::KeepEditing' -or
    $jsonPaint -notmatch '未保存的更改' -or
    $jsonPaint -notmatch '#B65F63FF' -or
    $styleWindowRoot -match '(?m)^\s*unsafe fn paint_discard_dialog\(' -or
    $jsonGeometry -notmatch 'pub\(super\) fn discard_dialog_rect\(' -or
    $jsonGeometry -notmatch 'pub\(super\) fn discard_dialog_button_rect\(' -or
    $styleWindowRoot -match '(?m)^fn discard_dialog_rect\(' -or
    $styleWindowRoot -match '(?m)^fn discard_dialog_button_rect\(' -or
    $styleWindowRoot -notmatch 'fn unsaved_json_confirmation_buttons_keep_distinct_hit_rects\(') {
    throw 'Unsaved JSON confirmation must keep its original layout, copy and actions when moving painting to json_paint.rs.'
}
$presetGalleryPaint = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\preset_gallery_paint.rs')
$styleWindow += "`n" + $presetGalleryPaint
if ($presetGalleryPaint -notmatch 'pub\(super\) fn preset_gallery_card_rect\(' -or
    $presetGalleryPaint -notmatch 'pub\(super\) fn preview_vertical_for_snapshot\(' -or
    $presetGalleryPaint -notmatch 'pub\(super\) fn preview_layout_flags\(' -or
    $presetGalleryPaint -notmatch 'pub\(super\) fn preset_card_rect_for_orientation\(' -or
    $presetGalleryPaint -notmatch 'pub\(super\) fn preset_card_rect\(' -or
    $presetGalleryPaint -notmatch 'pub\(super\) const PRESET_CARD_TOP:' -or
    $presetGalleryPaint -notmatch 'PRESET_CARD_MINIMAL_HEIGHT: i32 = 250' -or
    $styleWindowRoot -match '(?m)^\s*fn (?:preset_gallery_card_rect|preview_layout_flags|preset_card_rect)\(' -or
    $styleWindowRoot -notmatch 'fn preset_gallery_retains_vertical_and_horizontal_card_sizes\(') {
    throw 'Preset card sizing and position must remain unchanged.'
}
$appearancePaint = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\appearance_paint.rs')
$styleWindow += "`n" + $appearancePaint
$editorPaint = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\editor_paint.rs')
$styleWindow += "`n" + $editorPaint
if ($editorPaint -notmatch 'pub\(super\) unsafe fn paint_frosted_alpha_hint\(' -or
    $editorPaint -notmatch 'pub\(super\) fn row_label\(' -or
    $editorPaint -notmatch 'CreateFontW\(' -or
    $editorPaint -notmatch 'DeleteObject\(hint_font\)' -or
    $styleWindowRoot -match '(?m)^\s*(?:unsafe )?fn (?:paint_frosted_alpha_hint|row_label)\(' -or
    $styleWindowRoot -notmatch 'fn editor_labels_preserve_localized_frosted_and_progress_values\(') {
    throw 'Editor label and alpha-hint rendering must retain original locale and GDI lifecycle.'
}
$generalPagePaint = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\general_page_paint.rs')
$styleWindow += "`n" + $generalPagePaint
$navigationPaint = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\navigation_paint.rs')
$styleWindow += "`n" + $navigationPaint
$layoutGeometry = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window\layout_geometry.rs')
$styleWindow += "`n" + $layoutGeometry
if ($layoutGeometry -notmatch 'pub\(super\) fn window_dpi\(' -or
    $layoutGeometry -notmatch 'pub\(super\) fn scale\(' -or
    $layoutGeometry -notmatch 'pub\(super\) fn rect\(' -or
    $layoutGeometry -notmatch 'pub\(super\) fn point_from_lparam\(' -or
    $layoutGeometry -notmatch 'pub\(super\) fn pt_in_rect\(' -or
    $layoutGeometry -notmatch 'GetDpiForWindow\(hwnd\).max\(96\)' -or
    $layoutGeometry -notmatch 'as i16 as i32' -or
    $styleWindowRoot -match '(?m)^\s*fn (?:window_dpi|scale|rect|point_from_lparam|pt_in_rect)\(' -or
    $styleWindowRoot -notmatch 'fn settings_coordinate_helpers_keep_signed_points_and_half_open_hits\(') {
    throw 'Shared DPI pixel geometry and signed pointer hit bounds must remain unchanged.'
}
$styleWindow += "`n" + $editorGeometry
if ($styleWindowRoot -notmatch 'mod editor_geometry;' -or
    $styleWindowRoot -notmatch 'use self::editor_geometry::\*;' -or
    $editorGeometry -notmatch 'pub\(super\) fn rows\(' -or
    $editorGeometry -notmatch 'pub\(super\) fn blur_row_index\(' -or
    $editorGeometry -notmatch 'pub\(super\) fn blur_slider_track_rect\(' -or
    $editorGeometry -notmatch 'pub\(super\) fn blur_edit_rect\(' -or
    $editorGeometry -notmatch 'pub\(super\) fn numeric_edit_frame_rect\(' -or
    $editorGeometry -notmatch 'client.bottom - scale\(hwnd, 24\)' -or
    $editorGeometry -notmatch 'right: frame.right - scale\(hwnd, 28\)' -or
    $styleWindowRoot -match '(?m)^\s*fn (?:rows|blur_row_index|blur_edit_rect)\(' -or
    $styleWindowRoot -notmatch 'fn editor_slider_hit_regions_keep_blur_rows_aligned\(') {
    throw 'Editor row/slider/blur geometry must preserve original paint and hit targets.'
}
if ($styleWindowRoot -notmatch 'mod layout_geometry;' -or
    $styleWindowRoot -notmatch 'use self::layout_geometry::\*;' -or
    $layoutGeometry -notmatch 'pub\(super\) fn general_segment_rect\(' -or
    $layoutGeometry -notmatch 'pub\(super\) fn section_rect\(' -or
    $layoutGeometry -notmatch 'Section::Tooltip => \(220, 260\)' -or
    $layoutGeometry -notmatch 'pub\(super\) fn desktop_auto_collapse_rect\(' -or
    $layoutGeometry -notmatch 'pub\(super\) fn appearance_button_rect\(' -or
    $styleWindowRoot -match '(?m)^\s*fn (?:general_segment_rect|section_rect|appearance_button_rect)\(' -or
    $styleWindowRoot -notmatch 'fn appearance_button_hit_areas_keep_joined_edges\(') {
    throw 'Sidebar/General/appearance paint and hit-test geometry must remain shared.'
}
if ($styleWindowRoot -notmatch 'mod navigation_paint;' -or
    $styleWindowRoot -notmatch 'use self::navigation_paint::\*;' -or
    $navigationPaint -notmatch 'pub\(super\) unsafe fn paint_navigation\(' -or
    $navigationPaint -notmatch 'pub\(super\) fn navigation_icon\(' -or
    $navigationPaint -notmatch 'pub\(super\) fn navigation_text_inset\(' -or
    $navigationPaint -notmatch 'pub\(super\) fn section_label\(' -or
    $navigationPaint -notmatch 'pub\(super\) fn appearance_page_title\(' -or
    $navigationPaint -notmatch 'navigation_icon\(item\)' -or
    $navigationPaint -notmatch 'section_label\(item, snapshot.language\)' -or
    $navigationPaint -notmatch 'navigation_text_inset\(item\)' -or
    $navigationPaint -notmatch 'Section::Tooltip' -or
    $styleWindowRoot -match '(?m)^\s*(?:unsafe )?fn (?:paint_navigation|navigation_icon|navigation_text_inset|section_label|appearance_page_title)\(' -or
    $styleWindowRoot -notmatch 'fn settings_navigation_icons_labels_and_hit_regions_are_consistent\(') {
    throw 'Navigation painting, icon mapping and labels must remain in navigation_paint.rs without changing sidebar hit geometry or localization.'
}
if ($styleWindowRoot -notmatch 'mod general_page_paint;' -or
    $styleWindowRoot -notmatch 'use self::general_page_paint::\*;' -or
    $generalPagePaint -notmatch 'pub\(super\) unsafe fn draw_general_segment_group\(' -or
    $generalPagePaint -notmatch 'pub\(super\) unsafe fn draw_general_usage_checkbox\(' -or
    $generalPagePaint -notmatch 'pub\(super\) unsafe fn paint_general_page\(' -or
    $generalPagePaint -notmatch 'HitTarget::Refresh\(60_000\)' -or
    $generalPagePaint -notmatch 'HitTarget::Refresh\(3_600_000\)' -or
    $generalPagePaint -notmatch 'HitTarget::Alert\(30\)' -or
    $generalPagePaint -notmatch 'HitTarget::UsageSession' -or
    $generalPagePaint -notmatch 'HitTarget::UsageWeekly' -or
    $generalPagePaint -notmatch 'HitTarget::Startup' -or
    $generalPagePaint -notmatch 'HitTarget::LanguageOption\(index\)' -or
    $generalPagePaint -notmatch 'draw_rounded_outline_rect\(hdc, bounds, separator, radius, 1\)' -or
    $generalPagePaint -notmatch 'draw_switch\(' -or
    $styleWindowRoot -match '(?m)^\s*unsafe fn (?:draw_general_segment_group|draw_general_usage_checkbox|paint_general_page)\(') {
    throw 'General-page painters must live in general_page_paint.rs with refresh, quota, usage, startup, language and border contracts intact.'
}
if ($styleWindowRoot -notmatch 'mod editor_paint;' -or
    $styleWindowRoot -notmatch 'use self::editor_paint::\*;' -or
    $editorPaint -notmatch 'pub\(super\) unsafe fn paint_hex_edit_frames\(' -or
    $editorPaint -notmatch 'pub\(super\) unsafe fn paint_numeric_edit_frames\(' -or
    $editorPaint -notmatch 'pub\(super\) unsafe fn paint_blur_edit_frame\(' -or
    $editorPaint -notmatch 'pub\(super\) unsafe fn paint_corner_edit_frame\(' -or
    $editorPaint -notmatch 'pub\(super\) unsafe fn paint_editor\(' -or
    $editorPaint -notmatch 'pub\(super\) unsafe fn draw_slider\(' -or
    $editorPaint -notmatch 'if invalid\[index\]' -or
    $editorPaint -notmatch 'focused == Some\(target\)' -or
    $editorPaint -notmatch 'focused == Some\(index\)' -or
    $editorPaint -notmatch 'draw_text\(\s*hdc,\s*"%"' -or
    $editorPaint -notmatch 'draw_text\(\s*hdc,\s*"px"' -or
    $editorPaint -notmatch 'Color channels' -or
    $editorPaint -notmatch 'fill_rounded_rect\(hdc, track' -or
    $editorPaint -notmatch 'fill_rounded_rect\(hdc, thumb, accent, radius\)' -or
    $styleWindowRoot -match '(?m)^\s*unsafe fn (?:paint_hex_edit_frames|paint_numeric_edit_frames|paint_blur_edit_frame|paint_corner_edit_frame|paint_editor|draw_slider)\(') {
    throw 'Color editor frames and sliders must live in editor_paint.rs with original input focus, error, suffix and slider rendering.'
}
if ($styleWindowRoot -notmatch 'mod appearance_paint;' -or
    $styleWindowRoot -notmatch 'use self::appearance_paint::\*;' -or
    $appearancePaint -notmatch 'pub\(super\) fn submenu_mix\(' -or
    $appearancePaint -notmatch 'pub\(super\) fn submenu_content_alpha\(' -or
    $appearancePaint -notmatch 'pub\(super\) unsafe fn paint_submenu_well\(' -or
    $appearancePaint -notmatch 'pub\(super\) unsafe fn draw_appearance_choice_group\(' -or
    $appearancePaint -notmatch 'pub\(super\) unsafe fn paint_submenu_collapse_switch\(' -or
    $appearancePaint -notmatch 'pub\(super\) unsafe fn paint_submenu_content\(' -or
    $appearancePaint -notmatch 'pub\(super\) unsafe fn paint_appearance_page\(' -or
    $appearancePaint -notmatch '0\.45 \+ 0\.55 \* submenu_content_alpha\(progress\)' -or
    $appearancePaint -notmatch 'let saved = SaveDC\(hdc\)' -or
    $appearancePaint -notmatch 'RestoreDC\(hdc, saved\)' -or
    $appearancePaint -notmatch 'if submenu_only \{ return; \}' -or
    $appearancePaint -notmatch 'paint_preset_gallery\(' -or
    $appearancePaint -notmatch 'paint_hex_edit_frames\(' -or
    $appearancePaint -notmatch 'paint_numeric_edit_frames\(' -or
    $appearancePaint -notmatch 'paint_blur_edit_frame\(' -or
    $appearancePaint -notmatch 'paint_corner_edit_frame\(' -or
    $appearancePaint -notmatch 'draw_slider\(' -or
    $styleWindowRoot -match '(?m)^\s*(?:unsafe )?fn (?:submenu_mix|submenu_content_alpha|paint_submenu_well|draw_appearance_choice_group|paint_submenu_collapse_switch|paint_submenu_content|paint_appearance_page)\(') {
    throw 'Appearance page and desktop submenu painters must remain in appearance_paint.rs with existing selection, animation, clip and edit frames.'
}
if ($styleWindowRoot -notmatch 'mod preset_gallery_paint;' -or
    $styleWindowRoot -notmatch 'use self::preset_gallery_paint::\*;' -or
    $presetGalleryPaint -notmatch 'pub\(super\) fn preset_label\(' -or
    $presetGalleryPaint -notmatch 'pub\(super\) unsafe fn paint_preset_gallery\(' -or
    $presetGalleryPaint -notmatch 'pub\(super\) unsafe fn paint_style_preview_card\(' -or
    $presetGalleryPaint -notmatch 'if\s+!matched' -or
    $presetGalleryPaint -notmatch 'paint_vertical_style_preview\(' -or
    $presetGalleryPaint -notmatch 'paint_horizontal_style_preview\(' -or
    $presetGalleryPaint -notmatch 'StyleColorTarget::ProgressConsumed' -or
    $presetGalleryPaint -notmatch 'if selected \{ 2 \} else \{ 1 \}' -or
    $styleWindowRoot -match '(?m)^\s*(?:unsafe )?fn (?:preset_label|paint_preset_gallery|paint_style_preview_card)\(') {
    throw 'Preset gallery labels, cards and thumbnail rendering must live in preset_gallery_paint.rs.'
}
if ($styleWindowRoot -notmatch 'mod json_paint;' -or
    $styleWindowRoot -notmatch 'use self::json_paint::\*;' -or
    $jsonPaint -notmatch 'pub\(super\) unsafe fn paint_json_page\(' -or
    $jsonPaint -notmatch 'pub\(super\) unsafe fn paint_json_scrollbar_decoration\(' -or
    $jsonPaint -notmatch 'json_scroll_thumb_rect\(hwnd\)' -or
    $jsonPaint -notmatch 'json_editor_background\(snapshot\.is_dark\)' -or
    $jsonPaint -notmatch 'json_error_shake_offset\(hwnd, error_shake_step\)' -or
    $jsonPaint -notmatch 'JsonSaveFeedback::Saving' -or
    $jsonPaint -notmatch 'JsonSaveFeedback::Saved' -or
    $jsonPaint -notmatch 'JsonAction::Apply' -or
    $jsonPaint -notmatch 'paint_json_scrollbar_decoration\(hdc, hwnd, snapshot\.is_dark\)' -or
    $styleWindowRoot -match '(?m)^unsafe fn paint_json_(?:page|scrollbar_decoration)\(') {
    throw 'JSON page and scrollbar decoration painting must live in json_paint.rs without losing status and action rendering.'
}
if ($styleWindowRoot -notmatch 'mod render_primitives;' -or
    $styleWindowRoot -notmatch 'use self::render_primitives::\*;' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn draw_settings_icon\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn draw_segment_with_icon\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn draw_segment\(' -or
    $renderPrimitives -notmatch 'pub\(super\) fn subtle_control_border\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn fill_rounded_rect\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn draw_rounded_outline_rect\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn draw_switch\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn draw_outline_rect\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn draw_outline_rect_width\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn fill\(' -or
    $renderPrimitives -notmatch 'pub\(super\) unsafe fn draw_text\(' -or
    $renderPrimitives -notmatch 'native_interop::draw_antialiased_rounded_rect\(' -or
    $renderPrimitives -notmatch 'GetStockObject\(NULL_BRUSH\)' -or
    $renderPrimitives -notmatch 'SelectObject\(hdc, old_brush\)' -or
    $renderPrimitives -notmatch 'DeleteObject\(pen\)' -or
    $renderPrimitives -notmatch 'if text\.is_empty\(\)' -or
    $styleWindowRoot -match '(?m)^\s*(?:unsafe )?fn (?:draw_settings_icon|draw_segment_with_icon|draw_segment|subtle_control_border|fill_rounded_rect|draw_rounded_outline_rect|draw_switch|draw_outline_rect|draw_outline_rect_width|fill|draw_text)\(') {
    throw 'Settings drawing primitives must remain in render_primitives.rs with GDI fallbacks and empty-text guard.'
}
if ($styleWindowRoot -notmatch 'mod lifecycle_events;' -or
    $styleWindowRoot -notmatch 'x if is_settings_lifecycle_message\(x\)' -or
    $lifecycleEvents -notmatch 'pub\(super\) unsafe fn handle_settings_lifecycle_message\(' -or
    $lifecycleEvents -notmatch 'pub\(super\) fn is_settings_lifecycle_message\(' -or
    $lifecycleEvents -notmatch 'WM_DPICHANGED\s*=>' -or
    $lifecycleEvents -notmatch 'WM_SIZE\s*=>' -or
    $lifecycleEvents -notmatch 'WM_GETMINMAXINFO\s*=>' -or
    $lifecycleEvents -notmatch 'WM_CLOSE\s*=>' -or
    $lifecycleEvents -notmatch 'WM_DESTROY\s*=>' -or
    $lifecycleEvents -notmatch 'request_discard_confirmation\(hwnd, PendingDiscardAction::Close\)' -or
    $lifecycleEvents -notmatch 'send_parent\(WM_STYLE_SAVE, 0, 0\)' -or
    $lifecycleEvents -notmatch 'let _ = DestroyWindow\(hwnd\)' -or
    $lifecycleEvents -notmatch 'state.take\(\).map\(' -or
    $lifecycleEvents -notmatch 'end_submenu_frame_clock\(hwnd\)' -or
    $styleWindowRoot -match '(?m)^\s*WM_(?:DPICHANGED|SIZE|GETMINMAXINFO|CLOSE|DESTROY)\s*=>') {
    throw 'Settings lifecycle and DPI/size messages must remain in lifecycle_events.rs.'
}
if ($styleWindowRoot -notmatch 'mod timer_events;' -or
    $styleWindowRoot -notmatch 'x if is_settings_scheduled_message\(x\)' -or
    $timerEvents -notmatch 'pub\(super\) unsafe fn handle_settings_scheduled_message\(' -or
    $timerEvents -notmatch 'pub\(super\) fn is_settings_scheduled_message\(' -or
    $timerEvents -notmatch 'WM_TIMER\s*=>' -or
    $timerEvents -notmatch 'WM_SETTINGS_PREWARM_GALLERY\s*=>' -or
    $timerEvents -notmatch 'WM_SETTINGS_PRESET_GALLERY_REFRESH\s*=>' -or
    $timerEvents -notmatch 'WM_SETTINGS_MULTIMEDIA_ANIMATION_TICK\s*=>' -or
    $timerEvents -notmatch 'WM_SETTINGS_PRESENT_INTERACTIVE_FRAME\s*=>' -or
    $timerEvents -notmatch 'json_scroll_drag_paced\(' -and $timerEvents -notmatch 'update_json_scroll_drag_paced\(' -or
    $timerEvents -notmatch 'refresh_json_scrollbar_if_changed\(' -or
    $timerEvents -notmatch 'update_json_validation_status\(' -or
    $timerEvents -notmatch 'drive_submenu_frame_tick\(' -or
    $styleWindowRoot -match '(?m)^\s*WM_TIMER\s*=>') {
    throw 'Settings timer and scheduled refresh handlers must live in timer_events.rs.'
}
if ($styleWindowRoot -notmatch 'mod command_events;' -or
    $styleWindowRoot -notmatch 'WM_COMMAND\s*=>\s*handle_settings_command\(' -or
    $commandEvents -notmatch 'pub\(super\) unsafe fn handle_settings_command\(' -or
    $commandEvents -notmatch 'pub\(super\) fn decode_settings_command\(' -or
    $commandEvents -notmatch 'control_id == ID_EDIT_JSON' -or
    $commandEvents -notmatch 'target_from_hex_control_id\(control_id\)' -or
    $commandEvents -notmatch 'EN_CHANGE_CODE\s*=>' -or
    $commandEvents -notmatch 'EN_SETFOCUS_CODE\s*=>' -or
    $commandEvents -notmatch 'EN_KILLFOCUS_CODE\s*=>' -or
    $commandEvents -notmatch 'update_color_from_numeric_edit\(' -or
    $commandEvents -notmatch 'sync_corner_edit\(' -or
    $commandEvents -notmatch 'update_blur_from_numeric_edit\(' -or
    $styleWindowRoot -match '(?m)^\s*let control_id = \(wparam\.0 \& 0xFFFF\) as u16;') {
    throw 'Edit-control WM_COMMAND handlers must remain in command_events.rs.'
}
if ($styleWindowRoot -notmatch 'mod input_events;' -or
    $styleWindowRoot -notmatch 'x if is_settings_input_message\(x\)' -or
    $inputEvents -notmatch 'pub\(super\) unsafe fn handle_settings_input_message\(' -or
    $inputEvents -notmatch 'WM_MOUSEWHEEL\s*=>' -or
    $inputEvents -notmatch 'WM_SETCURSOR\s*=>' -or
    $inputEvents -notmatch 'WM_LBUTTONDOWN\s*=>' -or
    $inputEvents -notmatch 'WM_MOUSEMOVE\s*=>' -or
    $inputEvents -notmatch 'WM_MOUSELEAVE_MSG\s*=>' -or
    $inputEvents -notmatch 'WM_LBUTTONUP\s*=>' -or
    $inputEvents -notmatch 'WM_CANCELMODE\s*\|\s*WM_CAPTURECHANGED\s*=>' -or
    $styleWindowRoot -match '(?m)^\s*WM_LBUTTONDOWN\s*=>' -or
    $styleWindowRoot -match '(?m)^\s*WM_MOUSEMOVE\s*=>') {
    throw 'Settings mouse/wheel/capture event handling must live in input_events.rs.'
}
if ($styleWindowRoot -notmatch 'mod json_scroll;' -or
    $jsonScroll -notmatch 'pub\(super\) fn json_scroll_drag_should_apply\(' -or
    $jsonScroll -notmatch 'pub\(super\) fn json_scroll_thumb_rect\(' -or
    $jsonScroll -notmatch 'pub\(super\) fn json_scrollbar_wheel_target\(' -or
    $jsonScroll -notmatch 'pub\(super\) fn reset_json_scroll_frame_clock\(' -or
    $styleWindowRoot -match '(?m)^\s*fn json_scroll_line_metrics\(' -or
    $styleWindowRoot -match '(?m)^\s*fn update_json_scroll_drag\(') {
    throw 'JSON scroll geometry, pacing and drag management must live in json_scroll.rs.'
}
if ($styleWindow -notmatch 'mod json_support;' -or
    $jsonSupport -notmatch 'pub\(super\) fn jsonc_highlight_ranges\(' -or
    $jsonSupport -notmatch 'pub\(super\) fn friendly_json_error\(' -or
    $jsonSupport -notmatch 'pub\(super\) fn normalize_to_lf\(' -or
    ($styleWindow -split 'mod json_support;', 2)[0] -match 'fn jsonc_highlight_ranges\(') {
    throw 'JSONC text helpers must live in the shared JSON support module.'
}
$windowCommon = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window_common.rs')
if ($styleWindow -notmatch 'pub\(super\) unsafe fn paint_horizontal_style_preview\(' -or
    $styleWindow -notmatch 'pub\(super\) unsafe fn paint_vertical_style_preview\(' -or
    $styleWindow -notmatch 'pub\(super\) fn preview_ring_angles\(') {
    throw 'Preset preview painting must remain in the dedicated settings module.'
}
$settingsModel = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\settings_model.rs')
$popupMenu = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\popup_menu.rs')
$fonts = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\fonts.rs')
$windowProduction = ($window -split '#\[cfg\(test\)\]', 2)[0]
$windowProduction += "`n" + (Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\taskbar_replica.rs'))
$windowProduction += "`n" + (Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\blur_backdrop.rs'))
$windowProduction += "`n" + (Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\tooltip_measure.rs'))
$windowProduction += "`n" + (Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\floating_geometry.rs'))
$styleProduction = ($style -split '#\[cfg\(test\)\]', 2)[0]
$layeredBitmap = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\layered_bitmap.rs')
if ($layeredBitmap -notmatch 'struct WindowDc' -or
    $layeredBitmap -notmatch 'struct DibSurface' -or
    $layeredBitmap -notmatch 'impl Drop for WindowDc' -or
    $layeredBitmap -notmatch 'impl Drop for DibSurface' -or
    $layeredBitmap -notmatch 'SelectObject\(self\.dc, self\.previous\)' -or
    $layeredBitmap -notmatch 'DeleteObject\(self\.bitmap\)' -or
    $layeredBitmap -notmatch 'ReleaseDC\(self\.owner, self\.dc\)' -or
    $windowProduction -notmatch 'DibSurface::create\(screen_dc, width, height\)' -or
    $windowProduction -notmatch 'DibSurface::create\(screen_dc, canvas_w, canvas_h\)' -or
    ([regex]::Matches($windowProduction, 'DibSurface::create\(screen_dc, width, height\)')).Count -ne 2 -or
    ([regex]::Matches($windowProduction, 'reset_floating_transition_state\(s\)')).Count -ne 2 -or
    $windowProduction -notmatch 'fn reset_floating_transition_state\(') {
    throw 'Layered widget and tooltip must use scoped GDI handles and desktop transitions must share one reset routine.'
}



foreach ($mode in @('System', 'Dark', 'Light')) {
    if ($styleProduction -notmatch ("\b" + $mode + "\b")) {
        throw "Theme mode $mode is missing."
    }
}
if ($windowProduction -notmatch 'IDM_THEME_SYSTEM' -or
    $windowProduction -notmatch 'IDM_THEME_DARK' -or
    $windowProduction -notmatch 'IDM_THEME_LIGHT') {
    throw 'Top-level theme menu commands are missing.'
}
if ($styleWindow -notmatch '"排版"' -or
    $styleWindow -notmatch '"主题"' -or
    $windowProduction -notmatch '"设置\.\.\."' -or
    $windowProduction -notmatch '"Settings\.\.\."') {
    throw 'Unified settings must retain appearance Theme/Layout controls and expose one top-level Settings entry.'
}
if ($styleProduction -notmatch 'enum\s+ThemePreset' -or
    $styleProduction -notmatch 'Classic' -or
    $styleProduction -notmatch 'Ocean' -or
    $styleProduction -notmatch 'Forest' -or
    $styleProduction -notmatch 'apply_preset\(' -or
    $styleProduction -notmatch 'matches_preset\(' -or
    $styleWindow -notmatch '"石墨"' -or
    $styleWindow -notmatch '"深海"' -or
    $styleWindow -notmatch '"松影"' -or
    $styleWindow -notmatch '"云瓷"' -or
    $styleWindow -notmatch '"晴湾"' -or
    $styleWindow -notmatch '"麦光"' -or
    $styleWindow -notmatch '"Graphite"' -or
    $styleWindow -notmatch '"Deep Sea"' -or
    $styleWindow -notmatch '"Pine Shade"' -or
    $styleWindow -notmatch '"Cloud Porcelain"' -or
    $styleWindow -notmatch '"Clear Bay"' -or
    $styleWindow -notmatch '"Wheat Glow"' -or
    $styleWindow -notmatch 'Section::Preset' -or
    $styleWindow -notmatch 'paint_preset_gallery\(' -or
    $styleWindow -notmatch 'preset_card_rect\(' -or
    $styleWindow -notmatch 'WM_STYLE_PRESET_CHANGE' -or
    $windowProduction -notmatch 'WM_STYLE_PRESET_CHANGE' -or
    $windowProduction -notmatch 'ThemePreset::from_index') {
    throw 'Dark and light presets must have distinct user-facing names and live in the dedicated Presets section.'
}
if ($styleWindow -match 'fn\s+preset_rect\(') {
    throw 'The old top-row preset strip must stay removed; presets belong in sidebar preview cards.'
}
foreach ($hex in @(
    '#0F1B24FF', '#294252FF', '#3FB7E9FF',
    '#14211DFF', '#2B4038FF', '#56C596FF',
    '#F6F8FAFF', '#C7D0D9FF', '#46515DFF', '#17212BFF', '#596777FF', '#B33F49FF', '#4A8FD8FF',
    '#DCEFF5FF', '#AFCFD8FF', '#335965FF', '#12343DFF', '#426976FF', '#AD4146FF', '#1597B7FF',
    '#FFF1DCFF', '#DDBF8FFF', '#685339FF', '#332514FF', '#795F3DFF', '#B24039FF', '#4E8A6EFF'
)) {
    if ($styleProduction -notmatch [regex]::Escape($hex)) {
        throw "Expected coordinated theme-preset color is missing: $hex"
    }
}
if ($style -notmatch 'preset_text_colors_keep_readable_contrast' -or
    $style -notmatch 'redesigned_light_presets_keep_error_text_readable' -or
    $style -notmatch 'redesigned_light_presets_are_visually_distinct' -or
    $style -notmatch 'ratio\s*>=\s*4\.5') {
    throw 'Theme presets must keep an automated readable text-contrast quality gate.'
}
if ($windowProduction -notmatch 'IDM_STYLE_SETTINGS' -or
    $styleWindow -notmatch 'StyleWindowSnapshot' -or
    $styleWindow -notmatch 'WM_STYLE_COLOR_PREVIEW' -or
    $styleWindow -notmatch 'WM_STYLE_BLUR_PREVIEW' -or
    $styleWindow -notmatch 'Section::General' -or
    $styleWindow -notmatch 'Section::Preset' -or
    $styleWindow -notmatch 'Section::Panel' -or
    $styleWindow -notmatch 'Section::Tooltip' -or
    $styleWindow -notmatch 'Section::Text' -or
    $styleWindow -notmatch 'Section::Progress' -or
    $styleWindow -notmatch 'Section::Interaction' -or
    $styleWindow -notmatch 'Section::Json') {
    throw 'Unified General/Appearance/Advanced settings center contract is missing.'
}
if ($windowProduction -match 'AppendMenuW\([\s\S]{0,180}theme_menu' -or
    $windowProduction -match 'AppendMenuW\([\s\S]{0,180}layout_menu') {
    throw 'Theme and Layout must not be duplicated in the taskbar context menu.'
}
if ($styleWindow -match 'paint_preview\(') {
    throw 'The settings-panel preview card must be removed; the taskbar widget is the live preview.'
}
if ($styleWindow -match 'msctls_trackbar32' -or $styleWindow -match 'WM_HSCROLL') {
    throw 'Unified style panel must use self-drawn sliders instead of white native trackbars.'
}
if ($styleWindow -match '"RGBA"' -or $styleWindow -match '"强度调节"' -or $styleWindow -match '"Intensity"') {
    throw 'Redundant RGBA/intensity editor headings must stay removed.'
}
if ($styleWindow -match 'R/G/B 调整颜色' -or
    $styleWindow -match 'R/G/B adjust color' -or
    $styleWindow -match '0% 关闭磨砂' -or
    $styleWindow -match '0% turns blur off') {
    throw 'Adjustment help text must not be shown in the style panel.'
}
if ($styleWindow -notmatch 'HitTarget' -or
    $styleWindow -notmatch 'hovered:\s*Option<HitTarget>' -or
    $styleWindow -notmatch 'pressed:\s*Option<HitTarget>' -or
    $styleWindow -notmatch 'button_background\(' -or
    $styleWindow -notmatch 'TrackMouseEvent') {
    throw 'All style-panel buttons must expose hover and pressed feedback.'
}
if (($styleWindow -notmatch 'CreateWindowExW\(\s*WS_EX_APPWINDOW' -and
     $styleWindow -notmatch 'CreateWindowExW\(\s*settings_window_ex_style\(\)') -or
    $styleWindow -match 'WS_MAXIMIZEBOX' -or
    $styleWindow -notmatch 'WS_CAPTION\s*\|\s*WS_SYSMENU\s*\|\s*WS_MINIMIZEBOX' -or
    $styleWindow -notmatch 'WINDOW_HEIGHT,\s*HWND::default\(\)' -or
    $styleWindow -notmatch 'WM_CLOSE\s*=>\s*\{' -or
    $styleWindow -notmatch 'send_parent\(WM_STYLE_SAVE, 0, 0\)' -or
    $styleWindow -notmatch 'DestroyWindow\(hwnd\)') {
    throw 'Settings must be an ownerless taskbar window with minimize support and a standard caption close button.'
}
if ($styleWindow -notmatch 'fn settings_window_ex_style\(\)' -or
    $styleWindow -notmatch 'WS_EX_APPWINDOW\s*\|\s*WS_EX_COMPOSITED' -or
    $styleWindow -notmatch 'fn settings_composition_for_section\(section: Section\)' -or
    $styleWindow -notmatch 'section != Section::Json' -or
    $styleWindow -notmatch 'fn json_scroll_drag_should_apply\(' -or
    $styleWindow -notmatch 'fn present_json_scroll_frame_if_due\(') {
    throw 'JSON must use the default uncomposited scrolling optimization and preserve non-JSON composition.'
}
foreach ($debugFlag in @(
    'CODEX_USAGE_ANIMATION_METRICS',
    'CODEX_USAGE_JSON_SCROLL_METRICS',
    'CODEX_USAGE_JSON_SCROLL_NO_COMPOSITED',
    'CODEX_USAGE_JSON_SCROLL_KEEP_COMPOSITED',
    'CODEX_USAGE_JSON_SCROLL_DRAG_COALESCE_DISABLED',
    'CODEX_USAGE_JSON_SCROLL_FAST_FRAMES_DISABLED'
)) {
    if ($styleWindow.Contains($debugFlag)) {
        throw "Retired settings diagnostics environment switch must not return: $debugFlag"
    }
}
if ($styleWindow -notmatch 'json_save_mask' -or
    $styleWindow -notmatch 'WS_EX_LAYERED\s*\|\s*WS_EX_TOOLWINDOW\s*\|\s*WS_EX_NOACTIVATE' -or
    $styleWindow -notmatch 'WS_POPUP' -or
    $styleWindow -notmatch 'JSON_SAVE_MASK_ALPHA') {
    throw 'JSON save feedback mask must use an owned no-activate layered popup without changing the main settings window style.'
}
if ($styleWindow -notmatch 'ID_EDIT_R' -or
    $styleWindow -notmatch 'ES_NUMBER' -or
    $styleWindow -match 'WS_CHILD\.0\s*\|\s*WS_VISIBLE\.0\s*\|\s*ES_NUMBER' -or
    $styleWindow -notmatch 'EN_CHANGE_CODE' -or
    $styleWindow -notmatch 'sync_numeric_edits\(' -or
    $styleWindow -notmatch 'update_color_from_numeric_edit\(') {
    throw 'RGBA values must use linked numeric inputs that are created hidden and shown only on editor pages.'
}
if ($styleWindow -notmatch 'Section::General\s*\|\s*Section::Preset\s*\|\s*Section::Json' -or
    $styleWindow -notmatch 'preset_page_can_open_paint_and_destroy_without_editor_reentry') {
    throw 'Non-color settings pages must avoid hidden EDIT synchronization and keep the Win32 open/paint/destroy smoke test.'
}
if ($styleWindow -notmatch 'ID_EDIT_HEX_BASE' -or
    $styleWindow -notmatch 'HEX_EDIT_COUNT:\s*usize\s*=\s*13' -or
    $styleWindow -notmatch 'sync_hex_edits\(' -or
    $styleWindow -notmatch 'update_color_from_hex_edit\(' -or
    $styleWindow -notmatch 'parse_hex_input\(' -or
    $styleWindow -notmatch 'focused_hex_edit' -or
    $styleWindow -notmatch 'invalid_hex_edits' -or
    $styleWindow -notmatch 'select_editor\(EditorSelection::Color\(target\)\)') {
    throw 'Every color row must expose an editable Hex field that selects and synchronizes the matching RGBA editor.'
}
if ($styleWindow -notmatch 'sync_numeric_edits\(\);\s*sync_hex_edits\(\);' -or
    $styleWindow -notmatch 'set_edit_text_string\(' -or
    $styleWindow -notmatch 'Color::try_from_hex') {
    throw 'Hex and RGBA color editors must synchronize in both directions.'
}
if ($styleWindow -notmatch 'ID_EDIT_BLUR' -or
    $styleWindow -notmatch 'sync_blur_edit\(' -or
    $styleWindow -notmatch 'update_blur_from_numeric_edit\(' -or
    $styleWindow -notmatch 'focused_blur_edit' -or
    $styleWindow -notmatch 'blur_edit_frame_rect\(' -or
    $styleWindow -notmatch 'Section::Panel\s*\|\s*Section::Tooltip' -or
    $styleWindow -notmatch 'WM_STYLE_TOOLTIP_BLUR_PREVIEW') {
    throw 'Panel and Tooltip frosted intensity must use the shared inline 0-100 slider and numeric input.'
}
if ($styleWindow -notmatch 'editor_layout_snapshot\(' -or
    $styleWindow -notmatch 'release_editor_focus_before_layout\(' -or
    $styleWindow -notmatch 'hiding_focused_color' -or
    $styleWindow -notmatch 'hiding_focused_blur' -or
    $styleWindow -notmatch 'hiding_focused_corner' -or
    $styleWindow -notmatch 'SetFocus\(hwnd\)' -or
    $styleWindow -notmatch 'ShowWindow/SetFocus can synchronously send') {
    throw 'Style editor switching must release STATE before focus/visibility Win32 calls to avoid EDIT focus deadlocks.'
}
if ($styleWindow -match 'WS_BORDER' -or
    $styleWindow -notmatch 'numeric_edit_frame_rect\(' -or
    $styleWindow -notmatch 'focused_numeric_edit' -or
    $styleWindow -notmatch 'paint_numeric_edit_frames\(') {
    throw 'RGBA numeric inputs must use the custom focus-aware borderless style.'
}
if ($styleWindow -match 'WINDOW_HEIGHT_BLUR' -or
    $styleWindow -match 'resize_for_editor\(' -or
    $styleWindow -match '"当前强度"' -or
    $styleWindow -match '"Current"' -or
    $styleWindow -notmatch 'blur_slider_track_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'blur_edit_frame_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'inline_numeric_frame_rect\(' -or
    $styleWindow -notmatch 'blur_suffix_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'matches!\(editor, EditorSelection::Color\(_\)\)') {
    throw 'Blur must stay inline in its actual row, share the numeric frame geometry, keep its percent suffix internal, and never render a lower secondary editor.'
}
if ($styleWindow -notmatch 'select_editor\(EditorSelection::Blur\)' -or
    $styleWindow -notmatch 'blur selection must clear the lower RGBA editor' -or
    $styleWindow -notmatch 'hex_input_accepts_rgb_rgba_and_transient_partial_values') {
    throw 'Clicking blur must clear lower RGBA, while Hex focus must restore the matching RGBA editor in the Win32 smoke test.'
}
if ($styleWindow -notmatch 'DwmSetWindowAttribute' -or
    $styleWindow -notmatch 'DWMWA_CAPTION_COLOR' -or
    $styleWindow -notmatch 'DWMWA_TEXT_COLOR' -or
    $styleWindow -notmatch 'FIXED_CAPTION_COLORREF:\s*u32\s*=\s*0x00524843' -or
    $styleWindow -notmatch 'FIXED_CAPTION_TEXT_COLORREF:\s*u32\s*=\s*0x00FFFFFF' -or
    $styleWindow -notmatch 'apply_fixed_titlebar\(' -or
    $styleWindow -match 'DWMWA_USE_IMMERSIVE_DARK_MODE' -or
    $styleWindow -match 'DWMWA_TRANSITIONS_FORCEDISABLED') {
    throw 'Style window title bar must use one fixed neutral caption color in every theme.'
}
if ($styleWindow -notmatch 'WM_SETCURSOR' -or
    $styleWindow -notmatch 'IDC_HAND' -or
    $styleWindow -notmatch 'IDC_SIZEWE' -or
    $styleWindow -notmatch 'IDC_IBEAM' -or
    $styleWindow -notmatch 'cursor_hwnd' -or
    $styleWindow -notmatch 'slider_kind_at\(' -or
    $styleWindow -notmatch 'hit_target_at\(') {
    throw 'Style panel buttons, sliders, and numeric inputs must expose semantic mouse cursors.'
}
if ($windowProduction -notmatch 'WM_SETCURSOR' -or
    $windowProduction -notmatch 'small_taskbar_mode' -or
    $windowProduction -notmatch 'IDC_HAND') {
    throw 'Small-taskbar click-to-toggle mode must expose a hand cursor outside the drag handle.'
}
if ($windowProduction -notmatch 'popup_menu::show' -or
    $windowProduction -notmatch 'PopupItem::submenu' -or
    $windowProduction -match 'TrackPopupMenu' -or
    $windowProduction -match 'CreatePopupMenu') {
    throw 'Tray menu must use the standalone custom popup window rather than native HMENU/TrackPopupMenu.'
}
if ($popupMenu -notmatch 'WS_POPUP' -or
    $popupMenu -notmatch 'CS_DROPSHADOW' -or
    $popupMenu -notmatch 'DWMWA_WINDOW_CORNER_PREFERENCE' -or
    $popupMenu -notmatch 'ITEM_RADIUS' -or
    $popupMenu -notmatch 'PopupAction::Submenu' -or
    $popupMenu -notmatch 'menu_width\(' -or
    $popupMenu -notmatch 'DT_CALCRECT' -or
    $popupMenu -notmatch 'MENU_MIN_WIDTH' -or
    $popupMenu -match 'ROOT_WIDTH' -or
    $popupMenu -match 'SUBMENU_WIDTH') {
    throw 'Custom popup menu must provide native popup styling and content-adaptive root/submenu width.'
}
if ($windowProduction -notmatch 'PopupItem::command\(settings_text, IDM_STYLE_SETTINGS\)' -or
    $windowProduction -match 'settings_menu') {
    throw 'Unified Settings must remain one top-level custom-popup entry.'
}
foreach ($hex in @('#E9EEF4FF', '#DCE5EFFF', '#CBD7E4FF', '#C1CCD8FF', '#EEF3F8FF')) {
    if ($styleWindow -notmatch [regex]::Escape($hex)) {
        throw "Expected higher-contrast light style-panel color is missing: $hex"
    }
}
if ($styleWindow -notmatch 'WS_CLIPCHILDREN' -or
    $styleWindow -notmatch 'CreateCompatibleDC' -or
    $styleWindow -notmatch 'CreateCompatibleBitmap' -or
    $styleWindow -notmatch 'BitBlt') {
    throw 'Style panel must use clipped child controls and double-buffered painting to reduce flicker.'
}
if ($styleWindow -notmatch 'WM_APP \+ 120' -or
    $styleWindow -notmatch 'WM_APP \+ 126' -or
    $styleWindow -notmatch 'WM_APP \+ 132' -or
    $styleWindow -notmatch 'draw_slider\(') {
    throw 'Unified settings messages must use a collision-free WM_APP range and appearance must keep self-drawn sliders.'
}
if ($styleProduction -notmatch 'pub\s+dark:\s+ThemeStyle' -or $styleProduction -notmatch 'pub\s+light:\s+ThemeStyle') {
    throw 'Dark and light theme styles must be stored separately.'
}
if ($styleProduction -notmatch 'panel_corner_radius_max\(' -or
    $styleProduction -notmatch 'tooltip_corner_radius_max\(' -or
    $styleProduction -notmatch 'progress_corner_radius_max\(' -or
    $styleProduction -notmatch 'metrics\(\)\.widget_height' -or
    $styleProduction -notmatch 'metrics\(\)\.bar_height' -or
    $styleWindow -notmatch 'ID_EDIT_CORNER' -or
    $styleWindow -notmatch 'EditorSelection::CornerRadius' -or
    $styleWindow -notmatch 'corner_radius_max_for_section\(' -or
    $styleWindow -notmatch 'raw_value\.min\(u16::from\(radius_max\)\)' -or
    $styleWindow -notmatch 'WM_STYLE_CORNER_PREVIEW' -or
    $windowProduction -notmatch 'clamp_corner_radii') {
    throw 'Panel, tooltip, and progress corner-radius limits must adapt to their actual component dimensions.'
}
if ($styleProduction -notmatch 'panel_frosted_strength:\s*u8' -or
    $styleProduction -notmatch 'FROSTED_STRENGTH_MAX:\s*u8\s*=\s*100') {
    throw 'Per-theme 0-100 frosted intensity setting is missing.'
}
if ($styleProduction -notmatch 'tooltip_background:\s*Option<String>' -or
    $styleProduction -notmatch 'tooltip_border:\s*Option<String>' -or
    $styleProduction -notmatch 'tooltip_frosted_strength:\s*Option<u8>' -or
    $styleProduction -notmatch 'StyleColorTarget::TooltipBackground' -or
    $styleProduction -notmatch 'StyleColorTarget::TooltipBorder' -or
    $styleProduction -notmatch 'unwrap_or\(&self\.panel_background\)' -or
    $styleProduction -notmatch 'unwrap_or\(&self\.panel_border\)' -or
    $styleWindow -notmatch '"浮框"' -or
    $styleWindow -notmatch 'Section::Tooltip' -or
    $styleWindow -notmatch 'EditorSelection::TooltipBlur') {
    throw 'Tooltip style must expose independent background, border, and frosted controls.'
}
if ($styleWindow -match '恢复继承面板样式' -or
    $styleWindow -match 'Inherit panel style' -or
    $styleWindow -match '恢复当前主题默认' -or
    $styleWindow -match 'Reset current theme' -or
    $styleWindow -match 'WM_STYLE_TOOLTIP_RESET' -or
    $styleWindow -match 'WM_STYLE_RESET_CURRENT') {
    throw 'Appearance pages must not expose reset buttons; presets are the single restore path.'
}
if ($styleProduction -notmatch 'tooltip_background:\s*Some\(' -or
    $styleProduction -notmatch 'tooltip_border:\s*Some\(' -or
    $styleProduction -notmatch 'tooltip_frosted_strength:\s*Some\(0\)' -or
    $styleProduction -notmatch '\*self\s*=\s*Self::preset\(is_dark, preset\)' -or
    $styleWindow -notmatch 'StyleColorTarget::TooltipBackground' -or
    $styleWindow -notmatch 'StyleColorTarget::TooltipBorder') {
    throw 'Built-in appearance presets must include and preview tooltip styling and restore the complete appearance.'
}
if ($native -notmatch 'pub\s+a:\s+u8' -or $native -notmatch 'to_hex_rgba') {
    throw 'Native Color must support an alpha channel and RGBA serialization.'
}
foreach ($hex in @(
    '#242A31FF', '#343B43FF', '#A0A0A0FF', '#FFFFFFFF', '#92979DFF',
    '#F6F8FAFF', '#C7D0D9FF', '#46515DFF', '#17212BFF', '#596777FF',
    '#55A8F2FF', '#E6B84AFF', '#D95C5CFF', '#363A3FFF', '#D6DDE4FF'
)) {
    if ($styleProduction -notmatch [regex]::Escape($hex)) {
        throw "Expected RGBA default is missing: $hex"
    }
}
if ($windowProduction -notmatch 'TB_ENDTRACK_CODE') {
    throw 'Continuous editors must save when trackbar interaction ends.'
}
if ($windowProduction -notmatch 'apply_style_color\(' -or
    $windowProduction -notmatch 'apply_frosted_strength\(') {
    throw 'Color and frosted-intensity editors must apply live previews.'
}
if ($windowProduction -notmatch 'render_layered\(\);[\s\S]{0,200}TB_ENDTRACK_CODE') {
    Write-Warning 'Live-preview implementation shape changed; inspect manually if this warning appears.'
}
if ($native -notmatch 'create_composition_blur' -or
    $native -notmatch 'set_composition_blur_amount' -or
    $native -notmatch 'destroy_composition_blur') {
    throw 'Rust must use the native Windows Composition Gaussian-blur helper.'
}
if ($composition -notmatch 'CreateDesktopWindowTarget' -or
    $composition -notmatch 'CreateBackdropBrush' -or
    $composition -notmatch 'CLSID_D2D1GaussianBlur' -or
    $composition -notmatch 'Blur\.BlurAmount' -or
    $composition -notmatch 'IGraphicsEffectD2D1Interop') {
    throw 'Native helper must use DesktopWindowTarget + BackdropBrush + real GaussianBlurEffect.'
}
if ($composition -notmatch 'codex_composition_blur_set_bounds' -or
    $composition -notmatch 'codex_composition_blur_set_desktop_bounds' -or
    $composition -notmatch 'CreateRoundedRectangleGeometry' -or
    $composition -notmatch 'CreateGeometricClip' -or
    $composition -notmatch 'panel_geometry' -or
    $composition -notmatch 'grip_geometry' -or
    $composition -notmatch 'geometry\.CornerRadius' -or
    $composition -notmatch 'root\.Size\(size\)' -or
    $composition -notmatch 'blur_visual\.Size\(size\)' -or
    $composition -notmatch 'grip_blur_visual\.Size\(grip_size\)' -or
    $composition -notmatch 'grip_visual\.Opacity\(alpha\)' -or
    $composition -notmatch 'tint_visual\.Size\(size\)') {
    throw 'Composition backdrop must hard-clip the taskbar panel and independently clip, fade and blur the desktop grip without stale-DPI tails.'
}
# All settings buttons use the same embedded SVG assets, including GDI-only hosts.
$svgIcons = Get-ChildItem (Join-Path $PSScriptRoot '..\assets\settings-icons') -Filter '*.svg'
$svgRaster = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\svg_icons.rs')
if ($svgIcons.Count -ne 31 -or
    $native -notmatch 'codex_blend_settings_icon' -or
    $controlPrimitives -notmatch 'codex_blend_settings_icon' -or
    $controlPrimitives -notmatch 'AlphaBlend\(' -or
    $styleWindow -notmatch 'crate::svg_icons::draw\(' -or
    $svgRaster -notmatch 'resvg::render\(' -or
    $svgRaster -notmatch 'CACHE_CAPACITY' -or
    $svgRaster -notmatch 'premultiply_alpha' -or
    $controlPrimitives -match 'codex_draw_settings_icon' -or
    $native -match 'codex_draw_settings_icon') {
    throw 'All 31 settings glyphs must be embedded SVG files, theme/DPI raster-cached and alpha-composited.'
}

if ($native -notmatch 'set_composition_blur_bounds' -or
    $native -notmatch 'clip_inset' -or
    $native -notmatch 'clip_radius' -or
    $windowProduction -notmatch 'sync_composition_blur_bounds' -or
    $windowProduction -notmatch 'outer_inset \+ PANEL_BORDER_WIDTH_PX' -or
    $windowProduction -notmatch 'outer_radius - PANEL_BORDER_WIDTH_PX') {
    throw 'Rust must synchronize exact inner-fill geometry with the Composition backdrop.'
}
if ($windowProduction -notmatch 's\.taskbar_hwnd\.unwrap_or_else\(\|\| s\.hwnd\.to_hwnd\(\)\)') {
    throw 'DPI refresh must prefer the selected taskbar during cross-monitor popup moves.'
}
if ($windowProduction -notmatch 'FROSTED_MAX_BLUR_PX:\s*f32\s*=\s*20\.0' -or
    $windowProduction -notmatch 'blur_amount_for_strength' -or
    $windowProduction -notmatch 'FROSTED_MAX_BLUR_PX\s*\*\s*f32::from\(strength' -or
    $styleProduction -notmatch 'panel_frosted_strength\s*>\s*0') {
    throw 'Frosted intensity must map 0-100 linearly to a real Gaussian blur amount.'
}
if ($native -match 'SetWindowCompositionAttribute' -or
    $native -match 'ACCENT_ENABLE_ACRYLICBLURBEHIND' -or
    $native -match 'set_native_acrylic') {
    throw 'Legacy WCA Acrylic backend must not remain in the production blur path.'
}
if ($styleProduction -match 'LEGACY_FROSTED_STRENGTH_SENTINEL|panel_blur_radius|alias = "panel_rounded"' -or
    $settingsModel -match 'LegacyRounded|alias = "tooltip_rounded"') {
    throw 'Deprecated frost and corner aliases must not remain.'
}
if ($windowProduction -notmatch 'BLUR_BACKDROP_HWND' -or
    $windowProduction -notmatch 'BLUR_BACKDROP_CONTEXT' -or
    $windowProduction -notmatch 'ensure_blur_backdrop' -or
    $windowProduction -notmatch 'sync_blur_backdrop_zorder' -or
    $windowProduction -notmatch 'WS_EX_NOREDIRECTIONBITMAP') {
    throw 'Frosted glass must use a separate no-redirection Composition backdrop window.'
}
if ($windowProduction -match 'set_layered_style\(hwnd, false\)') {
    throw 'The foreground widget must remain layered while frosted glass is active.'
}
if ($windowProduction -notmatch 'activate_blur_backdrop' -or
    $windowProduction -notmatch 'restore_layered_taskbar_mode' -or
    $windowProduction -notmatch 'park_embedded_blur_backdrop' -or
    $windowProduction -notmatch 'restore_parked_embedded_blur_backdrop') {
    throw 'Frosted foreground must stay taskbar-embedded and restore cached child Composition backdrops.'
}
if ($windowProduction -notmatch 'MIN_INTERACTIVE_ALPHA:\s*u8\s*=\s*1' -or
    $windowProduction -notmatch 'else if background\.a == 0' -or
    $windowProduction -notmatch 'surface_style\.panel_background\s*=\s*Color::rgba' -or
    $windowProduction -notmatch 'UpdateLayeredWindow') {
    throw 'Transparent layered panels must keep a minimally nonzero alpha so blank areas remain interactive with or without Composition blur.'
}
if ($windowProduction -notmatch 'try_taskbar_replica_handoff' -or
    $windowProduction -notmatch 'sync_blur_backdrop_zorder' -or
    $windowProduction -notmatch 'GetAncestor\(blur, GA_PARENT\) == taskbar\.hwnd') {
    throw 'Cross-taskbar embedded blur handoff must validate each sibling and reuse cached backgrounds.'
}
if ($native -notmatch 'monitor_device_name\(' -or
    $native -notmatch 'MONITORINFOEXW' -or
    $native -notmatch 'monitor_device:\s*Option<String>' -or
    $windowProduction -notmatch 'taskbar_left_offset:\s*i32' -or
    $windowProduction -notmatch 'taskbar_monitor:\s*Option<String>' -or
    $windowProduction -match 'legacy_tray_offset|legacy_left_offset') {
    throw 'Taskbar placement must use a stable monitor identity and a left-edge offset without legacy migration.'
}
$positionBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn\s+position_at_taskbar\s*\(\)\s*\{.*?\n\}'
).Value
if ($positionBlock -notmatch 'taskbar_free_spans\(' -or
    $positionBlock -notmatch 'placement_for_free_spans\(' -or
    $positionBlock -match 'taskbar_left_offset\s*=\s*actual_left_offset') {
    throw 'Taskbar layout changes must snap only the current frame into a safe free span without persisting positional drift.'
}
if ($windowProduction -notmatch 'current_taskbar_hwnd\s*!=\s*Some\(hovered_taskbar\.hwnd\)' -or
    $windowProduction -notmatch 'attach_to_taskbar_window\(' -or
    $windowProduction -match 'attach_to_taskbar\(hwnd,\s*hovered_taskbar_index\)') {
    throw 'Cross-monitor dragging must bind the exact taskbar HWND already hit-tested instead of re-resolving an array index.'
}
$watchdogBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn\s+spawn_taskbar_watchdog\s*\(\)\s*\{.*?\n\}'
).Value
if ($watchdogBlock -notmatch 'taskbar_monitor' -or
    $watchdogBlock -notmatch 'waiting instead of switching monitors') {
    throw 'Taskbar watchdog recovery must wait for the same monitor instead of silently moving the widget to another taskbar.'
}
if ($windowProduction -notmatch 'STYLE_PREVIEW_FRAME_MS:\s*u64\s*=\s*16' -or
    $windowProduction -notmatch 'render_style_preview\(' -or
    $windowProduction -notmatch 'LAST_STYLE_PREVIEW_RENDER') {
    throw 'Live style preview must be frame-limited instead of repainting for every raw trackbar event.'
}
if ($windowProduction -notmatch 'DRAG_FRAME_MS:\s*u64\s*=\s*8' -or
    $windowProduction -notmatch 'drag_frame_due\(' -or
    $windowProduction -notmatch 'move_frosted_pair') {
    throw 'High-frequency drag updates must be throttled and move the frosted pair without z-order churn.'
}
if ($windowProduction -notmatch 'move_window_without_repaint' -or
    $windowProduction -notmatch 'SWP_NOZORDER\s*\|\s*SWP_NOACTIVATE') {
    throw 'Drag motion must preserve existing layered pixels instead of requesting redundant repaint work.'
}
if ($windowProduction -notmatch 'BLUR_BACKDROP_PARAMS' -or
    $windowProduction -notmatch '\*cached == Some\(params\)') {
    throw 'Composition blur/tint parameters must be cached to avoid redundant effect updates.'
}
$mouseMoveBlock = [regex]::Match(
    $windowProduction,
    '(?s)WM_MOUSEMOVE\s*=>\s*\{.*?WM_CANCELMODE\s*=>'
).Value
if ($mouseMoveBlock -notmatch 'taskbar_for_drag_point\(pt\)' -or
    $windowProduction -notmatch 'taskbar_at_point\(pt\)\.or_else\(current_drag_taskbar\)' -or
    $mouseMoveBlock -notmatch 'drag_left_for_pointer\(') {
    throw 'Outside-taskbar dragging must stay anchored to the current taskbar; real taskbar hits must still drive cross-monitor handoffs.'
}
$mouseUpBlock = [regex]::Match(
    $windowProduction,
    '(?s)WM_LBUTTONUP\s*=>\s*\{.*?updater::WM_APP_STARTUP_UPDATE_RESULT'
).Value
if ($mouseUpBlock -notmatch 'taskbar_for_drag_point\(pt\)' -or
    $mouseUpBlock -notmatch 'placement_for_free_spans\(' -or
    $mouseUpBlock -notmatch 'save_state_settings\(\)') {
    throw 'Releasing a drag over desktop must commit a safe position on the current taskbar.'
}
if ($mouseMoveBlock -match 'sync_blur_backdrop_zorder') {
    throw 'Drag frames must not reorder Composition backdrop/foreground windows.'
}
if ($windowProduction -match 'capture_taskbar_background' -or
    $windowProduction -match 'box_blur_bitmap' -or
    $windowProduction -match 'tint_frosted_panel_bitmap') {
    throw 'Taskbar screenshot/software blur must not be used for frosted glass.'
}
if ($styleWindow -notmatch '"磨砂强度"' -or
    $styleWindow -notmatch '"Frosted intensity"') {
    throw 'Unified frosted-intensity UI labels are missing.'
}
if ($windowProduction -notmatch 'reset_active\(s\.is_dark\)') {
    throw 'Reset Style must only reset the active theme.'
}
if ($windowProduction -notmatch 's\.styles\.active\(s\.is_dark\)') {
    throw 'Style menu/rendering must resolve the currently active theme.'
}


$settingsModel = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\settings_model.rs')
if ($styleWindow -notmatch 'WINDOW_WIDTH:\s*i32\s*=\s*980' -or
    $styleWindow -notmatch 'WS_THICKFRAME' -or
    $styleWindow -notmatch '"常规"' -or
    $styleWindow -notmatch '"JSON 配置"' -or
    $styleWindow -notmatch 'general_language_group_rect' -or
    $styleWindow -notmatch 'language_option_rect' -or
    $styleWindow -notmatch 'LanguageOption' -or
    $styleWindow -notmatch 'language_label_for_index' -or
    $styleWindow -match 'language_popup_open' -or
    $styleWindow -notmatch 'ID_EDIT_JSON') {
    throw 'Unified settings window must provide General, Appearance, JSON pages and inline two-choice language selection.'
}
# Title changes are synchronous Win32 messages. Explorer-hosted taskbar
# children can re-enter the app, so language state must be unlocked first.
$languageStateBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn apply_language_to_state\(.*?\n\}'
).Value
$titleRefreshBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn refresh_window_title\(\) \{.*?\n\}'
).Value
if ([string]::IsNullOrWhiteSpace($languageStateBlock) -or
    $languageStateBlock -match 'set_window_title\(' -or
    $languageStateBlock -match 'SetWindowTextW\(' -or
    [string]::IsNullOrWhiteSpace($titleRefreshBlock) -or
    $titleRefreshBlock -notmatch '(?s)let title = \{.*?lock_state\(\).*?\};\s*if let Some\(\(hwnd, strings\)\) = title \{\s*set_window_title\(') {
    throw 'Language state updates must release AppState lock before synchronous SetWindowTextW.'
}
foreach ($fragment in @(
    'apply_language_to_state(s, Some(language));',
    'apply_language_to_state(s, language_override);',
    'apply_language_to_state(app_state, None);'
)) {
    if (-not $windowProduction.Contains($fragment)) {
        throw "Missing language update path: $fragment"
    }
}
if ($windowProduction -notmatch '(?s)WM_SETTINGS_LANGUAGE_CHANGE\s*=>\s*\{.*?refresh_window_title\(\);.*?style_window::sync\(') {
    throw 'Settings language command must refresh title outside the app state lock before syncing UI.'
}

foreach ($message in @(
    'WM_SETTINGS_REFRESH_CHANGE',
    'WM_SETTINGS_USAGE_CHANGE',
    'WM_SETTINGS_ALERT_CHANGE',
    'WM_SETTINGS_STARTUP_CHANGE',
    'WM_SETTINGS_LANGUAGE_CHANGE',
    'WM_SETTINGS_JSON_APPLY'
)) {
    if ($styleWindow -notmatch $message -or $windowProduction -notmatch $message) {
        throw "Unified settings message is not connected end-to-end: $message"
    }
}
if ($styleWindow -match '"当前自定义"' -or
    $styleWindow -match '"Current custom"' -or
    $styleWindow -notmatch 'paint_style_preview_card\(' -or
    $styleWindow -notmatch 'if\s+!matched' -or
    $styleWindow -notmatch 'const\s+PRESET_CARD_WIDTH:\s*i32\s*=\s*172' -or
    $styleWindow -notmatch 'const\s+PRESET_CARD_HEIGHT:\s*i32\s*=\s*350' -or
    $styleWindow -notmatch 'const\s+PRESET_CARD_MINIMAL_HEIGHT:\s*i32\s*=\s*250' -or
    $styleWindow -notmatch 'const\s+PRESET_CARD_GAP:\s*i32\s*=\s*16' -or
    $styleWindow -notmatch 'preset_gallery_card_rect\(3, minimal\)' -or
    $styleWindow -notmatch 'portrait_preview_widget_bounds\(' -or
    $styleWindow -notmatch 'desktop_portrait_preview_bounds\(' -or
    $styleWindow -notmatch 'paint_horizontal_style_preview\(' -or
    $styleWindow -notmatch 'paint_preview_horizontal_minimal_text\(' -or
    $styleWindow -notmatch 'paint_preview_drag_tab\(' -or
    $styleWindow -notmatch 'preview_vertical_for_snapshot\(') {
    throw 'Preset gallery must support V23 compact heights, direction-aware previews, detached desktop drag tabs, and true horizontal minimal content.'
}
# Both thumbnail orientations must use the same sample text template
# as the production tooltip, with no time-only legacy preview strings.
$appearanceSource = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\appearance.rs')
if ($window -notmatch 'mod tooltip_measure;' -or
    $window -notmatch 'tooltip_measure::\{cached_minimal_tooltip_width, TOOLTIP_TEXT_INSET\}' -or
    $window -match 'static MINIMAL_TOOLTIP_WIDTH_CACHE:') {
    throw 'Tooltip measurement and cache must be owned by the dedicated window module.'
}
if ($appearanceSource -notmatch 'fn quota_reset_tooltip_text\(' -or
    $windowProduction -notmatch 'appearance::quota_reset_tooltip_text\(' -or
    $styleWindow -notmatch 'fn preview_reset_tooltip_lines\(' -or
    $styleWindow -notmatch 'quota_reset_tooltip_text\(' -or
    $styleWindow -notmatch 'let \(date, time\) = preview_reset_tooltip_lines\(minimal\)' -or
    $styleWindow -notmatch 'preset, "5H", "YYYY-MM-DD HH:mm"' -or
    $styleWindow -match 'preset, "5H", "20[0-9]{2}-[0-9]{2}-[0-9]{2}' -or
    $windowProduction -notmatch 'fn tooltip_width_with_render_slack\(' -or
    $windowProduction -notmatch 'TOOLTIP_TEXT_MEASUREMENT_SLACK' -or
    $windowProduction -notmatch 'fn tooltip_mask_content_width\(' -or
    $windowProduction -notmatch 'native_interop::directwrite_text_mask\(' -or
    $windowProduction -notmatch 'const TOOLTIP_TEXT_MEASUREMENT_SLACK: i32 = 3;' -or
    $styleWindow -match 'draw_text\(hdc, "21:30", tip,') {
    throw 'Every preview tooltip must use the real default/minimal reset-time text template, not a time-only placeholder.'
}
if ($settingsModel -notmatch 'serde\(deny_unknown_fields\)' -or
    $settingsModel -notmatch 'strip_jsonc_comments' -or
    $settingsModel -notmatch 'parse_jsonc' -or
    $settingsModel -notmatch 'show_usage.*cannot both be false' -or
    $settingsModel -notmatch 'expected #RRGGBB or #RRGGBBAA' -or
    $settingsModel -notmatch 'to_jsonc') {
    throw 'Advanced JSONC settings must use a strict public schema, safe comment parsing, and business validation.'
}
if ($styleWindow -match '编辑公开设置；注释仅用于说明' -or
    $styleWindow -match 'comments are documentation only and are omitted when applying or exporting' -or
    $styleWindow -notmatch 'top: scale\(hwnd, 108\)' -or
    $settingsModel -notmatch '注释仅用于说明，不属于配置数据；应用、保存和导出时不会写入配置') {
    throw 'JSON documentation notice must live inside generated JSONC, not as a separate page subtitle.'
}
if ($styleWindow -notmatch 'read_large_edit_text_raw' -or
    $styleWindow -notmatch 'EM_GETSCROLLPOS_MSG' -or
    $styleWindow -notmatch 'EM_SETSCROLLPOS_MSG' -or
    $styleWindow -notmatch 'previous_event_mask' -or
    $styleWindow -notmatch 'syntax_highlight_json_editor\(edit, &normalized, is_dark\)' -or
    $styleWindow -notmatch 'normalize_to_lf\(&read_large_edit_text_raw\(edit\)\)' -or
    $styleWindow -notmatch 'InvalidateRect\(edit, None, true\)' -or
    $styleWindow -notmatch 'UpdateWindow\(edit\)') {
    throw 'JSON RichEdit live highlighting must use RichEdit logical newline offsets, preserve selection/scroll state, suppress formatting notifications, and fully repaint after edits.'
}
if ($styleWindow -notmatch '● 有未保存更改' -or
    $styleWindow -notmatch '● Unsaved changes' -or
    $styleWindow -notmatch 'settings != &applied_settings' -or
    $styleWindow -notmatch 'json_action_enabled\(action, json_dirty, json_import_pending_apply\)' -or
    $styleWindow -notmatch 'let apply_background = if apply_enabled') {
    throw 'JSON editor must derive dirty state from parsed settings, show it on a separate line, and disable Apply while clean.'
}
if ($styleWindow -notmatch '✓ 已重新载入' -or
    $styleWindow -notmatch '✓ 已格式化' -or
    $styleWindow -notmatch '✓ 成功导入' -or
    $styleWindow -notmatch '✓ 成功导出' -or
    $styleWindow -notmatch 'JsonSaveFeedback::Saving' -or
    $styleWindow -notmatch 'JsonSaveFeedback::Saved' -or
    $styleWindow -notmatch '◌ 保存中' -or
    $styleWindow -notmatch '✓ 保存成功' -or
    $styleWindow -notmatch 'JsonStatusPath' -or
    $styleWindow -notmatch 'ShellExecuteW' -or
    $styleWindow -notmatch 'Comment\) => Color::from_hex\("#777777FF"\)' -or
    $styleWindow -notmatch 'Comment\) => Color::from_hex\("#999999FF"\)' -or
    $styleWindow -notmatch 'if zh \{ "应用" \} else \{ "Apply" \}') {
    throw 'JSON action feedback, second-line saving/saved state, clickable file paths, Apply label, and subdued comment colors must remain stable.'
}
if ($styleWindow -notmatch 'JSON_ACTION_DELAY_MS: u32 = 200' -or
    $styleWindow -notmatch '◌ 载入中' -or
    $styleWindow -notmatch '◌ 格式化中' -or
    $styleWindow -notmatch '◌ 导入中' -or
    $styleWindow -notmatch '◌ 导出中' -or
    $styleWindow -notmatch '◌ 保存中' -or
    $styleWindow -notmatch 'SetTimer\(hwnd, JSON_ACTION_TIMER_ID, JSON_ACTION_DELAY_MS' -or
    $styleWindow -notmatch 'pending_json_action' -or
    $styleWindow -notmatch 'json_apply_pending' -or
    $styleWindow -notmatch 'json_save_feedback = None' -or
    $styleWindow -notmatch 'GetTextExtentPoint32W' -or
    $styleWindow -notmatch 'path_left \+ path_width') {
    throw 'JSON actions including Apply must expose a 200ms running state; editing after save must restore unsaved state; file-link underline/hit area must match the rendered path width.'
}
if ($styleWindow -notmatch 'codex-usage-win-config-\{:\\04\}' -and
    $styleWindow -notmatch 'codex-usage-win-config-') {
    throw 'JSON export must provide a timestamped default filename.'
}
if ($styleWindow -notmatch 'GetOpenFileNameW' -or
    $styleWindow -notmatch 'GetSaveFileNameW' -or
    $styleWindow -notmatch 'RICHEDIT50W' -or
    $styleWindow -notmatch 'Msftedit\.dll' -or
    $styleWindow -notmatch 'to_windows_newlines' -or
    $styleWindow -notmatch 'syntax_highlight_json_editor' -or
    $styleWindow -notmatch 'friendly_json_error' -or
    $styleWindow -notmatch 'locate_json_error' -or
    $styleWindow -notmatch 'reload_json_editor_from_snapshot' -or
    $styleWindow -notmatch 'format_json_editor' -or
    $styleWindow -notmatch 'apply_json_editor') {
    throw 'JSON settings page must use RichEdit JSONC formatting/highlighting, friendly error navigation, reload, import/export, and validated apply.'
}
if ($windowProduction -notmatch 'theme::is_dark_mode\(\)' -or
    $windowProduction -notmatch 'PopupItem::submenu\(version_label' -or
    $popupMenu -notmatch '#1F1F1FFF' -or
    $popupMenu -notmatch '#FAFAFAFF' -or
    $popupMenu -notmatch 'draw_text' -and $popupMenu -notmatch 'DrawTextW' -or
    $popupMenu -match 'AppendMenuW') {
    throw 'Custom tray popup must follow Windows Dark/Light, draw its own single submenu arrow, and avoid native menu arrows.'
}
if ($fonts -notmatch 'Segoe UI' -or
    $fonts -notmatch 'Microsoft YaHei UI' -or
    $fonts -notmatch 'JetBrains Mono' -or
    $fonts -notmatch 'FontRole::Ui \| FontRole::Taskbar' -or
    $fonts -match 'Inter Variable' -or
    $fonts -match 'Noto Sans SC') {
    throw 'UI and taskbar must share the Windows-native font stack, with only JetBrains Mono embedded for monospaced text.'
}
if ($styleWindow -notmatch 'FontRole::Ui' -or
    $styleWindow -notmatch 'FontRole::Mono' -or
    $fonts -notmatch 'pub enum FontRole') {
    throw 'Settings and JSON editors must use the centralized semantic UI/mono font roles.'
}
if ($styleWindow -notmatch 'SWP_SHOWWINDOW' -or
    $styleWindow -notmatch 'SWP_HIDEWINDOW' -or
    $styleWindow -notmatch 'IsWindowVisible\(json_edit\)') {
    throw 'JSON editor visibility must follow the active settings section atomically and be covered by a UI regression test.'
}
if ($styleWindow -notmatch 'fill_rounded_rect' -or
    $styleWindow -notmatch 'draw_rounded_outline_rect' -or
    $styleWindow -notmatch 'draw_switch' -or
    $native -notmatch 'draw_antialiased_rounded_rect' -or
    $controlPrimitives -notmatch 'D2D1_ANTIALIAS_MODE_PER_PRIMITIVE' -or
    $controlPrimitives -notmatch 'FillRoundedRectangle' -or
    $controlPrimitives -notmatch 'DrawRoundedRectangle') {
    throw 'Settings controls must keep the shared Direct2D-antialiased Fluent-lite control primitives.'
}
$paintNavigationBlock = [regex]::Match(
    $styleWindow,
    '(?s)unsafe\s+fn\s+paint_navigation\s*\(.*?\n\}'
).Value
if ($paintNavigationBlock -notmatch 'Section::Tooltip' -or
    $paintNavigationBlock -notmatch 'navigation_icon\(item\)' -or
    $paintNavigationBlock -notmatch 'draw_settings_icon\(') {
    throw 'Settings sidebar must paint the Tooltip entry and its matching navigation icon.'
}
if ($styleWindow -notmatch 'enum\s+SettingsIcon' -or
    $styleWindow -notmatch '#\[repr\(i32\)\]' -or
    $styleWindow -notmatch 'SettingsIcon::General' -or
    $styleWindow -notmatch 'SettingsIcon::Preset' -or
    $styleWindow -notmatch 'SettingsIcon::Panel' -or
    $styleWindow -notmatch 'SettingsIcon::Tooltip' -or
    $styleWindow -notmatch 'SettingsIcon::Text' -or
    $styleWindow -notmatch 'SettingsIcon::Progress' -or
    $styleWindow -notmatch 'SettingsIcon::Interaction' -or
    $styleWindow -notmatch 'SettingsIcon::Json' -or
    $styleWindow -notmatch 'SettingsIcon::ThemeSystem' -or
    $styleWindow -notmatch 'SettingsIcon::ThemeDark' -or
    $styleWindow -notmatch 'SettingsIcon::ThemeLight' -or
    $styleWindow -notmatch 'SettingsIcon::LayoutDefault' -or
    $styleWindow -notmatch 'SettingsIcon::LayoutMinimal' -or
    $styleWindow -notmatch 'SettingsIcon::PresetClassic' -or
    $styleWindow -notmatch 'SettingsIcon::PresetOcean' -or
    $styleWindow -notmatch 'SettingsIcon::PresetForest' -or
    $styleWindow -notmatch 'SettingsIcon::PresetCustom' -or
    $styleWindow -notmatch 'SettingsIcon::PresetCloud' -or
    $styleWindow -notmatch 'SettingsIcon::PresetBay' -or
    $styleWindow -notmatch 'SettingsIcon::PresetWheat' -or
    $styleWindow -notmatch 'SettingsIcon::JsonReload' -or
    $styleWindow -notmatch 'SettingsIcon::JsonFormat' -or
    $styleWindow -notmatch 'SettingsIcon::JsonImport' -or
    $styleWindow -notmatch 'SettingsIcon::JsonExport' -or
    $styleWindow -notmatch 'SettingsIcon::JsonApply' -or
    $styleWindow -notmatch 'draw_segment_with_icon\(' -or
    $styleWindow -notmatch 'crate::svg_icons::draw\(' -or
    $styleWindow -notmatch 'theme_icon\(ThemeMode::System\)' -or
    $styleWindow -notmatch 'layout_icon\(AppearancePreset::Default\)' -or
    $styleWindow -notmatch 'preset_icon\(preset,\s*snapshot\.is_dark\)' -or
    $styleWindow -notmatch 'json_action_icon\(action\)' -or
    $styleWindow -notmatch 'json_action_icon\(apply\)') {
    throw 'Settings icon vocabulary must use the precise anti-aliased path renderer for navigation, Theme/Layout buttons, preset cards, and Custom.'
}
if ($styleWindow -notmatch 'fn draw_appearance_choice_group\(' -or
    $styleWindow -notmatch 'general_segment_rect\(bounds, index, choices\.len\(\)\)' -or
    $styleWindow -notmatch 'fn paint_submenu_collapse_switch\(' -or
    $styleWindow -notmatch 'fn desktop_auto_collapse_rect\(hwnd: HWND\)' -or
    $styleWindow -notmatch 'hdc, strings\.start_with_windows,') {
    throw 'Appearance controls must use joined icon choices, a single collapse switch and abbreviated Auto-start label.'
}
if ($styleWindow -notmatch 'title:\s*RECT' -or
    $styleWindow -notmatch 'if zh \{ "桌面模式" \} else \{ "Desktop mode" \}' -or
    $styleWindow -notmatch 'draw_antialiased_rounded_rect\(' -or
    $styleWindow -notmatch '0\.45 \+ 0\.55 \* submenu_content_alpha' -or
    $styleWindow -match 'foot:\s*RECT' -or
    $styleWindow -match 'draw_antialiased_desktop_inset_well\(') {
    throw 'Desktop controls must use a permanently visible, titled, independent right-side inset without the old L-shaped connection.'
}
$localeZh = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\localization\simplified_chinese.rs')
$localeEn = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\localization\english.rs')
if ($localeZh -notmatch 'start_with_windows:\s*"自启"' -or
    $localeEn -notmatch 'start_with_windows:\s*"Auto-start"') {
    throw 'Localized startup labels must be shortened without bypassing the Strings resource.'
}
if ($styleWindow -match 'snapshot\.language\.strings\(\)\.settings' -or
    $styleWindow -notmatch 'navigation_text_inset' -or
    $styleWindow -notmatch 'Section::General => \(28, 68\)' -or
    $styleWindow -notmatch 'Section::Tooltip => \(220, 260\)' -or
    $styleWindow -notmatch 'Section::Json => \(456, 496\)') {
    throw 'Settings sidebar must remain a compact grouped navigation without the redundant Settings heading.'
}
$sliderKindBlock = [regex]::Match(
    $styleWindow,
    '(?s)fn\s+slider_kind_at\s*\(.*?\n\}'
).Value
if ($sliderKindBlock -notmatch 'Section::Panel\s*\|\s*Section::Tooltip\s*\|\s*Section::Text\s*\|\s*Section::Progress\s*\|\s*Section::Interaction' -or
    $sliderKindBlock -notmatch 'return None;' -or
    $styleWindow -notmatch 'Section::General\s*\|\s*Section::Preset\s*\|\s*Section::Json\s*=>\s*&\[\]' -or
    $styleWindow -notmatch 's\.focused_numeric_edit\s*=\s*None' -or
    $styleWindow -notmatch 's\.focused_blur_edit\s*=\s*false' -or
    $styleWindow -notmatch 's\.focused_hex_edit\s*=\s*None' -or
    $styleWindow -notmatch 's\.pressed\s*=\s*None' -or
    $styleWindow -notmatch 's\.hovered\s*=\s*None') {
    throw 'Editorless settings pages must not inherit color-slider hit testing or stale editor interaction state.'
}
if ($styleWindow -notmatch 'if section == Section::Json' -or
    $styleWindow -notmatch 'reload_json_editor_from_snapshot\(\)' -or
    $styleWindow -notmatch 'refresh_json_editor_theme\(\)') {
    throw 'JSON editor must only refresh on the JSON section and remain untouched while preset/theme pages synchronize.'
}
$presetHitBlock = [regex]::Match(
    $styleWindow,
    '(?s)if\s+section\s*==\s*Section::Preset\s*\{.*?HitTarget::Preset\(preset\).*?\}'
).Value
if ([string]::IsNullOrWhiteSpace($presetHitBlock) -or
    $styleWindow -notmatch 'preset_card_rect\(hwnd, preset\)' -or
    $styleWindow -notmatch 'ThemePreset::ALL') {
    throw 'Preset page must keep dedicated hit targets for all three preset cards.'
}
if ($styleWindow -match 'Manage refresh, display, alerts, and application behavior' -or
    $styleWindow -notmatch 'settings_page_title_rect' -or
    $styleWindow -notmatch 'settings_card_rect' -or
    $styleWindow -notmatch 'settings_card_label_rect' -or
    $styleWindow -notmatch 'general_segment_rect' -or
    $styleWindow -notmatch 'draw_general_segment_group' -or
    $styleWindow -notmatch 'draw_general_usage_checkbox' -or
    $styleWindow -notmatch 'if zh \{ "常规" \} else \{ "General" \}' -or
    $styleWindow -notmatch 'general_refresh_group_rect\(hwnd\)' -or
    $styleWindow -notmatch 'general_alert_group_rect\(hwnd\)' -or
    $styleWindow -notmatch 'general_usage_card_rect\(hwnd\)' -or
    $styleWindow -notmatch 'general_application_card_rect\(hwnd\)') {
    throw 'Settings pages must keep the shared right pane and connected General choice groups plus half-width cards.'
}
if ($styleWindow -notmatch 'SETTINGS_EDITOR_HEIGHT: i32 = 164' -or
    $styleWindow -notmatch 'SETTINGS_EDITOR_CHANNEL_GAP: i32 = 30' -or
    $styleWindow -notmatch 'client\.bottom - scale\(hwnd, 24\)' -or
    $styleWindow -notmatch 'editor\.bottom = editor\.bottom\.min\(safe_bottom\)') {
    throw 'Appearance editors must keep compact shared channel spacing and a bottom safety margin.'
}
if ($styleWindow -notmatch 'WS_CAPTION\s*\|\s*WS_SYSMENU' -or
    $styleWindow -notmatch 'WM_CLOSE\s*=>\s*\{' -or
    $styleWindow -notmatch 'send_parent\(WM_STYLE_SAVE, 0, 0\)' -or
    $styleWindow -notmatch 'DestroyWindow\(hwnd\)') {
    throw 'Settings must use the native titlebar close button and save before closing.'
}
# The JSON editor, save overlay and toolbar hit zones now share one
# geometry module rather than relying on definitions in the parent file.
if ($styleWindowRoot -notmatch 'mod json_geometry;' -or
    $styleWindowRoot -notmatch 'use self::json_geometry::\*;' -or
    $jsonGeometry -notmatch 'pub\(super\) fn json_edit_rect\(' -or
    $jsonGeometry -notmatch 'pub\(super\) fn json_edit_text_rect\(' -or
    $jsonGeometry -notmatch 'pub\(super\) fn json_save_mask_rect\(' -or
    $jsonGeometry -notmatch 'pub\(super\) fn json_action_rect\(' -or
    $styleWindowRoot -match '(?m)^fn (?:json_edit_rect|json_edit_text_rect|json_save_mask_rect|json_action_rect)\(') {
    throw 'JSON editor geometry must stay shared after moving to json_geometry.rs.'
}
# Canceling the Windows Import/Export file picker must clear the temporary
# progress message without changing JSON contents or save/dirty state.
if ($jsonFileIo -notmatch '(?s)fn import_json_file\(hwnd: HWND\).*?let Some\(path\) = file_dialog\(hwnd, false, None\) else \{\s*//[^\n]*\n\s*//[^\n]*\n\s*set_json_status\(String::new\(\)\);\s*return;\s*\};' -or
    $jsonFileIo -notmatch '(?s)fn export_json_file\(hwnd: HWND\).*?let Some\(mut path\) = file_dialog\(hwnd, true, Some\(&default_name\)\) else \{\s*//[^\n]*\n\s*set_json_status\(String::new\(\)\);\s*return;\s*\};') {
    throw 'Cancelled JSON Import/Export must clear its in-progress status.'
}
if ($styleWindow -notmatch 'let edit = json_edit_rect\(hwnd\)' -or
    $styleWindow -notmatch 'let button_width = scale\(hwnd, 108\)' -or
    $styleWindow -notmatch 'JsonAction::Export\s*=>\s*RECT\s*\{[\s\S]{0,180}right:\s*edit\.right' -or
    $styleWindow -notmatch 'JsonAction::Import\s*=>\s*RECT\s*\{[\s\S]{0,220}right:\s*edit\.right - button_width - button_gap' -or
    $styleWindow -notmatch 'JsonAction::Apply\s*=>[\s\S]{0,260}let right = edit\.right') {
    throw 'JSON Import, Export, and Apply must share the JSON editor right edge and compact button width.'
}
if ($styleWindow -notmatch 'blur_row_index\(' -or
    $styleWindow -notmatch 'inline_numeric_frame_rect\(' -or
    $styleWindow -notmatch 'blur_edit_frame_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'blur_suffix_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'right: frame\.right - scale\(hwnd, 28\)') {
    throw 'Frosted-strength input must follow its actual settings row and keep percent as an internal fixed suffix.'
}
if ($styleWindow -match 'draw_outline_rect\(hdc, language_button') {
    throw 'Language selector must use one rounded outline rather than stacking a square outline over it.'
}


if ($styleWindow -notmatch 'load_embedded_app_icons\(' -or
    $styleWindow -notmatch 'hIcon:\s*large_icon' -or
    $styleWindow -notmatch 'hIconSm:\s*small_icon' -or
    $styleWindow -notmatch 'set_window_caption_icons\(' -or
    $windowProduction -notmatch 'set_window_caption_icons\(' -or
    $windowCommon -notmatch 'fn load_embedded_app_icons\(' -or
    $windowCommon -notmatch 'fn set_window_caption_icons\(' -or
    $windowCommon -notmatch 'WM_SETICON' -or
    $windowCommon -notmatch 'ICON_BIG' -or
    $windowCommon -notmatch 'ICON_SMALL' -or
    $windowCommon -notmatch 'unsafe impl Send for SendHwnd' -or
    $window -match 'struct SendHwnd\(' -or
    $styleWindow -match 'struct SendHwnd\(') {
    throw 'Both windows must reuse the shared Win32 handle and embedded executable caption icon helpers.'
}
if ($controlPrimitives -match 'codex_draw_desktop_inset_well' -or
    $controlPrimitives -match 'DesktopInsetCache' -or
    $controlPrimitives -match 'g_desktop_inset_cache') {
    throw 'The removed L-shaped desktop inset must not remain in native C++.'
}
if ($styleWindow -match 'WS_VSCROLL' -or
    $styleWindow -match 'ShowScrollBar' -or
    $styleWindow -notmatch 'EM_GETLINECOUNT_MSG' -or
    $styleWindow -notmatch 'EM_GETFIRSTVISIBLELINE_MSG' -or
    $styleWindow -notmatch 'EM_LINESCROLL_MSG' -or
    $styleWindow -notmatch 'json_scroll_line_metrics\(' -or
    $styleWindow -notmatch 'json_scroll_thumb_rect\(' -or
    $styleWindow -notmatch 'first_visible\.clamp\(' -or
    $styleWindow -notmatch 'json_scroll_hovered' -or
    $styleWindow -notmatch '#555B64FF' -or
    $styleWindow -notmatch '#777E88FF' -or
    $styleWindow -match 'DarkMode_Explorer' -or
    $styleWindow -match 'SetWindowTheme\(') {
    throw 'JSON editor must have no native scrollbar and its single custom thumb must follow the RichEdit visible line.'
}
if ($styleWindow -notmatch 'PendingDiscardAction' -or
    $styleWindow -notmatch 'paint_discard_dialog\(' -or
    $styleWindow -notmatch 'HitTarget::DiscardChanges' -or
    $styleWindow -notmatch 'HitTarget::KeepEditing' -or
    $styleWindow -notmatch 'request_discard_confirmation\(hwnd, PendingDiscardAction::SwitchSection\(section\)\)' -or
    $styleWindow -notmatch 'request_discard_confirmation\(hwnd, PendingDiscardAction::Close\)' -or
    $styleWindow -notmatch 'if zh \{ "未保存的更改" \} else \{ "Unsaved changes" \}' -or
    $styleWindow -notmatch '当前 JSON 配置尚未保存。继续操作将丢失这些更改。' -or
    $styleWindow -notmatch 'if zh \{ "丢弃更改" \} else \{ "Discard changes" \}' -or
    $styleWindow -notmatch 'if zh \{ "返回编辑" \} else \{ "Return to editing" \}' -or
    $styleWindow -notmatch '#B65F63FF' -or
    $styleWindow -match '#C93C49FF' -or
    $styleWindow -match 'MessageBoxW\(' -or
    $styleWindow -match 'MB_YESNO') {
    throw 'Unsaved JSON changes must use the revised warning copy, a muted red Discard changes action, and Return to editing.'
}
if ($styleWindow -notmatch 'redraw_settings_window\(' -or
    $styleWindow -notmatch 'RDW_INVALIDATE\s*\|\s*RDW_ERASE\s*\|\s*RDW_ALLCHILDREN\s*\|\s*RDW_UPDATENOW' -or
    $styleWindow -notmatch 'fill_rounded_rect\(hdc, r, section_bg' -or
    $styleWindow -notmatch 'fill_rounded_rect\([\s\S]{0,260}r\.right - scale\(hwnd, 202\)' -or
    $styleWindow -notmatch 'fn general_language_group_rect\(' -or
    $styleWindow -notmatch 'draw_general_segment_group\(' -or
    $styleWindow -notmatch 'draw_rounded_outline_rect\(hdc, bounds, separator, radius, 1\)') {
    throw 'Settings page switching must fully redraw, preserve rounded navigation and swatches, and keep an inline language group with a shared border.'
}
if ($styleWindow -notmatch 'i32::from\(style\.panel_corner_radius\)' -or
    $styleWindow -notmatch 'i32::from\(style\.tooltip_corner_radius\)' -or
    $styleWindow -notmatch 'i32::from\(style\.progress_corner_radius\)' -or
    $styleWindow -notmatch 'if tooltip_radius > 0' -or
    $styleWindow -notmatch 'if progress_radius > 0' -or
    $styleWindow -notmatch 'fill\(hdc, body, style\.color\(StyleColorTarget::PanelBackground\)\)' -or
    $styleWindow -notmatch 'fill\(hdc, widget, style\.color\(StyleColorTarget::PanelBackground\)\)' -or
    $styleWindow -notmatch 'draw_outline_rect\(hdc, widget, style\.color\(StyleColorTarget::PanelBorder\)\)') {
    throw 'Preset component previews must preserve their themed body, tooltip, progress radii and square-corner fallbacks.'
}
# The card outline is required, but the preview-positioning rectangle must
# never draw an additional frame around the grip, panel and tooltip.
$previewCardPainter = [regex]::Match(
    $presetGalleryPaint,
    '(?s)pub\(super\) unsafe fn paint_style_preview_card\(.*?\n\}'
).Value
if ([string]::IsNullOrWhiteSpace($previewCardPainter)) {
    throw 'Unable to isolate preset preview card painting for border regression verification.'
}
if ($previewCardPainter -notmatch 'draw_rounded_outline_rect\(\s*hdc,\s*r,' -or
    $previewCardPainter -match '(?:draw_rounded_outline_rect|draw_outline_rect|fill_rounded_rect|fill)\(\s*hdc,\s*preview\s*,') {
    throw 'Preset cards must keep their outer selection border but never paint a secondary enclosing preview frame.'
}
if ($settingsModel -notmatch 'panel_corner_radius:\s*u8' -or
    $settingsModel -notmatch 'tooltip_corner_radius:\s*u8' -or
    $settingsModel -notmatch 'progress_corner_radius:\s*u8' -or
    $settingsModel -notmatch 'panel_radius_max' -or
    $settingsModel -notmatch 'tooltip_radius_max' -or
    $settingsModel -notmatch 'progress_radius_max' -or
    $settingsModel -notmatch 'panel_corner_radius_max\(' -or
    $settingsModel -notmatch 'tooltip_corner_radius_max\(' -or
    $settingsModel -notmatch 'progress_corner_radius_max\(') {
    throw 'Numeric corner radii must round-trip through JSON/JSONC and validate against component-specific bounds.'
}

if ($styleWindow -notmatch "if s\.json_status\.starts_with\('×'\)" -or
    $styleWindow -notmatch 's\.json_status\.clear\(\)' -or
    $styleWindow -notmatch 's\.json_status_path = None') {
    throw 'JSON syntax/configuration errors must clear immediately after the edited JSON becomes valid.'
}

$drawSliderBlock = [regex]::Match(
    $styleWindow,
    '(?s)unsafe\s+fn\s+draw_slider\s*\(.*?\n\}'
).Value
if ($drawSliderBlock -match 'Ellipse\(' -or
    $drawSliderBlock -notmatch 'fill_rounded_rect\(hdc, track' -or
    $drawSliderBlock -notmatch 'fill_rounded_rect\(hdc, thumb, accent, radius\)') {
    throw 'Settings sliders must use anti-aliased rounded primitives instead of GDI Ellipse thumbs.'
}
if ($styleWindow -notmatch 'SETTINGS_EDIT_SUBCLASS_ID' -or
    $styleWindow -notmatch 'handle_settings_mouse_wheel\(' -or
    $styleWindow -notmatch 'adjust_numeric_by_wheel\(' -or
    $styleWindow -notmatch 'adjust_slider_by_wheel\(' -or
    $styleWindow -notmatch 'JSON_ERROR_SHAKE_STEPS:\s*u8\s*=\s*10' -or
    $styleWindow -notmatch 'scale\(hwnd, 4\)' -or
    $styleWindow -notmatch '"5H"' -or
    $styleWindow -notmatch '"7D"' -or
    $styleWindow -notmatch 'LanguageId::SELECTABLE\.len\(\)' -or
    $styleWindow -notmatch 'Some\(LanguageId::SimplifiedChinese\).*"中文"' -or
    $styleWindow -notmatch 'Some\(LanguageId::English\).*"English"') {
    throw 'Settings must support wheel adjustment, compact JSON-error shake, 5H/7D labels, and Chinese/English-only language selection.'
}

# Explicit desktop orientation must not make a synchronous Explorer shell
# query while processing the style-window message on the widget UI thread.
# SHAppBarMessage(ABM_GETTASKBARPOS) can synchronously wait for Explorer;
# Explorer can simultaneously be waiting for one of our parked taskbar-child
# HWNDs, freezing both the desktop widget and Windows taskbar.
$desktopBranch = [regex]::Match(
    $window, '(?s)fn\s+position_at_taskbar\(\)\s*\{(.*?)\n    let\s*\('
).Groups[1].Value
if ([string]::IsNullOrWhiteSpace($desktopBranch) -or
    $desktopBranch -notmatch 'display_position\s*==\s*DisplayPosition::Desktop' -or
    $desktopBranch -notmatch 'enable_floating_widget\("explicit desktop preference"' -or
    $desktopBranch -match '(?m)^\s*[^/\n]*get_taskbar_rect\s*\(') {
    throw 'Desktop orientation must return before any synchronous Explorer taskbar geometry query.'
}

# V24 keeps the grip dots fixed, moves the feedback to the entire
# themed widget perimeter, and highlights the separate desktop tab as well.
if ($windowProduction -match 'let\s+grip\s*=\s*RECT\s*\{\s*left:\s*sc\(3\),\s*top:\s*sc\(2\)' -or
    $windowProduction -notmatch 'fn\s+drag_handle_dot_style\(' -or
    $windowProduction -notmatch 'drag_handle_dot_style\(drag_color, hovered, false\)' -or
    $windowProduction -notmatch 'drag_handle_dot_style\(\*color, hovered, true\)' -or
    $windowProduction -notmatch 'let\s+\(dot, gap_x, gap_y, dots\)\s*=\s*drag_handle_dot_style' -or
    $windowProduction -notmatch 'fn\s+drag_hover_outline_color\(' -or
    $windowProduction -notmatch 'StyleColorTarget::ProgressHigh' -or
    $windowProduction -notmatch 'fn\s+draw_drag_hover_outer_glow\(' -or
    $windowProduction -notmatch 'fn\s+drag_hover_glow_coverage\(' -or
    $window -notmatch 'fn\s+drag_hover_signed_edge_distance\(' -or
    $window -notmatch 'drag_hover_glow_coverage\(distance\)' -or
    $windowProduction -notmatch 'if !floating && drag_hovered' -or
    $windowProduction -notmatch 'if hovered && !hidden' -or
    $windowProduction -notmatch 'floating_grip_rect\(' -or
    $windowProduction -notmatch 'draw_drag_hover_outer_glow\(\s*canvas, canvas_w, canvas_h, body, grip, &style, accent' -or
    $windowProduction -notmatch 'fn\s+panel_outer_shape_contains\(' -or
    $windowProduction -notmatch 'fn\s+draw_drag_hover_outer_glow\(' -or
    $windowProduction -notmatch 'drag_hover_outline_color\(&style\)') {
    throw 'V24 must retain stable six-dot grips and provide theme-adaptive perimeter hover on both the panel and desktop tab.'
}

# V24.6: halo pixels must never widen actual drag targets. The
# alpha-1 probe makes otherwise invisible gutter positions observable by
# WM_NCHITTEST; a deferred notification clears stale highlight safely.
if ($windowProduction -notmatch 'fn floating_hover_probe_rect\(' -or
    $windowProduction -notmatch 'fn paint_floating_hover_hit_probe\(' -or
    $windowProduction -notmatch 'pixels\[index\] = 0x01000000' -or
    $windowProduction -notmatch 'fn floating_drag_tab_hit_code\(' -or
    $windowProduction -notmatch 'WM_APP_DESKTOP_HOVER_EXIT' -or
    $windowProduction -notmatch 'PostMessageW\(' -or
    $windowProduction -notmatch 'if hovered == Some\(false\)' -or
    $window -notmatch 'v24_6_hit_probe_has_no_extra_drag_targets_in_any_orientation') {
    throw 'V24.6 must watch the real draggable rectangle via a transparent hit-test probe and deferred hover cleanup.'
}

# V24.19: glow starts beneath the REAL rounded border (not the inner
# content), with premultiplied source-over. An alpha-zero corner pixel must
# never punch out the backing glow.
if ($windowProduction -notmatch 'fn composite_premultiplied_pixel\(' -or
    $windowProduction -notmatch 'fn panel_inner_shape_contains\(' -or
    $windowProduction -notmatch 'glow\[index\] = glow\[index\]\.max\(drag_hover_glow_coverage\(0\.75\)\)' -or
    $windowProduction -notmatch 'canvas\[index\] = composite_premultiplied_pixel\(' -or
    $windowProduction -notmatch 'pixels\[index\] = composite_premultiplied_pixel\(' -or
    $window -notmatch 'v24_19_transparent_round_corners_do_not_erase_underlying_halo' -or
    $window -notmatch 'v24_19_border_underside_continuity_without_inner_tint' -or
    $window -notmatch 'v24_19_premultiplied_foreground_source_over_preserves_color_rules') {
    throw 'V24.19 must underpaint real rounded border, alpha-compose foreground and preserve content.'
}

# V24.7 retains the formerly clipped glow gutter on undocking.
# V24.18 renders a perimeter halo BEHIND the real rounded body and tab,
# sharing their exact exterior mask and never coloring transparent interiors.
if ($windowProduction -notmatch 'floating_drag_glow_detached' -or
    $windowProduction -notmatch 'fn floating_drag_canvas_geometry\(' -or
    $windowProduction -notmatch 'fn floating_drag_has_cleared_edge\(' -or
    $windowProduction -notmatch 'if should_unlock' -or
    $windowProduction -notmatch 'render_layered_at\(Some\(POINT \{ x: new_x, y: new_y \}\)\)' -or
    $windowProduction -notmatch 'fn draw_drag_hover_outer_glow\(' -or
    $windowProduction -notmatch 'fn panel_outer_shape_contains\(' -or
    $windowProduction -notmatch 'if occupied\[other\] \{ continue; \}' -or
    $windowProduction -notmatch 'if !hidden \{' -or
    $window -notmatch 'v24_7_drag_clears_physical_edge_and_restores_all_outer_gutters' -or
    $window -notmatch 'v24_18_outer_glow_leaves_all_panel_and_tab_pixels_untinted' -or
    $window -notmatch 'v24_18_outer_glow_contacts_actual_rounded_contour_without_any_gap' -or
    $window -notmatch 'v24_18_nearby_panel_and_tab_glows_do_not_tint_each_other') {
    throw 'V24.18 must retain undocking halo recovery while sharing the real rounded silhouette and avoiding interior tint.'
}

Write-Host 'PASS: v1.0.5 theme/style customization contract is satisfied.'


# Fixed-caption and layered-text regression contract
if ($styleWindow -match 'DWMWA_TRANSITIONS_FORCEDISABLED' -or
    $styleWindow -match 'DWMWA_USE_IMMERSIVE_DARK_MODE') {
    throw 'The fixed neutral style-window caption must not switch or animate with the active theme.'
}
if ($windowProduction -notmatch 'render_taskbar_text' -or
    $windowProduction -notmatch 'draw_directwrite_text' -or
    $windowProduction -notmatch 'directwrite_text_mask' -or
    $windowProduction -notmatch 'composite_premultiplied_text' -or
    $windowProduction -match 'text_quality_for_layered_surface' -or
    $windowProduction -match 'widget_text_quality\(\)' -or
    $windowProduction -match 'repair_taskbar_text' -or
    $windowProduction -match 'draw_layered_antialiased_text') {
    throw 'Taskbar text must use the final DirectWrite coverage path without obsolete rasterization experiments.'
}
