# Windows 发布

每个正式版的 Windows 安装包(.msi/.exe)、平台配置与发布说明放这里。

## 打包(在 Windows PC 上)
```
cd axiom3-desktop && npm run tauri build
# 产物:target/release/bundle/msi/*.msi → 复制到 release/windows/<版本号>/
```
