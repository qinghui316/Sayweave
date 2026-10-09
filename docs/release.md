# Windows 发布与更新

Sayweave 的源码开发使用 `main` 和短期功能分支；正式发布使用不可变的 `v<version>` tag。源码构建和安装包是同一提交的两种交付方式，不维护两套长期分支。

## 本地发布检查

```powershell
npm ci
npm run check
npm run test:e2e
npm run check:distribution
npm run desktop:package
npm run desktop:release:manifest
npm run desktop:release:verify
```

发布构建需要 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。私钥只放在受保护的 GitHub Environment 或本机受控目录，不能提交到仓库。

`desktop:package` 生成 Windows x64 NSIS 安装程序和 Tauri updater 签名。`desktop:release:manifest` 生成 GitHub Releases 使用的 `latest.json`、校验清单和 release receipt；`desktop:release:verify` 会重新核对版本、tag、下载地址、签名、安装包大小和 SHA-512。

## GitHub 发布

发布从已验证的 `main` 提交创建 `v<version>` tag。Actions 先执行完整检查，再在 `windows-signing` Environment 构建签名安装包并创建 Draft Release。Draft 资产会重新下载并校验；`windows-release` Environment 审核通过后才标记为 Latest。

已发布的 tag 和 Release 资产不覆盖。发现问题时撤回 Latest，并从新的更高 patch 版本发布修复，不自动降级用户。

## 应用内更新

稳定安装版从 GitHub Releases 的 `latest.json` 检查更新。更新下载在后台进行，设置/关于底部的圆形控件显示真实进度；下载完成后由用户点击重启更新。录音、文字整理、原生输入或归档未结束时不会安装。

首个公开安装包是 bootstrap。后续 patch 版本验证从已安装的首个版本升级，验证历史、模型凭据、词典、数据库、TSF 组件和窗口状态保持。

首发不配置 Windows Authenticode，因此 Windows 可能显示未知发布者；updater 产物签名仍然是强制的信任边界。
