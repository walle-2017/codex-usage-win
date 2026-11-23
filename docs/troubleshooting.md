# Troubleshooting Codex Usage Win

## Taskbar error labels

Codex Usage Win separates authentication failures from transient service failures:

| Simplified Chinese | Other languages | Meaning | Recommended action |
|---|---|---|---|
| `!` | `!` | Codex credentials are missing or expired | Sign in with the official Codex CLI/app, then refresh or restart Codex Usage Win |
| `网络` | `NET` | Network or TLS connection failed | Check connectivity, VPN, proxy, and firewall settings |
| `限流` | `429` | Codex usage endpoint rate limit | Wait for the retry window; Codex Usage Win retries with backoff |
| `服务` | `5XX` | Service failure | Wait and retry; check service status if the problem persists |
| `错误` | `ERR` | Invalid or unsupported response | Inspect the runtime log next to the executable |

When the usage endpoint returns `401` or `403`, authentication polling is paused until the local Codex credential source changes. The monitor does not launch Codex CLI processes to refresh credentials.

Transient failures use backoff up to the configured refresh interval.

## Authentication recovery

If the taskbar shows `!`:

1. Open the official Codex CLI or Codex app yourself.
2. Complete sign-in if required.
3. Confirm that the local Codex credentials have been refreshed.
4. Refresh or restart Codex Usage Win.

The monitor reads `$CODEX_HOME/auth.json` or `~/.codex/auth.json` but does not directly edit it.

## Runtime log

Runtime logging is disabled during normal startup. Start the executable with `--diagnose` to create or append `codex-usage-win.log` in the same directory as the running `codex-usage-win.exe`.

The log is append-only across normal restarts and records key events such as:

- application version and executable path
- polling success/failure category and retry behavior
- taskbar placement and Explorer-recovery events
- whether Windows manual system proxy support was selected
- in-app update checks, GitHub/HTTP/network/TLS failures, selected Release version, download/checksum status, and updater launch status

When `codex-usage-win.log` reaches 5 MB, it is rotated to `codex-usage-win.log.1`. Only the current file and one previous file are retained.

The log does not include access tokens, refresh tokens, credential-file contents, API response bodies, or proxy passwords.

If the executable directory is read-only, failure to create the log does not prevent the application from starting.

## Network and proxy failures

First check whether explicit proxy variables are set:

```powershell
Get-ChildItem Env:HTTPS_PROXY,Env:HTTP_PROXY,Env:ALL_PROXY -ErrorAction SilentlyContinue
```

If none are set, Codex Usage Win can use the current user's Windows manual proxy from:

```text
HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings
```

The application reads `ProxyEnable` and `ProxyServer` without changing the registry. PAC/WPAD automatic discovery is not supported.

If a proxy is used, it must be trusted because the Codex usage request carries an OAuth Bearer token inside the TLS connection.

## Reset local position without deleting settings

Right-click the taskbar component or tray icon and choose **Reset Position** from the context menu. Settings are stored at:

```text
%APPDATA%\CodexUsage\settings.json
```

## Reinstall while preserving settings

Normal uninstall keeps the settings file. Reinstalling restores saved taskbar position, refresh interval, language, appearance, visible 5h/7d rows, and quota-alert preferences.

Use:

```powershell
uninstall.ps1 -RemoveSettings
```

only when a full settings reset is intended.

## Explorer/taskbar restart recovery

The application keeps a watchdog for Explorer/taskbar replacement. If Explorer restarts and destroys the embedded taskbar window, Codex Usage Win relaunches a fresh instance and uses a single-instance mutex to hand over cleanly.

If the widget does not return after Explorer has stabilized, exit any remaining `codex-usage-win.exe` process and start the application again.

## In-app update

Click the version item under **Settings** to check the latest stable Release from `walle-2017/codex-usage-win`. When a newer version is available, the application downloads `codex-usage-win.exe` and `codex-usage-win.exe.sha256`, verifies SHA256, replaces the installed or portable executable safely, and restarts without an extra confirmation prompt.

A check-only update lookup runs once after startup. It is silent when the current version is latest or the lookup fails, and only notifies when a higher stable version is found. There is no periodic background update check.

## In-app update fails

The running version is left unchanged when the Release check, download, SHA256 verification, target-directory write preflight, or updater launch fails. Manual update failures include a concise technical reason. For the full diagnostic sequence, restart Codex Usage Win with `--diagnose`; normal startup does not write `codex-usage-win.log`.

Ensure GitHub is reachable through the same proxy environment used by Codex Usage Win and that the directory containing the running executable is writable. The updater does not request UAC elevation.
