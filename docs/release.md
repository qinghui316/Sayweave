# Windows 发布与更新

Sayweave 的源码开发使用 `main` 和短期功能分支；正式发布使用不可变的 `v<version>` tag。源码构建和安装包是同一提交的两种交付方式，不维护两套长期分支。

## 构建身份

普通源码及日常构建默认使用开发渠道，不接收公开自动更新。正式构建编译固定仓库、公钥、版本、完整提交和目标平台；隔离验收使用独立应用身份和临时签名，测试信任不能进入正式构建，运行时参数不能切换正式更新源。正式构建禁止 `desktop-acceptance` feature，编译时即拒绝混入验收能力。

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

发布从已验证的 `main` 提交创建 `v<version>` tag。Actions 核验同一完整 SHA 的成功主线 CI 和 Windows 更新机制证据，复用其中按哈希绑定的正式组件，在 `windows-signing` Environment 构建签名安装包并创建 Draft Release。Draft 资产会重新下载并校验，实际安装正式草稿包通过后，由 `windows-release` Environment 审核公开并标记 Latest。两个环境仅允许 `v*` 标签。

资产恰好为安装器、`.sig`、`latest.json`、`release-receipt.json` 和 `SHA256SUMS.txt`。校验清单必须覆盖前四份资产，缺项、重复、额外文件、越界路径、错误提交和外部下载 URL 均拒绝。客户端与发布验证器共用完整 minisign 签名及受签名版本校验。发布说明由结构化文件生成可读 Markdown，与应用内说明一致。

已发布的 tag 和 Release 资产不覆盖。发现问题时撤回 Latest，并从新的更高 patch 版本发布修复，不自动降级用户。

`v0.1.3`、`v0.1.5` 和 `v0.1.6` 因缺陷撤为草稿，保留原标签和资产。已安装这些版本的用户需手动覆盖安装修复版；后续版本使用应用内更新。0.1.7 的正式发行结果以公开端点及独立验收记录为准。

## 应用内更新

原生 `UpdateService` 统一拥有检查、传输、缓存、验签及安装；页面卸载只释放订阅。前端展示原生修订快照，安装绑定任务 ID 和修订，旧按钮不能安装另一份包。侧栏入口和关于页使用同一状态。

正式版启动20秒后空闲检查，24小时至多自动检查一次。检查限时15秒、下载5分钟；元数据64KiB、安装包128MiB上限。保留 TLS 和 HTTPS 重定向校验，前台口述可取消传输。缓存只持有一份候选，下载后和安装前分别核验大小、哈希、签名与版本。

安装先复用模型／词条表单的保存确认，再暂停新的即时设置提交并排空队列（5秒）。原生层建立准入屏障，等待已接收的保存及归档（10秒）。当前口述未结束或可撤销准备失败时释放屏障，保留程序与填写内容。

进入停止阶段后，依次停止快捷键、语音、口述、原生观察，完成归档、后台学习与模型结算，最后封闭并排空 DataWorker。已接收的学习证据先落库；停止时排空剩余证据，不等待新的模型分析，存储失败阻止安装。停止限时5秒；超时或失败不启动安装，也不恢复部分停止的服务，界面提供重启当前版本。

原生适配器以参数数组启动已验证 NSIS 包，确认子进程创建后才退出。原子更新记录只保存包身份和阶段，不含正文。新进程实际运行目标版本才确认更新完成；重启旧版不算成功，不自动重复安装。上次未完成的安装可重试，损坏缓存需重新下载。

## 真实 Windows 验收

验收分为工具、更新机制和正式安装包三个独立阶段。先运行 `node scripts/test-windows-update.mjs --tools-only`：从精确提交归档分别在 `component-target` 和 `probe-target` 构建正式 DLL 与启用 `acceptance` 的探针，验证 `capabilities` 及 `installed-fixture`。短检查在一次性机器临时注册正式 TIP，以真实服务身份加载明确指定的 DLL，检查实际 EDIT 文字、协议8、故障注入导出缺失及 COM 最终卸载。注册前检查 HKCU/HKLM 的本组件 COM/TIP 和 TSF 资料均为空；收尾只移除路径与所有者匹配的本次注册，确认两处注册及资料均已移除后才通过。TSF 注册 API 可能修改机器元数据，不能称为只写 HKCU。短检查失败时不会导入证书、签名或构建安装包。

通过后使用 `--tools-dir <工具目录>` 运行机制验收，构建两份隔离版本。每次打包前后重新核对工具哈希，包内组件只来自正式 DLL 目录。所有清理成功后才生成可复用的 `windows-update-evidence`，绑定完整提交、工具哈希、成功机制记录和有界合成数据；不上传签名私钥、证书私钥或凭据值。

正式验收使用 `--stable-installer <草稿回下载包> --mechanism-evidence <同SHA成功证据目录>`，不重建或重跑两份隔离版本。发布流程先严格验签和核对资产身份，再实际安装正式草稿包，覆盖实际旧版并核对原生界面、稳定渠道、完整提交、数据及组件身份。不同候选的工具或机制证据不能互用。

每次执行保存阶段、失败阶段、退出码、信号、超时与工具身份。探针 stdout/stderr 和无密钥的组件／探针 Cargo 构建日志分别有界保存到 `diagnostics` 并上传；涉及签名、证书和模型的完整日志只留在一次性环境的私有 `build.log`，不进入公开验收资产。进程标准流关闭后才结算命令结果。

`node scripts/test-windows-update.mjs` 仅允许已提权的一次性 Windows runner 或明确标记的一次性虚拟机，不能在日常机器仅新建账户后运行。脚本先核验管理员权限，不发起交互提权；合成 HTTPS 根证书有界导入该虚拟机的 LocalMachine/Root，以精确指纹核验并在收尾移除。它从同一候选构建两个隔离版本，经真实 HTTPS、临时签名、NSIS、自动重启核对凭据值与数据。安装启动后的中断由测试主动终止已确认身份的 NSIS 进程树注入，验证旧版重开、资料保持及不自动重试；这与安装器自行报错退出分别记录。测试恢复原安装目录 ACL 后，通过界面再次安装。

旧公开版覆盖检查保留原 SQLite 文件、迁移账本及旧程序创建的模型、默认项、凭据引用和词条，仅按冻结结构追加有界的合成历史／偏好来源。旧程序先读取这些资料，覆盖后再核对内容哈希；已保存密钥通过真实 HTTPS 模型测试验证，报告只保存匹配结果。输入组件在一次性账户实际安装，以注册路径、DLL 哈希和原生输入探针核对保持。所有实际安装包，包括下载的旧版和外部传入的正式草稿包，都记录哈希。正式发行再次回下载并实际安装正式草稿包。不要在日常账户设置一次性标志。

记录绑定具体提交、安装器哈希、环境与实际检查项；模拟 UI、原生窗口和真实安装分别留证。数据库009、TSF8及已有资料保持；日常机器不执行此安装验收。

首发不配置 Windows Authenticode，因此 Windows 可能显示未知发布者；updater 产物签名仍然是强制的信任边界。
