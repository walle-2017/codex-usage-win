# Maintenance Baseline

This document records the maintained product constraints for **Codex Usage Win**.

## Current baseline

- Current release: `v1.0.5`
- Repository: `walle-2017/codex-usage-win`
- Product name: `Codex Usage Win`
- Standalone executable: `codex-usage-win.exe`
- Runtime scope: Codex-only
- Settings compatibility directory: `%APPDATA%\CodexUsage`

## Security constraints

- Never restore automatic Codex CLI token refresh.
- Codex credentials are read from the existing Codex credential file only.
- 401/403 pauses normal polling until credentials change.
- Update discovery is pinned to this repository's stable GitHub Releases.
- Standalone update assets require SHA256 verification before replacement.
- Diagnostic logging is opt-in via `--diagnose`.

## Update channels

### GitHub standalone

The default Cargo feature is `github-update`. It supports startup Release discovery and manual executable replacement.

A renamed/missing Release asset after a newer version has been discovered must fall back to a manual-update notice and the GitHub Releases menu instead of surfacing raw HTTP 404.

### Microsoft Store

Store packages are built with:

```text
--no-default-features --features store
```

Store builds must not initiate GitHub executable self-updates. Package updates are managed by Microsoft Store.

## Compatibility

v1.0.3 intentionally keeps the legacy product name, icon, and `codex-usage.exe` filename, while its rewritten release uses the current repository URL and supports the manual-update migration notice.

v1.0.4 is the renamed Codex Usage Win baseline with the new icon and complete current functionality.

## UI invariants

The DPI-aware multi-monitor drag implementation must preserve the logical pointer anchor across taskbars. Mouse capture must be released before reparenting, restored after the switch, and released unconditionally on button-up.

## Stable multi-taskbar architecture

- The primary widget HWND is the sole service owner (poll timers, tray icon, updater and settings callbacks). Foreground replicas do not create extra pollers.
- Each taskbar receives its own child HWND when first visited; an existing embedded HWND must never be reparented to another Explorer taskbar.
- Frosted mode uses a taskbar-child DirectComposition backdrop behind the layered foreground; it must not create a separate topmost foreground popup.
- When switching taskbars, hide and park the previous blur HWND and Composition context under their original taskbar; restore the existing pair when returning. Create a new pair only on the first visit or after validation failure.
- Reject failed handoffs and roll back to the previous visible HWND and backdrop. Keep resource-count and memory safeguards, DPI-aware cursor anchors and the parked-replica fallback when Start temporarily hides taskbar enumeration.
- Normal startup and `--diagnose` must have identical behavior; the flag controls logging only. No `CODEX_TASKBAR_*` environment flags are supported.

## Verification

Run `cargo test`, `cargo clippy -- -D warnings`, and `cargo build --release` on Windows. Before merging, manually verify cross-taskbar round trips with Start open, settings/style changes, Explorer restart, sleep/resume, monitor reconfiguration and resource cleanup.
