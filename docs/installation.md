# Codex Usage Win installation model

Codex Usage Win is a single native Windows executable. Installation places the executable and uninstall helper in a stable per-user directory; it does not add a runtime, service, driver, telemetry component, or machine-wide dependency.

## Direct installation

- Install directory: `%LOCALAPPDATA%\Programs\CodexUsageWin`
- Executable: `%LOCALAPPDATA%\Programs\CodexUsageWin\codex-usage-win.exe`
- Uninstall helper: `%LOCALAPPDATA%\Programs\CodexUsage\uninstall.ps1`
- Start menu shortcut: `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Codex Usage Win.lnk`
- Desktop shortcut: `Codex Usage Win.lnk`
- Add/Remove Programs key: `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexUsageWin`

The installer is per-user and does not request elevation. It verifies the release SHA256 before replacing an existing executable. Replacement uses a staged `.new` file and preserves the previous executable as `.old` until the new installation has completed successfully.

Online installation downloads release assets only from the fork repository:

```text
walle-2017/codex-usage-win
```

The installer expects `codex-usage-win.exe`, `codex-usage-win.exe.sha256`, and `uninstall.ps1` from the same Release.

## Portable mode

`codex-usage-win.exe` can be run directly from any user-writable directory. Portable and installed copies use the same settings file:

```text
%APPDATA%\CodexUsage\settings.json
```

Both installed and portable copies perform one check-only lookup of the latest stable Release from `walle-2017/codex-usage-win` after startup. If a higher version exists, Codex Usage Win notifies the user and the **Settings** version row displays `vCURRENT --> vLATEST`. No file is downloaded by the startup check. Clicking the version row starts the manual updater, which downloads `codex-usage-win.exe` plus `codex-usage-win.exe.sha256`, verifies SHA256, stages the replacement beside the running executable, and restarts automatically.

The updater does not request UAC elevation. If the executable directory is not writable, the running version is left unchanged. It never downloads or executes the Release `install.ps1` as part of an in-app update.

## Settings and startup behavior

- Upgrades preserve `%APPDATA%\CodexUsage\settings.json`.
- Normal uninstall preserves settings so a later reinstall restores preferences.
- `uninstall.ps1 -RemoveSettings` explicitly deletes the settings directory.
- The installer does not enable startup automatically.
- If Start with Windows was already enabled, reinstall/upgrade preserves that choice and updates the registry entry to the stable installed executable.
- Users can enable or disable Start with Windows from the application's settings menu.
- Uninstall removes the `CodexUsage` startup entry because the executable no longer exists.

Current settings include taskbar position/screen, polling frequency, language, appearance, visible 5h/7d rows, quota-alert threshold, and alert de-duplication state.

## Release integrity

The release executable is accompanied by `codex-usage-win.exe.sha256`. The PowerShell installer and in-app updater both verify the staged executable against the Release checksum before replacement.

The executable's Windows FileVersion/ProductVersion and the application's version menu are derived from the Cargo package version.

## In-app update behavior

Latest-version discovery runs once after application startup and again when the user clicks the version row. The startup pass is check-only and silent when current or failed; it only notifies when a higher stable version is found. There is no periodic background update check. Discovery uses the fork's GitHub `releases/latest` redirect and strictly accepts only this repository's stable numeric `vX.Y.Z` tag path.

Replacement uses a generated local one-shot PowerShell helper. The helper waits for the old process to exit, preserves the old executable as `.old`, installs the verified `.new` file, rolls back on replacement/relaunch failure, and restarts the new executable with a one-shot internal success argument. That argument is consumed by the new process and does not create a persistent marker file in the program directory.
