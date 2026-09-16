![Windows](https://img.shields.io/badge/platform-Windows-blue)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**English** | [简体中文](README.zh-CN.md)

<p align="center">
  <img src="docs/assets/icon.png" alt="Codex Usage Win icon" width="112" height="112">
</p>

# Codex Usage Win

A lightweight native Windows taskbar monitor for **Codex usage**. Current release: **v1.0.5**.

## Features

- Codex 5-hour and 7-day remaining quota
- Reset time/date display
- Default and minimal taskbar layouts
- Follow-system, forced dark, and forced light themes
- Per-theme live style editing for RGBA colors and 0–100% Windows Composition Gaussian backdrop blur
- Optional low-quota alerts
- Configurable refresh interval
- Windows light/dark theme support
- Multiple UI languages
- Multi-monitor taskbar placement and DPI-aware dragging
- Explorer restart recovery and single-instance protection
- Windows manual system-proxy support
- GitHub Release update discovery for the standalone build
- Microsoft Store MSIX build with Store-managed updates

## Safety

Codex Usage Win reads the credentials already maintained by Codex from `$CODEX_HOME/auth.json` or `~/.codex/auth.json`. It does **not** launch the Codex CLI to refresh credentials and does not copy Codex credentials to a publisher-operated backend.

If Codex returns 401/403, sign in again using the official Codex CLI/app and then refresh or restart Codex Usage Win.

See [PRIVACY.md](PRIVACY.md) and [SECURITY.md](SECURITY.md).

## Install

### GitHub Release

Download `install.ps1` from the [latest release](https://github.com/walle-2017/codex-usage-win/releases/latest), then run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
```

The installer verifies SHA256 and installs to:

```text
%LOCALAPPDATA%\Programs\CodexUsageWin
```

Portable use is also supported with `codex-usage-win.exe`.

### Microsoft Store

The repository contains an MSIX build for Microsoft Store distribution. Store builds disable executable self-replacement and rely on Microsoft Store for package updates.

## Updates

The standalone GitHub build checks the latest stable release once at startup. When a newer version exists, the Settings version row becomes:

```text
vCURRENT --> vLATEST
```

Clicking it performs the manual in-app update with SHA256 verification and rollback. The menu also provides a direct **GitHub Releases** link.

The Microsoft Store build does not perform GitHub executable self-updates.

## Diagnostics

Normal launches do not write diagnostic logs. Start with `--diagnose` to enable `codex-usage-win.log` beside the executable; it rotates to `codex-usage-win.log.1` after 5 MB.

## Build

```powershell
cargo build --release
```

Store MSIX packaging is documented in [packaging/README.md](packaging/README.md).

## Documentation

- [Installation](docs/installation.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Maintenance baseline](docs/MAINTENANCE.md)
- [Privacy Policy](PRIVACY.md)
- [Security Policy](SECURITY.md)
- [Changelog](CHANGELOG.md)

## License

MIT. See [LICENSE](LICENSE).
