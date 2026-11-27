# Microsoft Store MSIX packaging

This directory contains the Microsoft Store packaging configuration for **Codex Usage Win**.

## Store identity

- Package/Identity/Name: `WalleStudio.CodexUsageWin`
- Package/Identity/Publisher: `CN=2AE5CECF-93C9-4AF7-82E0-23BE2024603E`
- PublisherDisplayName: `Walle Studio`
- Store ID: `9NGSJ2MXF6GS`
- Architecture: `x64`

The package version is derived from `Cargo.toml` and converted from `x.y.z` to `x.y.z.0` for Microsoft Store submission. The GitHub Actions validation and artifact name follow the derived version. Store submissions must use a higher package version than any package version already published.

## Build

On a Windows machine with the Windows 10/11 SDK and Python 3 installed:

```powershell
python -m pip install Pillow==11.3.0
cargo build --release --no-default-features --features store
.\packaging\build-msix.ps1 -OutputDir dist
```

The build script creates Store assets from `docs/assets/icon.png`, indexes qualified assets in a single `resources.pri`, and packages the release executable with `MakeAppx.exe`. The Search compatibility workaround restores all three 80px target-size variants and composites them onto opaque black RGB. Only the 40px unplated variant receives the verified Bold40 treatment (3x3 maximum filter blended at 35%, preserving the original alpha mask). The other 62 PNG resources are re-encoded losslessly as RGBA without pixel changes, reproducing the successful diagnostic pipeline. The 80px transparent variant with Bold40 failed on the tested Windows Search configuration, so it is not used. The underlying Windows cache mechanism is not established; validate repeated Search, taskbar and DPI behavior under the production package identity before release.

Do not use a default-feature `cargo build --release` executable for Store packaging: it includes the GitHub standalone updater. The MSIX script packages the existing executable without changing Cargo features.

## Artifacts

The GitHub Actions workflow produces:

- `codex-usage-win_<version>_x64.msix` — unsigned package for Microsoft Store submission.
- `codex-usage-win_<version>_x64-test-signed.msix` — self-signed package for local installation tests only.
- `codex-usage-win-test.cer` — temporary certificate for the local test package.
- `install-test.ps1` — helper for installing the test certificate and test package.
- `msix-icon-report.json` — pre/post resource hashes and pixel metadata; CI verifies the unpacked package against this report.

The self-signed test package is not intended for public distribution. Microsoft Store will sign the Store-distributed package after certification.
