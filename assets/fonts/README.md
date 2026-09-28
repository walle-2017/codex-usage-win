# Embedded fonts

The application UI and taskbar widget use the Windows-native system font stack:

- Segoe UI
- Microsoft YaHei UI fallback
- Arial final fallback

These system fonts are not bundled.

Only JetBrains Mono is embedded and privately registered for the current process. It is used by the JSON editor and other monospaced configuration text, with Consolas as the fallback.

License information for the embedded font is stored in `assets/fonts/licenses/`.
