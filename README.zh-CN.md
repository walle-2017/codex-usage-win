![Windows](https://img.shields.io/badge/platform-Windows-blue)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

[English](README.md) | **简体中文**

<p align="center">
  <img src="docs/assets/icon.png" alt="Codex Usage Win 图标" width="112" height="112">
</p>

# Codex Usage Win

一款轻量级 Windows 原生 **Codex 用量监控工具**。当前正式版本：**v1.0.5**。

## 功能

- 显示 Codex 5 小时和 7 天剩余额度
- 显示重置时间/日期
- 紧凑、极简两种任务栏排版
- 支持跟随系统、强制深色、强制浅色主题
- 深浅主题分别保存 RGBA 配色和面板模糊，并支持实时预览
- 可配置低额度提醒
- 可配置刷新间隔
- 跟随 Windows 明/暗主题
- 支持多种界面语言
- 支持多显示器任务栏和 DPI-aware 拖动
- Explorer 重启恢复和单实例保护
- 支持 Windows 手动系统代理
- GitHub 独立版支持稳定 Release 更新发现
- Microsoft Store MSIX 版由 Store 管理更新

## 安全边界

程序读取 Codex 已维护的 `$CODEX_HOME/auth.json` 或 `~/.codex/auth.json`，**不会自动调用 Codex CLI 刷新 Token**，也不会把 Codex 凭据复制到发布者运营的服务器。

如果 Codex 返回 401/403，请通过官方 Codex CLI / 应用重新登录，再刷新或重启 Codex Usage Win。

详见 [隐私策略](PRIVACY.md) 和 [安全策略](SECURITY.md)。

## 安装

### GitHub Release

从 [最新 Release](https://github.com/walle-2017/codex-usage-win/releases/latest) 下载 `install.ps1`，执行：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
```

安装脚本校验 SHA256，并安装到：

```text
%LOCALAPPDATA%\Programs\CodexUsageWin
```

也可以直接运行便携版 `codex-usage-win.exe`。

### Microsoft Store

仓库包含 Microsoft Store 使用的 MSIX 构建。Store 版禁用 EXE 原地自更新，由 Microsoft Store 管理应用更新。

## 更新

GitHub 独立版每次启动只检查一次最新稳定 Release。存在新版本时，设置菜单版本项显示：

```text
v当前版本 --> v最新版本
```

点击后执行带 SHA256 校验和回滚的手动程序内更新；菜单同时提供 **GitHub Releases** 直接入口。

Microsoft Store 版不会执行 GitHub EXE 自更新。

## 诊断

普通启动不写诊断日志。使用 `--diagnose` 启动后，会在程序目录写入 `codex-usage-win.log`；超过 5 MB 后轮转为 `codex-usage-win.log.1`。

## 构建

```powershell
cargo build --release
```

Store MSIX 打包说明见 [packaging/README.md](packaging/README.md)。

## 文档

- [安装](docs/installation.md)
- [故障排查](docs/troubleshooting.md)
- [维护基线](docs/MAINTENANCE.md)
- [隐私策略](PRIVACY.md)
- [安全策略](SECURITY.md)
- [变更记录](CHANGELOG.md)

## 许可证

MIT，详见 [LICENSE](LICENSE)。
