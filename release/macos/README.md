# macOS 发布

每个正式版的 macOS 安装包(.dmg/.app)、平台配置与发布说明放这里。

## 打包(在 macOS 上)
```
cd axiom3-desktop && npm run tauri build -- --bundles dmg
# 产物:target/release/bundle/dmg/*.dmg → 复制到 release/macos/<版本号>/
```
