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

The DPI-aware multi-monitor drag implementation must preserve the logical pointer anchor across taskbars. A normal cross-taskbar handoff switches between existing taskbar-specific child HWNDs, not by reparenting an active embedded HWND. Release and reacquire capture safely across a handoff, and release capture unconditionally on button-up. When the pointer leaves every taskbar while dragging, keep moving horizontally within the validated current taskbar, clamp to a safe position, and persist that position on release; only an actual hit on another taskbar may initiate a handoff.

An open Explorer Start menu must not obscure the embedded widget or the reset-time/date tooltip; the tooltip foreground and Composition backdrop share the current Explorer taskbar owner.

The unified settings window is a single reusable top-level window. Reopening it from the widget or tray menu must restore/raise the existing HWND after the popup menu closes without discarding pending edits, and must not make it permanently topmost.

Panel and tooltip Composition blur are independently active only when the configured frosted intensity is greater than zero **and** the respective background Alpha is below 255. An opaque background disables blur without resetting the stored intensity. The same predicate must govern rendering and frosted handoff logic.

## Stable multi-taskbar architecture

- The primary widget HWND is the sole service owner (poll timers, tray icon, updater and settings callbacks). Foreground replicas do not create extra pollers.
- Each taskbar receives its own child HWND when first visited; an existing embedded HWND must never be reparented to another Explorer taskbar.
- Frosted mode uses a taskbar-child DirectComposition backdrop behind the layered foreground; it must not create a separate topmost foreground popup.
- When switching taskbars, hide and park the previous blur HWND and Composition context under their original taskbar; restore the existing pair when returning. Create a new pair only on the first visit or after validation failure.
- Reject failed handoffs and roll back to the previous visible HWND and backdrop. Keep resource-count and memory safeguards, DPI-aware cursor anchors and the parked-replica fallback when Start temporarily hides taskbar enumeration.
- Normal startup and `--diagnose` must have identical behavior; the flag controls logging only. No `CODEX_TASKBAR_*` environment flags are supported.

## Source ownership

- `src/window.rs` owns the shared UI state, event dispatch, drag gestures, positioning and layered rendering.
- `src/window/taskbar_replica.rs` owns bounded taskbar-specific child window replicas, parent validation, the single service HWND and resource guards.
- `src/window/blur_backdrop.rs` owns DirectComposition Gaussian blur lifecycle, target validation and taskbar-specific parked-context reuse.
- These are Rust child modules, not runtime feature switches. A split must not change HWND parents, mouse capture or rendering order.
- Verified application baseline (2026-10-16): `9c00439` (`fix/settings-focus-frosted-alpha`), including module extraction, tooltip foreground ownership, outside-taskbar drag and settings/frosted-alpha fixes. Keep it as the rollback point during final cleanup.
- The historical pre-extraction cleanup baseline is `0c65724`; module-extraction and subsequent fixes have now passed Windows CI and manual regression.
- The final-cleanup work is on a separate branch. The baseline has not been merged into `main` yet.

## Verification

Run `cargo test`, `cargo clippy -- -D warnings`, and `cargo build --release` on Windows. The CI guards also cover permanent replica activation, blur cache invariants, and popup ownership.

Before merging, manually verify cross-taskbar round trips with Start open, horizontal drag while the cursor is outside the taskbar, tooltip visibility with Start open, existing settings window restore/foreground and edit preservation, Alpha 255 vs Alpha below 255 blur behavior, Explorer restart, sleep/resume, monitor reconfiguration and resource cleanup. Test both the default startup and the opt-in `--diagnose` mode.
