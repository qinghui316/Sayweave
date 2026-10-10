# Windows 发布与更新

Sayweave 的源码开发使用 `main` 和短期功能分支；正式发布使用不可变的 `v<version>` tag。源码构建和安装包是同一提交的两种交付方式，不维护两套长期分支。

## 构建身份

普通源码及日常构建默认使用开发渠道，不接收公开自动更新。正式构建编译固定仓库、公钥、版本、完整提交和目标平台；隔离验收使用独立应用身份和临时签名，测试信任不能进入正式构建，运行时参数不能切换正式更新源。

正式构建设置 `SAYWEAVE_BUILD_CHANNEL=stable`、`SAYWEAVE_BUILD_COMMIT=<完整提交SHA>`、`SAYWEAVE_RELEASE_COMMIT=<同一SHA>` 和 `SAYWEAVE_RELEASE_TAG=v<版本>`。标签、构建 HEAD 与预期提交必须一致，该提交必须属于已验收的 `origin/main`。package、Cargo、Tauri 和锁文件版本一致，工作区干净。

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

发布从已验证的 `main` 提交创建 `v<version>` tag。Actions 先执行完整检查，再在 `windows-signing` Environment 构建签名安装包并创建 Draft Release。Draft 资产会重新下载并校验，实际安装正式草稿包通过后，由 `windows-release` Environment 审核公开并标记 Latest。两个环境仅允许 `v*` 标签。

资产恰好为安装器、`.sig`、`latest.json`、`release-receipt.json` 和 `SHA256SUMS.txt`。校验清单必须覆盖前四份资产，缺项、重复、额外文件、越界路径、错误提交和外部下载 URL 均拒绝。客户端与发布验证器共用完整 minisign 签名及受签名版本校验。发布说明由结构化文件生成可读 Markdown，与应用内说明一致。

已发布的 tag 和 Release 资产不覆盖。发现问题时撤回 Latest，并从新的更高 patch 版本发布修复，不自动降级用户。

`v0.1.3`、`v0.1.5` 和 `v0.1.6` 因缺陷撤为草稿，保留原标签和资产。已安装这些版本的用户需手动覆盖安装修复版；后续版本使用应用内更新。0.1.7 的正式发行结果以公开端点及独立验收记录为准。

## 应用内更新

原生 `UpdateService` 统一拥有检查、传输、缓存、验签及安装；页面卸载只释放订阅。前端展示原生修订快照，安装绑定任务 ID 和修订，旧按钮不能安装另一份包。侧栏入口和关于页使用同一状态。

正式版启动20秒后空闲检查，24小时至多自动检查一次。检查限时15秒、下载5分钟；元数据64KiB、安装包128MiB上限。保留 TLS 和 HTTPS 重定向校验，前台口述可取消传输。缓存只持有一份候选，下载后和安装前分别核验大小、哈希、签名与版本。

安装先复用模型／词条表单的保存确认，再暂停新的即时设置提交并排空队列（5秒）。原生层建立准入屏障，等待已接收的保存及归档（10秒）。当前口述未结束或可撤销准备失败时释放屏障，保留程序与填写内容。

进入停止阶段后，依次停止快捷键、语音、口述、原生观察，完成归档、后台学习与模型结算，最后封闭并排空 DataWorker。停止限时5秒；超时或失败不启动安装，也不恢复部分停止的服务，界面提供重启当前版本。

原生适配器以参数数组启动已验证 NSIS 包，确认子进程创建后才退出。原子更新记录只保存包身份和阶段，不含正文。新进程实际运行目标版本才确认更新完成；重启旧版不算成功，不自动重复安装。上次未完成的安装可重试，损坏缓存需重新下载。

## 真实 Windows 验收

`node scripts/test-windows-update.mjs` 仅允许一次性 Windows runner 或明确标记的一次性账户。它从同一候选构建两个隔离版本，经真实 HTTPS、临时签名、NSIS、自动重启核对凭据值与数据，另验证旧公开版手动覆盖及中断／停止／启动失败。正式发行再次回下载并实际安装正式草稿包。不要在日常账户设置一次性标志。

记录绑定具体提交、安装器哈希、环境与实际检查项；模拟 UI、原生窗口和真实安装分别留证。数据库009、TSF8及已有资料保持；本轮不重装输入组件。

首发不配置 Windows Authenticode，因此 Windows 可能显示未知发布者；updater 产物签名仍然是强制的信任边界。
