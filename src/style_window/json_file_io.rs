//! Windows JSON file pickers and configuration import/export.
//! On cancel, clear the transient Importing/Exporting status without changing edits.

use super::*;

pub(super) fn file_dialog(hwnd: HWND, save: bool, default_name: Option<&str>) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 4096];
    if let Some(name) = default_name {
        let encoded: Vec<u16> = name.encode_utf16().collect();
        let copy_len = encoded.len().min(buffer.len().saturating_sub(1));
        buffer[..copy_len].copy_from_slice(&encoded[..copy_len]);
    }
    let filter: Vec<u16> =
        "JSON/JSONC (*.json;*.jsonc)\0*.json;*.jsonc\0JSON (*.json)\0*.json\0All files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR::from_raw(filter.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        Flags: if save {
            OFN_OVERWRITEPROMPT
        } else {
            OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST
        },
        ..Default::default()
    };
    let accepted = unsafe {
        if save {
            GetSaveFileNameW(&mut ofn).as_bool()
        } else {
            GetOpenFileNameW(&mut ofn).as_bool()
        }
    };
    if !accepted {
        return None;
    }
    let len = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}

pub(super) fn import_json_file(hwnd: HWND) {
    let Some(path) = file_dialog(hwnd, false, None) else {
        // The dialog was dismissed without selecting a file. The pending
        // action has already ended; do not leave a stale "Importing" status.
        set_json_status(String::new());
        return;
    };
    let text = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(error) => {
            let language = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .map(|s| s.snapshot.language)
                    .unwrap_or(LanguageId::English)
            };
            set_json_status(if language == LanguageId::SimplifiedChinese {
                format!("× 无法读取导入文件\n{}：{error}", path.display())
            } else {
                format!("× Cannot read imported file\n{}: {error}", path.display())
            });
            return;
        }
    };
    let language = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state
            .as_ref()
            .map(|s| s.snapshot.language)
            .unwrap_or(LanguageId::English)
    };

    match parse_jsonc(&text) {
        Ok(settings) => {
            let dirty = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .is_some_and(|s| settings != s.snapshot.editable_settings)
            };
            let standard = settings.to_jsonc(language);
            let status = if language == LanguageId::SimplifiedChinese {
                "✓ 成功导入".to_string()
            } else {
                "✓ Imported successfully".to_string()
            };
            write_json_editor(&standard, status, dirty);
            set_json_status_with_path(
                if language == LanguageId::SimplifiedChinese {
                    "✓ 成功导入".to_string()
                } else {
                    "✓ Imported successfully".to_string()
                },
                path,
            );
        }
        Err(error) => {
            let status = json_action_error(
                "导入文件存在错误，可直接在编辑器中修复",
                "Imported file has errors; edit it directly below",
                &error,
                language,
            );
            write_json_editor(&text, status, true);
            let edit = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.json_edit.to_hwnd())
            };
            if let Some(edit) = edit {
                locate_json_error(edit, &error);
            }
        }
    }
}

pub(super) fn export_json_file(hwnd: HWND) {
    let (edit, language) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.snapshot.language)
    };
    let raw = read_large_edit_text(edit);
    let settings = match parse_jsonc(&raw) {
        Ok(value) => value,
        Err(error) => {
            set_json_status(json_action_error(
                "无法导出：配置存在错误",
                "Cannot export: configuration has errors",
                &error,
                language,
            ));
            locate_json_error(edit, &error);
            return;
        }
    };

    let default_name = export_default_filename();
    let Some(mut path) = file_dialog(hwnd, true, Some(&default_name)) else {
        // Discard the temporary "Exporting" status on file-dialog dismissal.
        set_json_status(String::new());
        return;
    };
    if path.extension().is_none() {
        path.set_extension("json");
    }
    let json = match settings.to_pretty_json() {
        Ok(value) => value,
        Err(error) => {
            set_json_status(if language == LanguageId::SimplifiedChinese {
                format!("× 无法导出\n{error}")
            } else {
                format!("× Cannot export\n{error}")
            });
            return;
        }
    };
    match fs::write(&path, json) {
        Ok(()) => {
            set_json_status_with_path(
                if language == LanguageId::SimplifiedChinese {
                    "✓ 成功导出".to_string()
                } else {
                    "✓ Exported successfully".to_string()
                },
                path,
            );
        }
        Err(error) => set_json_status(if language == LanguageId::SimplifiedChinese {
            format!("× 无法写入导出文件\n{}：{error}", path.display())
        } else {
            format!("× Cannot write exported file\n{}: {error}", path.display())
        }),
    }
}
