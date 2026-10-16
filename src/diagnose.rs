use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_FILE_NAME: &str = "codex-usage-win.log";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

struct LogWriter {
    path: PathBuf,
    file: Option<File>,
}

struct DiagnoseState {
    writer: Mutex<LogWriter>,
}

static DIAGNOSE_STATE: OnceLock<DiagnoseState> = OnceLock::new();

pub fn init() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Unable to resolve executable path for logging: {e}"))?;
    let directory = exe
        .parent()
        .ok_or_else(|| "Executable has no parent directory for logging".to_string())?;
    let path = directory.join(LOG_FILE_NAME);
    rotate_if_oversized(&path)?;
    let file = open_log(&path)?;
    let _ = DIAGNOSE_STATE.set(DiagnoseState {
        writer: Mutex::new(LogWriter {
            path: path.clone(),
            file: Some(file),
        }),
    });
    log("runtime logging enabled");
    Ok(path)
}

fn open_log(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("Unable to open runtime log file {}: {e}", path.display()))
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_file_name("codex-usage-win.log.1")
}

fn rotate_if_oversized(path: &Path) -> Result<(), String> {
    let oversized = fs::metadata(path)
        .map(|metadata| metadata.len() >= MAX_LOG_BYTES)
        .unwrap_or(false);
    if !oversized {
        return Ok(());
    }
    let backup = backup_path(path);
    let _ = fs::remove_file(&backup);
    fs::rename(path, &backup).map_err(|e| {
        format!(
            "Unable to rotate runtime log {} to {}: {e}",
            path.display(),
            backup.display()
        )
    })
}

fn rotate_locked(writer: &mut LogWriter) -> Result<(), String> {
    let oversized = fs::metadata(&writer.path)
        .map(|metadata| metadata.len() >= MAX_LOG_BYTES)
        .unwrap_or(false);
    if !oversized {
        return Ok(());
    }
    if let Some(file) = writer.file.take() {
        let _ = file.sync_all();
        drop(file);
    }
    rotate_if_oversized(&writer.path)?;
    writer.file = Some(open_log(&writer.path)?);
    Ok(())
}

pub fn is_enabled() -> bool {
    DIAGNOSE_STATE.get().is_some()
}

/// Avoid allocating diagnostic messages when --diagnose is not active.
/// The closure runs synchronously and only for an enabled log writer.
pub fn log_lazy(build: impl FnOnce() -> String) {
    if is_enabled() {
        log(build());
    }
}

pub fn log(message: impl AsRef<str>) {
    let Some(state) = DIAGNOSE_STATE.get() else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    if let Ok(mut writer) = state.writer.lock() {
        if rotate_locked(&mut writer).is_err() && writer.file.is_none() {
            writer.file = open_log(&writer.path).ok();
        }
        if let Some(file) = writer.file.as_mut() {
            let _ = writeln!(file, "[{timestamp}] {}", message.as_ref());
            let _ = file.flush();
        }
    }
}

pub fn log_error(context: &str, error: impl std::fmt::Display) {
    log(format!("{context}: {error}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_name_is_single_generation() {
        assert_eq!(
            backup_path(Path::new(r"C:\Tools\codex-usage-win.log")),
            PathBuf::from(r"C:\Tools\codex-usage-win.log.1")
        );
    }

    #[test]
    fn rotation_threshold_is_five_megabytes() {
        assert_eq!(MAX_LOG_BYTES, 5_242_880);
    }
}
