# 发布目录 · Release

**源码只有一套**(`axiom3-desktop/` + `crates/`),Tauri 跨平台编译。
本目录只放**各操作系统的发布产物与配置**,让每个正式版的 Windows / macOS 包各自独立、互不干扰。

- `windows/` — Windows 发布:`tauri.windows.conf.json` 覆盖配置、图标、打好的 `.msi`/`.exe`、发布说明、更新清单
- `macos/`   — macOS 发布:`tauri.macos.conf.json` 覆盖配置、图标、打好的 `.dmg`/`.app`、发布说明、更新清单

## 版本号规则
- 本地测试/增量更新:走**末位**,如 `1.1.0 → 1.1.1 → 1.1.2`
- 正式版发布:进**中间位**,如 `1.1.x → 1.2.0`
- 三处同步:`tauri.conf.json` + `src-tauri/Cargo.toml` + `package.json`
