# Windows 发布与更新

Sayweave 的源码开发使用 `main` 和短期功能分支；正式发布使用不可变的 `v<version>` tag。源码构建和安装包是同一提交的两种交付方式，不维护两套长期分支。

日常工作从本地 `main` 新建短期开发分支，本地验证和独立审查后按 Harness 合入本地 `main`，再直接 `git push origin main`。开发与审查候选默认只在本地保存；已完成的远端临时分支在主线同步和内容保留核对后清理。具体约定见 [开发说明](development.md)。

## 构建身份

普通源码及日常构建默认使用开发渠道，不接收公开自动更新。正式构建编译固定仓库、公钥、版本、完整提交和目标平台；隔离验收使用独立应用身份和临时签名，测试信任不能进入正式构建，运行时参数不能切换正式更新源。正式构建禁止 `desktop-acceptance` feature，编译时即拒绝混入验收能力。

正式构建设置 `SAYWEAVE_BUILD_CHANNEL=stable`、`SAYWEAVE_BUILD_COMMIT=<完整提交SHA>`、`SAYWEAVE_RELEASE_COMMIT=<同一SHA>` 和 `SAYWEAVE_RELEASE_TAG=v<版本>`。标签、构建 HEAD 与预期提交必须一致，该提交必须属于已验收的 `origin/main`。package、Cargo、Tauri 和锁文件版本一致，工作区干净。

## 本地发布检查

检查分层及更新专项的授权范围见 [本地 Harness Verify 工作流](../.agents/skills/beaver-4a20fc43577a-harness/references/workflows/verify.md)。包构建、清单和签名工具仍使用现有 desktop:package、desktop:release:manifest 和 desktop:release:verify 入口。

发布构建需要 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。私钥只放在受保护的 GitHub Environment 或本机受控目录，不能提交到仓库。

`desktop:package` 生成 Windows x64 NSIS 安装程序和 Tauri updater 签名。`desktop:release:manifest` 生成 GitHub Releases 使用的 `latest.json`、校验清单和 release receipt；`desktop:release:verify` 会重新核对版本、tag、下载地址、签名、安装包大小和 SHA-512。它直接调用已记录的 `verify-update-signature.exe`，不再通过 `cargo run` 编译应用。首次使用先在同一干净正式源码及 stable 环境中执行 `node scripts/verify-update-release.mjs build-tool`，后续复用 `SAYWEAVE_VERIFIER_DIR`（默认 `release/verifier`）中的工具及回执。

## GitHub 发布

面向 `main` 的独立 CI ruleset 要求 GitHub Actions 的 `CI` 检查通过，并为仓库所有者保留日常直推例外。现有对管理员生效的线性历史、禁止强推和禁止删除保护继续执行。所有者例外仅用于主线直推；外部 PR 合入前仍须等待 CI 通过。正式 `v*` 标签触发独立发布流程，`v*-release-check-*` 诊断标签仍从自动发布中排除。

日常源码合入与同步按已有 Harness 执行。当前普通 CI 由 PR 或明确手动请求触发，main 推送不运行普通 CI。正式标签流程核对源码，先核对 owner 已创建的精确草稿及发布权限，再构建签名包；包保全后上传至该草稿，回下载校验资产身份与完整签名。windows-signing 和 windows-release 两处环境授权保持；后者由所有者在人工试用后审核。默认发布不执行实际安装；已有 accept-existing 模式仅用于明确请求的固定草稿安装专项。

资产恰好为安装器、`.sig`、`latest.json`、`release-receipt.json` 和 `SHA256SUMS.txt`。校验清单必须覆盖前四份资产，缺项、重复、额外文件、越界路径、错误提交和外部下载 URL 均拒绝。客户端与发布验证器共用完整 minisign 签名及受签名版本校验。发布说明由结构化文件生成可读 Markdown，与应用内说明一致。

冻结标签首次构建被过期的界面测试阻断时，可在同一 Windows release 工作流显式选择 `build-frozen`，提供原产品标签和完整提交，并从已独立审查、经 I2 合入的匹配 `v<version>-release-check-*` 控制标签运行。控制提交与产品提交均须属于 `origin/main`；两者差异仅允许列明的两个 E2E 文件、发行工作流、发行验证脚本及测试和本文。产品代码或其他文件有差异时拒绝续发。完整项目检查仍在原产品执行，E2E 使用有哈希来源记录的两个已审测试文件，并在成功或失败后恢复原产品源码。签名构建使用干净的原产品 checkout，资产继续绑定原不可变标签与提交；五项资产、完整签名和 `windows-release` 人工审核保持。

已发布的 tag 和 Release 资产不覆盖。发现问题时撤回 Latest，并从新的更高 patch 版本发布修复，不自动降级用户。

草稿由现有 owner 发布授权预先创建；`build-frozen` 传入精确 `release_id`，普通标签构建可按标签发现唯一草稿。CI 用具备 contents:write 的发布 job 在构建前读取并核对同一标签、完整产品提交、未公开状态及空资产，缺少权限或已有保全包时拒绝构建。GitHub 对相对默认分支修改工作流的目标提交要求 workflow 权限，CI 的 GITHUB_TOKEN 不支持此权限；owner 创建草稿解决这一边界，不把 owner 凭据交给 CI。

### Cargo 与验签工具复用

同一 release 控制标签中的检查、签名构建及失败续跑复用 Cargo 热缓存。注册表缓存只含公开 crates.io 的 index/cache/src；编译缓存只含主程序与输入组件 target 中分别位于 debug/release 的 deps、build、.fingerprint、incremental。不缓存 Cargo 凭据或配置、用户目录、.local、录音、数据库、安装包及整个工程。

编译兼容前缀绑定实际 rustc/cargo、Windows x64 MSVC、runner 镜像、Cargo 配置及构建 flags/profile；精确键追加两个 Cargo.lock、原生源码与构建渠道身份。工具链、平台或编译配置变化隔离缓存；锁文件或源码变化可恢复兼容依赖，但仍由 Cargo 指纹决定重编译。主程序在包装前用 metadata --locked 核对锁文件，组件和验签工具用 build --locked；包装沿用冻结产品原入口，完成后强制回读干净源码及锁文件，不能把解析结果变化当缓存命中跳过。控制提交 SHA 本身不影响键。缓存保存安排在 Release API 失败也能收尾的位置，且不清空本地热缓存。GitHub 缓存按 ref 与默认分支隔离：不同发布标签之间不保证命中，不为预热新建普通 main CI；实际命中、恢复键及耗时记录于必要发布日志。[GitHub 缓存边界](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)

先准备普通组件和工具需要的现有前端资源，只编译一次现有完整 Rust/minisign 验证器，使用同一 release 依赖缓存。工具及独立版本化回执作为 `release-verifier-<tag>-<完整产品提交>` CI artifact 保存十四天。工具保全及来源检查成功后，才允许构建签名包，避免留下有包却没有验签工具的恢复死路。同 run 的失败 job 重跑发现已保全工具时直接下载核对，不再次编译或覆盖 artifact。回执绑定原产品提交、原生 Git 源码身份、工具链、平台、profile、features、构建配置、可信 workflow 生产 run 与工具大小/SHA-256。后续上传前、草稿回下载、资产检查及发行审核阶段复用该工具；来源、配置、哈希或资产异常时拒绝执行，不退回冷编译。工具及回执不进入正式五项 Release 资产。

### 包保全与同包续发

签名包及清单生成后立即保全为 `release-package-<tag>-<完整产品提交>` artifact，保存十四天；构建失败的现有 bundle 诊断保留七天。保全失败阻止 Release 操作。包已存在时，任何上传、权限、回下载或公开失败都继续使用这些原始字节，不重新打包。预构建只读权限/草稿查询发生在包产生前；包产生后的 Release 读写均排在持久保全之后。完整签名检查仍在上传前及每次回下载执行。

`resume-package` 提供原 `release_tag`、`release_commit`、`release_id`、`checksum_sha256`、`package_run_id` 和有效 `verification_run_id`，从已审、合入的匹配诊断控制标签运行。生产 run 必须属于本仓库本 release 工作流，确实成功构建并保全包/工具及完整验签，且控制源码在主线上；查询该 run 全部 attempts，后来的失败重跑不能遮掉原生产证据。两份 artifact 必须存在且未过期。已有保全包时，包括同 run 的失败 job 重跑和 artifact 过期，都不自动重新打包；过期资料需找回原资产。恢复模式不安装依赖、编译程序、构建工具或重新打包，只核对来源、完整签名及所有资产后续传缺项。已经上传的每份资产须名称、大小和 GitHub SHA-256 digest 完全匹配；重复、外来、损坏、半上传或已公开资产拒绝处理，绝不 clobber。

本次 `v0.1.9` 原产品 `784e43172780255e83d5b652507c7bec666ff859` 的 run `38251026083` 已完成签名及完整验签，但保全缺失、runner 已结束、artifact 为零、owner 草稿为空。其包确实丢失，完整五项流水线修复审查/I2 后允许替代构建一次；后续重试复用这份替代包。历史同版本或其他提交包不充当该候选。

明确指定 `verification_run_id` 时，只复用同仓库、已完成、准确诊断控制标签的成功 verify job；原产品、本地及远端控制提交必须匹配，完整 check、已审全套 E2E 和分发步骤都须成功，两个已审 E2E 文件的 Git 字节必须相同。任一事实变化或缺失均拒绝复用；未指定时仍执行现有完整检查。后续创建草稿或权限失败不会改写此前独立通过的产品检查，也不触发仅因控制 SHA 变化的重复验收。

`v0.1.3`、`v0.1.5` 和 `v0.1.6` 因缺陷撤为草稿，保留原标签和资产。`v0.1.7` 标签保持不变，其发行任务在新 runner 的证据目录准备阶段失败，未生成正式安装包或公开发行。修复后的首个可用版本接续为0.1.8，正式结果以公开端点及独立验收记录为准。已安装旧版的用户需手动覆盖安装修复版；后续版本使用应用内更新。

## 应用内更新

原生 `UpdateService` 统一拥有检查、传输、缓存、验签及安装；页面卸载只释放订阅。前端展示原生修订快照，安装绑定任务 ID 和修订，旧按钮不能安装另一份包。侧栏入口和关于页使用同一状态。

正式版启动20秒后空闲检查，24小时至多自动检查一次。检查限时15秒、下载5分钟；元数据64KiB、安装包128MiB上限。保留 TLS 和 HTTPS 重定向校验，前台口述可取消传输。缓存只持有一份候选，下载后和安装前分别核验大小、哈希、签名与版本。

安装先复用模型／词条表单的保存确认，再暂停新的即时设置提交并排空队列（5秒）。原生层建立准入屏障，等待已接收的保存及归档（10秒）。当前口述未结束或可撤销准备失败时释放屏障，保留程序与填写内容。

进入停止阶段后，依次停止快捷键、语音、口述、原生观察，完成归档、后台学习与模型结算，最后封闭并排空 DataWorker。已接收的学习证据先落库；停止时排空剩余证据，不等待新的模型分析，存储失败阻止安装。停止限时5秒；超时或失败不启动安装，也不恢复部分停止的服务，界面提供重启当前版本。

原生适配器以参数数组启动已验证 NSIS 包，确认子进程创建后才退出。原子更新记录只保存包身份和阶段，不含正文。新进程实际运行目标版本才确认更新完成；重启旧版不算成功，不自动重复安装。上次未完成的安装可重试，损坏缓存需重新下载。

## 更新专项工具与历史记录

专项的授权、停止条件及范围以 [本地 Harness Verify 工作流](../.agents/skills/beaver-4a20fc43577a-harness/references/workflows/verify.md) 为准。现有 Windows 工具支持 tooling、startup、full、diagnose 和 accept-existing 等历史模式；其结果分别保留实际提交、包、组件、账户与执行范围。旧的模拟、安装后启动、完整机制、草稿安装和公开发行记录保持各自身份，不能互相代替，也不能把未执行场景写成通过。

工具仅用于原先约定的一次性 Windows 环境，不能通过在日常账户设置一次性标志来绕过边界。真实用户数据库、凭据、录音、学习资料及共享组件不进入临时清理范围。正式资产保持不可变。

首发不配置 Windows Authenticode，因此 Windows 可能显示未知发布者；updater 产物签名仍然是强制的信任边界。

## 版本递增

日常修复、本地开发程序重建及公开仓库源码推送沿用当前开发版本，以提交、构建来源和程序／组件哈希区分。正式 Release 才使用新的不可变版本标签；已发布版本不复用。本轮开发版本保持 0.1.9，下一次正式发行使用 v0.1.9；发布后准备下一正式版本时再统一递增元数据。

## 0.1.9 安装事务与入口

0.1.9 准备新的品牌、独立个性化页面、固定关于/更新状态位，以及安装器自动部署组件。发现新版自动后台下载，验签后由用户确认重启并更新。正常空结果静默收起。整体配色继续沿用当前设计。

首次安装默认启用，覆盖升级保留启停选择。安装取消、中断或必要维护失败先恢复旧程序及已拥有登记；不能确认时保留待恢复状态和备份。全机组件是共享资源，真正卸载仅删除已核实属于该安装的程序与元数据，用户资料、共享登记/DLL 与启停选择保留。

本段描述新候选的发布契约，不代表安装验收、正式标签或公开发行已经完成。0.1.8 标签与资产保持不可变；新正式版本仍需精确候选的有效证据、独立审查、I2、本地 main 同步，以及草稿回下载/验签、人工试用和发行环境授权。未覆盖的实际更新场景保持待人工试用。
