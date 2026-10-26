# 测试与覆盖边界

持续草稿分支的 `processing/draft`、`dictation/drafts` 回放和 `data/archive_recovery` 回归覆盖同块重整、编辑基线、容量收尾、撤销许可与兼容恢复。`SAYWEAVE_BLOCK_FIXTURE=1` 下运行验收构建的 `tsf-probe fixture`，只操作自有 RichEdit，证明范围、正文、租约与回调事实；不会修改机器登记。普通构建不包含该夹具或故障入口。`live_revisable_draft_comparison` 是显式忽略的真实供应商对照，仅读取日常选定配置，以合成语料记录真实用量和请求耗时，不证明麦克风或目标显示时间。分支契约见 [持续草稿与编辑接管](revisable-draft.md)。

安装环境见 [开发说明](development.md)。测试在项目根目录执行，不依赖个人启动器或私有工具安装脚本。

编辑恢复回归复用 `scripts/test-prefix-input.mjs` 的 `SAYWEAVE_BLOCK_REGRESSION=1`、`SAYWEAVE_BLOCK_ACTOR_ONLY=1` 生产链路及当前已构建的 `SAYWEAVE_ACTOR_TEST_EXE`。`SAYWEAVE_BLOCK_BACKSPACE=1` 直接在活动组合中删除，不先用方向键结束组合；可叠加 `SAYWEAVE_BLOCK_EARLY_EDIT=1` 覆盖客户端尚未处理跟踪回执的时序。`SAYWEAVE_BLOCK_PRESENTATION=1` 检查音频路由与已发布录音状态，`SAYWEAVE_BLOCK_USER_NEWLINE=1` 保留编辑器支持的段落换行，`SAYWEAVE_BLOCK_SENTINELS=1` 核对块外正文。三次编辑均检查宿主事件处理后的实际正文。普通 paragraph 中用 `insertText` 塞入裸 LF 会被该夹具自身解析为空格，不能用它冒充段落换行；原失败和不经 TSF 的对照证据仍保留。

原生入口检查另覆盖父属性缺失、空值、错误类型和不支持的接口；Direct 不因父能力后来出现而升级，已选 Parent 丢失后不降级到新范围。同文档焦点丢失仍粘性撤销跟踪。启动诊断按任务／轮次／操作绑定，附加元数据不改变只读或组词等待的业务分类。

人工编辑时，可在同一 `richtext_actor` 入口显式启用 `SAYWEAVE_ACTOR_PHYSICAL_INPUT=1`，复用桌面的键盘、鼠标和焦点事件所有者；测试结束释放监听。按实际编辑事件及按键释放设置续写屏障，状态输出包含活动序号和按键持有状态。单次注入 `edited` 只覆盖那次通知，不能代替后续真实按键；固定等待超过既有15秒期限的退出须与早期接管失败分开。焦点回调当时的文档／窗口匹配事实随既有写锁内首拒结构输出；无法取得后续编辑锁时，这部分覆盖仍未完成。元数据不授予写入许可。

实际宿主正文可在同一入口叠加 `SAYWEAVE_ACTOR_BODY_READBACK=1`。先在目标输入框放入 `测试：`，测试在30秒内等待其稳定，然后保留该聚焦可编辑 UIA 元素；`body` 命令读取该字段自己的 TextPattern 范围，范围所属内层节点必须沿父链归属该字段，结束回执另附 `actualBody`。换字段、密码／只读、范围属于其他字段、前缀不符或超过1024个 UTF-16 单元均拒绝读取。UIA 超时单独返回读取错误，不能算作正文通过。原生／模型回执与这份宿主正文分别记录，不读取会话根文档，不借正文读取授予生产写入许可。`input-fixture` 的 `fixture-text` 与原有 `fixture-key` 使用同一个验收标记；自动改字需要让文本和方向／删除键都进入原编辑监听，Unicode 注入仍与真实输入法选词分开结算。

`SAYWEAVE_BLOCK_INTERIOR_EDIT=1` 在现有生产富文本 actor 中保留首次局部改字后的块内光标，直接发送下一段已确认识别；整理后再次通过原生按键交还组合，断言尾部正文完整，再继续删除和第三次修改。该模式使用普通已绑定块，不与首写屏障或首笔 Backspace 合并，覆盖接管后的写入光标与下一次组合提交之间的往返。

同一入口的 `SAYWEAVE_BLOCK_CONTROL=controlled-input` 使用受控 React 输入框，默认 `richtext` 保留富文本。每次宿主事件稳定后同时断言原生可见值与 React 状态，收尾后再次核对。后续程序化局部编辑与直接 Backspace 分别记录，不将前者视为真实输入法测试。普通原生 EDIT 的持续父文档验证使用既有 `input-fixture` 和同一 SessionDraft actor，核对三轮修改、删除后续说、块外标记及同窗同正文的换框反例；临时载体清空与持久正文保留分别观察。载体读回调超期后不得再执行捕获的父锁操作，原生效果等级和实际正文分别留证。

同一识别连接的 PCM 暂停／恢复由 `retained_edit_resume_sends_pcm_on_the_same_connection_without_ready_event` 验证；界面真实电平订阅由 `tests/voice-edit-resume.test.tsx` 验证。受控音频、宿主正文、用户实际 Codex 试用分别记录，原生 `submitted/unconfirmed` 不升级为 `confirmed`。本次 f2ab7319 候选的同一 Codex 三步文字与波形验收已通过，不能扩展成所有宿主和时序的保证。

## 常规检查

本地识别分支增加 `node scripts/check-local-asr.mjs`：复用原 target，检查固定工作程序、私有有界协议和隔离管道替身的轮次／取消契约，不下载模型或访问麦克风。真实模型测试使用主程序的 `speech::local::tests::real_worker_through_main_recognition_adapter` 显式入口；`SAYWEAVE_LOCAL_DEVICE`、`SAYWEAVE_LOCAL_EXPECT_BACKEND`、`SAYWEAVE_LOCAL_AUDIO`、`SAYWEAVE_LOCAL_ROUNDS` 和唯一 `SAYWEAVE_LOCAL_REPORT` 分别固定设备、素材、重复次数及报告。报告保留实际转写、时序、进程、源码载荷和驱动身份；不能覆盖首个失败或将合成音频视为麦克风试用。

下载用本机 HTTP 夹具验证断点范围、ETag、完整哈希、取消和文件租约；schema10→11 与旧配置／凭据／默认选择在独立资料内验证。新包必须显式要求完整本地运行载荷，旧冻结安装包按其原清单验证，不运行额外安装故障专项。

已通过的验证在源码、构建身份与覆盖范围未变化时直接复用，保留原提交与记录；新改动只补受影响的检查。本地 I2 合入使用已有验证和精确候选审查，不等待远程 CI。CI 在 PR 或手动触发，普通主线直推不重复检查；实际安装与升级验收仅在相关改动或发版时执行。

纯流程、配置或文档收尾按实际影响检查，不重建未变化的产品。合入外部 PR 时仍要求 GitHub Actions 的 `CI` 通过；所有者日常直推例外不用于跳过外部 PR 的验收。开发分支和候选的保存、合入与清理顺序见 [开发说明](development.md)。

`check:native` 包含 `scripts/check-native-boundaries.mjs`：检查契约、数据、网络基础及语音展示归属，并禁止业务目录实现 SQL Repository。它检查相关静态依赖边界，不声称解析全项目的 Rust 宏或运行时调用图。语音输出测试还检查试说终结与外部启动的快照合并、旧收尾和演示归属；浏览器来源投影验证继续保留。

| 命令 | 检查范围 |
|---|---|
| npm run check | TypeScript、ESLint、模块边界/循环、UTF-8、UI测试、Rust检查与前端构建 |
| npm run test:ui | 公共组件、适配器、词典页面与序列化契约 |
| npm run check:native | Rust格式、check、Clippy和单元测试 |
| npm run test:e2e | 浏览器导航、弹窗、动效与词典流程 |
| npm run check:distribution | Git跟踪文件的明确源码分发范围 |
| npm run test:input | Windows 隔离编辑器实际输入、选区、剪贴板及故障恢复 |

E2E前运行 `npx playwright install chromium`。截图、报告、运行数据不跟踪到Git。浏览器内存词典不证明磁盘持久化。

冷启动 TLS 回归：`cargo test --locked --manifest-path src-tauri/Cargo.toml --lib cold_process_tls -- --nocapture`。测试分别启动独立子进程，先确认没有全局加密实现，再执行应用初始化；覆盖 WSS 优先、HTTPS 客户端优先和重复初始化。WSS 使用生产库的默认系统证书连接器，访问有界本机 TCP 夹具，确认发出 TLS ClientHello 后正常处理握手中断。测试不使用验收专用信任、供应商凭据或用户资料，不替代真实识别服务验收。

测试构建刻意同时启用 AWS-LC 和 ring，以保持对原冲突的检测能力；普通产品移除了闲置更新插件，不为这项回归启用第二种加密实现。

更新专项工具的选择和执行范围以 [本地 Harness Verify 工作流](../.agents/skills/beaver-4a20fc43577a-harness/references/workflows/verify.md) 为准。下述历史工具仅供明确请求的固定专项使用，既有记录保留原身份；当前未覆盖的实际更新场景待人工试用。

## Windows 桌面检查

```powershell
npm run test:desktop
```

脚本构建独立 desktop-acceptance 二进制，使用 UUID 数据、凭据及 WebView2 目录检查窗口/托盘调用、隐藏恢复、共享会话、旧缩窗拒绝和退出清理。已存在相同验收二进制的实例时拒绝启动。只清理核验为本次所有的进程，不按名称或端口批量结束应用。日志写入 `.local/`。

词典真实磁盘流程：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml dictionary_query_benchmark -- --ignored --nocapture
npm run desktop:build -- --features desktop-acceptance
node scripts/test-dictionary-desktop.mjs
```

基准命令先创建10k测试夹具。随后脚本检查隔离目录中的CRUD、重复反馈、重启持久化、第二实例拒绝与悬浮窗访问限制。它只连接自己启动的WebView2调试端口；被占用时报错。测试目录在 `.local/data-acceptance/`，普通构建不允许覆盖词典路径。

测试专用构建后重新生成普通二进制：

```powershell
npm run desktop:build
node scripts/test-dictionary-desktop.mjs --regular
```

关闭Vite后检查普通二进制独立运行、本地字体、原生接口和无开发组件入口。`--regular`不修改用户词典内容。

## 数据与性能

录音/历史原生闭环（实际默认麦克风、独立凭据和受控 WSS，未调用供应商）：

```powershell
npm run test:desktop
node scripts/test-speech-desktop.mjs
cargo test --manifest-path src-tauri/Cargo.toml history_query_benchmark -- --ignored --nocapture
npm run desktop:build
node scripts/test-models-desktop.mjs --regular
```

使用 Tauri CLI 构建以启用本地资源协议；直接 `cargo build --release` 不能替代该步骤。语音脚本连接自己启动的 WebView2，验证麦克风 PCM、等待 task-started、双窗/权限、WAV/回放、重启、取消与结果版本、认证/空结果/服务错误及删除。通过实际模型菜单切换3.0后再次录音，断言两款模型各自的 run-task ID、已有配置和凭据保持、重启仍选择3.0。随后通过模型菜单选择豆包，验证跨服务草稿密钥清空、二进制/gzip、累计结果去重、录音与重识别、取消、仅明确资源拒绝的两资源尝试、认证错误不重试，以及第四次进程重启后的持久化。它保存原尺寸预览、受控首字/结束耗时、渲染帧间隔和内存指标，最后清理本测试 UUID 凭据和证书。测试会录制几秒当前默认麦克风，PCM 发给 localhost 夹具，不发送给供应商。个人录音和报告留在 `.local/`。

基准在独立库测试 1k/10k/100k 的第一页、末页、中英文包含及无结果搜索。记录 DB 执行 p95，队列和 IPC 是另外的诊断项。10k 普通查询预算 50ms、包含 100ms，100k 仅压力观察，不能承诺容量。真实原生检查不能替代供应商识别质量；供应商凭据需在应用内填写，再检查中文、中英混合、停顿、静音和重试。

模型配置原生闭环（隔离数据库、测试凭据与受控 HTTPS）：

```powershell
npm run desktop:build -- --features desktop-acceptance
node scripts/test-models-desktop.mjs
npm run desktop:build
node scripts/test-models-desktop.mjs --regular
```

测试自行生成 UUID 范围的证书和凭据，最终清理显式记录的引用；截图、日志留在 `.local/`。普通构建检查只读，不改变个人配置。测试专用信任不编入普通构建。Rust 单元测试额外启动真实子进程，在凭据事务各阶段退出并验证恢复。供应商凭据未提供时不能用上述受控服务代替供应商实测。

模型界面回归覆盖语音三款具体预设、文字直接输入、分段页签、颜色、移动反馈及减少动态的键盘操作、独立默认、旧配置用途保留、名称生成、保存失败保留输入、请求取消与迟到结果。浏览器流程在1120×760和960×640检查必要字段、固定保存与关闭按钮的实际命中区域；从更多设置底部触发修改确认，并检查继续、放弃、Esc、遮罩和焦点恢复。原生帮助命令测试主窗口、未知标识及悬浮窗拒绝，不为测试打开外部浏览器。

窗口修订需另做当前Windows缩放下的实际桌面截图检查：普通、最大化、还原、拖动与托盘恢复；主窗口外沿为系统圆角，页面四角内部连续填充。主窗口与悬浮窗背景规则分别检查，浏览器截图不能替代系统外沿验收。

Rust数据测试覆盖Unicode、约束、版本冲突、事务、迁移拒绝、备份/恢复副本、锁与损坏。显式基准使用1k/10k/100k独立数据库，检查查询计划和耗时；10k常规查询/写入p95预算50ms，包含搜索100ms，100k仅压力观察。具体性能依赖机器和磁盘，不能把单次结果当作容量保证。

## 人工验证

历史流程包含1000条浏览器记录的连续续载、搜索、路由阅读位置恢复、最小窗口键盘菜单，以及完整旧版本复制、迟到响应、失败重试保留成功结果、行内删除确认与保存设置冲突。播放测试确认空闲、暂停、隐藏期间没有周期查询，离开期间的慢播放会先结束再停止，后续挂载不受旧清理影响。

语音桌面脚本在隔离环境验证新摘要的主窗口权限、原生行内回放及保存设置保护，随后停止应用、通过 `history_desktop_fixture --ignored` 生成10000条隔离历史，再重新启动验证IPC摘要耗时、至少1000条连续续载及阅读恢复。夹具只接受本次UUID，要求现有历史为空，不能用于个人数据库。专用尺寸命令只编入 acceptance 功能，且限定隔离主窗口的1120×760和960×640；普通构建不包含该命令。原尺寸截图、实测耗时及进程日志留在 `.local/`，不进入贡献者文档或源码分发。

检查默认1120×760和收缩窗口的中文排版、焦点、导航几何、中性文字、白色主体及语音状态。正常/减少动态、隐藏恢复和快速取消分别观察。原生自动化覆盖命令与窗口状态，不能替代物理任务栏操作、所有DPI、多显示器、掉电、安装/卸载、签名及真实语音服务验证。

源码分发检查使用显式文件范围；新模块、构建脚本或文档需同时审核其用途并更新检查器。可用 `node scripts/check-distribution.mjs --tree REV` 审核某个提交，或 `--snapshot DIRECTORY` 检查解压源码。

## 跨应用口述分支验证

职责、输入策略及恢复边界见 [Windows 口述](dictation.md)。`npm run test:input` 构建两个仅测试特性启用的编辑器夹具，验证实际文字与选区、常见剪贴板格式、延迟渲染及故障恢复。`npm run test:desktop` 后执行 `node scripts/test-speech-desktop.mjs`，使用独立验收 EXE、受控 TLS 服务和实际麦克风验证配置、录音、历史以及原生编辑器/Chromium 的交付路径。脚本记录各控件是确认填入、无法确认还是提供复制；受控文字不能证明供应商识别质量。

发布检查使用 `npm run desktop:build`（无安装包），验收特性及夹具不进入该构建。需要并行保留运行中的本机版本时，可使用标准 `--target x86_64-pc-windows-msvc` 生成到独立 Cargo 目标目录。Windows 150% 以外的缩放、实际物理快捷键、个人常用应用版本及真实供应商短录音分别留证。

口述反馈验证覆盖准备、识别等待、输入完成、提示与无输入目标的可复制结果。`dictation_dismiss` 与结果复制均绑定任务及展示身份；排队的旧复制在实际写入剪贴板前失效。受控语音脚本延迟最终结果，检查停止后继续等待尾字、已显示悬浮窗的自然收起、输入目标变化时保留应用内结果，以及无目标结果的复制和关闭。设备和快捷键的浏览器流程检查两种窗口尺寸下公共控件及保存按钮的实际裁剪边界。

悬浮窗结束按钮还必须经过真实 Win32 鼠标消息验证。辅助程序按当前 DPI 转换坐标、核对目标 HWND／进程和命中根窗口，只有实际发送了鼠标按下与松开且目标文字正确才通过。命令结束和浏览器调试点击作为不同入口留证。原位路径应保留已交还的文字，阻止旧轮继续改写。任务窗口 lease、旧事件时间和迟到布局分别有身份回归；源码审查或此类受控通过不代替 Codex 实际输入与物理点按／按住验收。

脚本记录启动 EXE 的 SHA256、PID、隔离数据目录，并从该子进程诊断流取得任务事件分类和提交结果。诊断只含身份、状态、错误码与耗时。普通路径日志与运行进程实际持有文件不一致时，先核实文件身份；不能用旧文件内容推断当前任务。读取差异没有被验证前保持待查。

原生输入测试会先完整保护当前剪贴板。含有无法安全备份的格式或超出预算时，测试停止并保留原内容；改为复制一小段普通文字后再运行。该环境阻塞不能记为输入验证通过，也不能通过删除格式或关闭保护绕过。真实麦克风的音量起伏、物理点按/按住及供应商最终尾字仍需对应实测证据。
## 原位输入分阶段验证

连续草稿回归覆盖自有分段结束的同步重入、无对象／旧对象通知、真实当前组合结束、首个错误及后续不确定效果。现有 `test:composition` 使用普通已部署组件，读回自有编辑器的累计与最终正文，并覆盖空尾1500ms停顿后继续输入；输出目标PID可用于核对实际加载模块。协调器状态测试、原生控件正文、普通开发程序窗口几何与用户Codex试用分别记录，不能相互代替。日常开发和候选复用规则以项目Harness的Verify工作流为准。

`npm run check:tsf` 检查独立 TSF 组件的格式、lint 与契约测试；`npm run test:tsf` 在一次性 RichEdit 目标中断言累计更新、提交无重复、选区替换、取消恢复、过期请求及用户属性修改后的保护。普通 EDIT 另行验证组合区的两次累计更新，以及提交后控件中的完整文字；组合期间的正文不一定已进入其 WM_GETTEXT 缓冲，两种读取证据分别记录。此验证不修改用户剪贴板或已安装输入法，不代表 Codex、全局启用或真实语音已经通过。

`npm run test:composition` 在隔离原生编辑器中检查主程序的累计更新、草稿提交、编辑交接、新轮及真实前台切换保护，断言实际目标文字。它不发起供应商请求；控制的编辑证据不能替代真实物理快捷键与用户打字。识别测试分别覆盖轮次音频标签、FFT 尾部隔离、暂停丢弃、旧轮原文汇总、停止优先、取消资源及任务级总量上限。

组件部署和实际环境范围见 [组件说明](../src-tauri/text-service/README.md)。Codex 原位探针、受控识别以及真实供应商口述分别留证；确认探针最终文字进入输入框，不代表点按、按住、真实编辑交接和所有输入法共存已经通过。机器级安装、更新、移除及缺失文件恢复不得用纯内存测试代替。


实时输入验收必须在结束前观察两次同范围更新，并核对正常任务未调用模拟粘贴、剪贴板序号未变。日常启动环境中的组件启用与受控进程分别留证；组件不可用时应在采集之前拒绝准备。Codex 实际口述尚未通过时，受控结果不能代替该验收。

多行回归必须同时检查临时组合与正式提交边界：普通 LF、CRLF、连续换行、原文中的 U+2028、Unicode、选区替换及键盘交接，正式正文严格匹配原文换行语义。生成的临时表示与原文分开校验，不能全局替换 U+2028 后宣称正确。检查协议 4 的实际目标组件版本；排队清理遇到重复断连仍须执行，外部属性变化只能撤销转换权限，不能取消资源释放或覆盖新属性。已失败的旧报告保持原值，新版本通过需单独报告。

### 分段候选验证

开发中的协议8候选使用 `node scripts/test-prefix-input.mjs` 检查自有浏览器控件：input、textarea、contenteditable、ProseMirror，以及可多行控件和支持行内换行的富文本结构。空框、只有左文、只有右文与选区替换分别校验；可选择隔离 Chromium 或本机 Edge，并记录浏览器版本。初始识别、两次前段提交、继续增长的尾部和最终全文分别断言。测试源不含字面 U+2028，中间观测仅解码已知的生成软换行；最终正文保持严格换行语义断言，不解码标记字符。

`test:tsf` 同时保留不安全末尾换行前段的明确拒绝回归：原正文、光标和组合不变，并可继续提交正确边界。取消必须保留前段且删除仍持有的分隔符／尾部；只剩分隔符的正常结束必须形成实际段落。私有管道回复测试检查重复 Settle／Inspect 的拒绝不会退役原 pending，这属于 dispatch 与真实回复管道证据，不是所有入站传输测试。

组件加载版本、目标实际正文、测试操作条件和用户 Codex 验证单独记录。旧组件上的边界试验通过不能代替新版组件验收。桌面不可操作导致前台激活失败时，保留失败报告并等待操作条件恢复，不绕过实际文字断言。

### 完整分段整理链路

空尾停顿接续使用 `node scripts/test-prefix-input.mjs ProseMirror-single empty chromium gap`：两次前段提交均留下空尾，间隔1500ms再续写，严格核对最终正文。第三个参数还可用 `left`、`right`、`selected`。末参数可指定 `delete-prefix`、`replace-prefix`、`outside-edit`、`outside-delete`、`right-edit`、`caret-return`，在相应正例通过后验证固定位置保护码及实际保留文字；左右编辑须使用对应的 `left`／`right` 形状。`suffix`、`suffix-abort` 在提交后向文档末尾增加段落，验证继续输入或取消只影响自有尾部，新增换行及右侧原文保持。测试使用程序化编辑事件；物理键鼠交接仍由原生输入和日常验收分别覆盖。

选区与焦点准备应单独记录。默认 `SAYWEAVE_PREFIX_SETUP=state` 保留程序化初始选区；`keyboard` 先聚焦自有窗口，再通过浏览器键盘事件建立选区，并断言实际选区。`SAYWEAVE_PREFIX_RETURN_FOCUS=1` 在选区建立后切到自有原生夹具再返回，保留已有选区；它与开始口述后切换目标是不同场景。不能用其中一种准备方式通过，抹去另一种方式的失败，也不能把浏览器注入事件标成物理键盘验收。

选区形状还包括 `selected-unicode`、`selected-second-line` 和 `selected-across-lines`，分别覆盖 emoji、段落后的坐标和跨段选区。`SAYWEAVE_PREFIX_ABORT_INITIAL=before|after` 检查首字前或首次替换后的取消，严格断言原始全文和段落，仅允许原选区或恢复末端光标。`SAYWEAVE_PREFIX_EDIT_PREPARED=text|caret` 用于 `ProseMirror-single selected`：收到 Begin 答复后、首份 Update 前，修改等长正文或移动光标，要求旧 Update 被拒绝且实际用户正文未被覆盖。它与开始前的重新选区测试分开。

原生故障注入由 acceptance 构建提供；`SAYWEAVE_TSF_SELECTION_REPAIR=1` 仅启用原生夹具中的初始选区故障矩阵，不控制产品修复。矩阵覆盖首份直接分段提交、首写后组合启动失败、写入中取消和写入前取消，并核对实际正文、修改状态及所有资源释放。

`SAYWEAVE_PREFIX_READ_SELECTION=1` 在开始输入前，用独立 MTA 只读核对自有前台目标的 UIA 选区。报告仅含受限正文的摘要、UTF-16 长度及选区位置；不输出正文，不移动选区，也不改变原生写入权限。`stable` 只表示两次接口读取相符，不代表 UIA 与 TSF 存在原子快照或共享变化版本。此取证会增加启动前查询，不能用于产品延迟测量。

定向时序诊断支持 `SAYWEAVE_PREFIX_STEP_MS`（100—2000ms），以及分别控制 Begin 后、首次 Update 后间隔的 `SAYWEAVE_PREFIX_BEGIN_MS`／`SAYWEAVE_PREFIX_FIRST_MS`（0—2000ms）。`SAYWEAVE_PREFIX_RENDER_PAUSE_MS`（0—1000ms）只在自有页面的首次 `compositionstart`、`beforeinput` 或 `compositionend` 暂停一次，事件由 `SAYWEAVE_PREFIX_RENDER_EVENT` 指定。暂停不修改正文或选区，不进入产品代码；所有参数写入报告。极短间隔可能让轮询看不到中间帧：即使最终全文正确，原有中间帧断言仍保留为未通过，不能混称完整验收。可用 `SAYWEAVE_PREFIX_TRACE_IME=1` 留存隔离浏览器的输入事件追踪；空追踪不作为回调未发生的证据。退出时关闭追踪与所有自有测试资源。

`node scripts/test-speech-desktop.mjs --polish` 使用实际Windows麦克风、WebView、已部署输入组件及自有原生编辑器，网络仅连接隔离TLS ASR与文字生成服务。先读取目标中的原始ASR再释放相应模型输出，分别断言两次整理前段、最新尾部、结束后的完整Unicode正文，以及独立原文/整理历史和摘要引用。原生EDIT的读取断言使用明确CRLF，历史语义正文使用LF；不全局转换原文U+2028。

第二次会话在前段已提交、下一语段仍等待模型时取消，断言实际前段保留、尾部及分隔符移除、历史只保留提交范围、待生成记录删除及录音不可用。夹具、凭据、窗口、监听和证书按UUID清理；测试不改个人模型、历史、剪贴板或输入法。观察屏障仅控制隔离服务，不为产品增加等待。

扩展链路分别控制已确认句段的冲突重复、模型等待期间 Backspace 交接及新轮、15秒生成超时原文提交，以及生成等待期间取消。每项读回完整目标正文；取消读回被替换的原内容并记录实际选区，不承诺重新高亮原范围。键盘事件来自带专用标记的测试注入，不能称为物理键盘验证。千问已确认句段不可再修订，冲突重复由协议适配器忽略；来源修订资格由来源契约测试另行覆盖，不能用伪造的供应商更新声称 actor 修订测试通过。

只修订最后的取消验收时，使用 `node scripts/test-speech-desktop.mjs --polish-case=cancel`。它仍构建当前程序、创建隔离配置并读取实际正文，但不重复已通过的其他语段及超时用例；完整入口的默认范围不变。

显式来源索引基准为 `cargo test --release --manifest-path src-tauri/Cargo.toml --lib source_validation_benchmark -- --ignored --nocapture`。它检查1万、5万及10万 Unicode 字符的快照构建和全部冻结单位校验，报告各次耗时；不访问设备、数据库或网络，也不代表整次口述延迟或容量保证。

`models::live_acceptance::live_processing_acceptance` 是默认跳过的真实服务验收。只有显式授权的执行设置 `SAYWEAVE_LIVE_PROCESSING_ACCEPTANCE=selected-default-synthetic`；测试只读日常默认文字配置，使用日常release凭据命名空间，仅上传代码中固定的合成口述。必须核对实际配置目录及编译测试身份；Codex子进程的重定向空目录不能替代日常目录。报告含稳定错误码、合成结果和耗时，凭据不写出。单次通过不能声称供应商质量或p95全面达标。

这些链路证据与真实供应商质量、Codex日常输入、物理快捷键、不同缩放和长期压力分开记录。正常悬浮阶段不应出现文字段落；无障碍名称、失败恢复操作和无目标结果分别验证。

## 接续与后台保存验证

`dictation/drafts_tests.rs` 使用实际草稿状态转移、受控HTTPS生成和可暂停的原生答复，验证前段落地期间下一段派发、最新尾部保留、相同正文确认推进及来源修订撤销依赖结果。`archive.rs` 使用可控时间覆盖五秒保存延迟、队列合并、容量与终结屏障；音频测试覆盖文件线程背压和准备失败后的清理所有权。取消日志测试覆盖晚到写入、重启恢复及意图写入失败。受控状态测试不替代真实目标文字断言。

`node scripts/test-speech-desktop.mjs --polish` 使用隔离模型、数据库、录音和实际编辑器验证分段整理；报告包含实际目标文字及组件哈希，结束后关闭自有窗口并清理测试凭据。若输入组件源码、协议和已验收产物均未变，可通过构建环境变量 `SAYWEAVE_TSF_PAYLOAD` 指定该已核实DLL，避免一次重建导致无功能变化的组件安装要求；必须记录来源提交与SHA256。未设置时按源码构建。这个变量只影响构建产物，不向前端开放组件路径。

### 结束后纠词采集验证

`windows_input::learning_observation` 覆盖原输入框快照的归属、重复文本、范围外变化、IME、撤销、删除、来源栅栏和两种观察来源的幂等入库。`dictation::revisable` 的生产默认路径回归核对正常结束只转移只读观察、分段结算携带实际来源；测试不能切回旧草稿策略代替生产路径。

现有 `scripts/test-prefix-input.mjs` 的 actor 分支可显式设置 `SAYWEAVE_ACTOR_LEARNING=1`。它驱动真实 `SessionDraft`、注册的原生输入组件和自有 ProseMirror，实际交付 `项目使用SQLit`，正常结束后通过受控原生按键补上 `e`，分别核对宿主事件处理后的正文、真实采集证据、隔离数据库自动词条，以及下一轮词典热词和整理上下文。模型分类回复在这项测试中固定；真实供应商仍由 `test:learning:live` 单独验收。

测试二进制必须先以已核实的 `SAYWEAVE_TSF_PAYLOAD` 和 `SAYWEAVE_TSF_PROXY_PAYLOAD` 构建，并通过 `SAYWEAVE_ACTOR_TEST_EXE` 指定；组件身份不一致时仍拒绝输入。入口设置 `SAYWEAVE_BLOCK_REGRESSION=1`、`SAYWEAVE_BLOCK_ACTOR_ONLY=1`，普通结束场景再设置 `SAYWEAVE_ACTOR_RAW_ONLY=1`。`SAYWEAVE_ACTOR_LEARNING_FIELD=1` 仅在测试二进制内关闭原范围观察，独立证明快照采集；它不是产品设置。`SAYWEAVE_ACTOR_LEARNING_CASE=resume` 配合固定回复 `应用使用SQLit，配置完成`，核对手动编辑与接管后观察重新建立。`regression` 则保持学习观察开启，运行原有三次编辑及正文回归。

`replace-word` 在结束后用原生选择键整词替换 `Seville→Sayweave`；`delete-word` 逐字符 Backspace，再重新输入该词。后者叠加 `SAYWEAVE_BLOCK_FLOATING_PREDICTION=1` 使 `S` 与 `eville` 分属两个真实输入块，核对最终正文和完整词形证据；固定模型只回复当前可改的 `eville`，不能返回整词改变被测条件。该富文本提供者在空文档返回末尾 CR，不能代替真正空字符串提供者的清空分支验收。`recording-word` 在录音中改词，等待采集屏障后于既有编辑总期限内续说，检查学习及结束后的正文。注入 Unicode 键不代表真实输入法候选已提交；未提交组词、确认提交后计时、并发结算、范围失效、超时和下一轮即时结算另由定向单元覆盖。

报告保存于 `.local/prefix-input/<run-id>/report.json`，包括实际正文、交付、证据、词条、组件身份及清理事实。自有宿主通过不代表微信、Codex 或其他宿主的读取能力已通过；实际软件另记录输入、读取、纠错、分类、入词和下一轮使用六项结果。

性能记录区分ASR句段确认、请求派发、服务返回、原生应用与保存完成。同一段文字确认状态改变仍须进入调度。真实服务可能直到结束才确认；在未采集相应时序前，只能报告受控接续通过。显式真实文字模型测试只上传固定合成语料、只读原配置和凭据；数据目录为空时拒绝网络请求。模型返回成功、正文可用及实际原生应用分别记录；内容质量通过人工评测判断，单次耗时不作为p95或客户端优化比例。

### Explicit recording-time supplier acceptance

The ignored `speech::doubao::live_segmentation::live_segmentation` test requires `SAYWEAVE_SEGMENTATION_ACCEPTANCE=synthetic-only`, a generated mono16kHz/i16 WAV in `SAYWEAVE_SEGMENTATION_WAV`, and `SAYWEAVE_SEGMENTATION_VARIANT=baseline|window|two-pass`. It reads only the selected daily speech configuration/credential and records raw confirmation counts against mapped counts before/after Finish. The two-pass variant is diagnostic only and is not a product setting. Use the same synthetic waveform for comparison; no microphone or daily recording is read.

The ignored `speech::live_continuation::live_continuation` test requires `SAYWEAVE_LIVE_CONTINUATION=synthetic-only` and the same waveform variable. Build `input-fixture` with desktop-acceptance first. It uses the production RecognitionSession, serial draft actor, selected real text provider and installed TSF component against an owned foreground editor. It samples actual target text, requires at least two visibly changed model prefixes before Finish, and compares the complete final text with the settlement report. Daily settings/history are read-only; temporary storage is isolated. This does not exercise physical shortcuts, microphone capture or Codex rendering. Run explicitly in the same ordinary-user environment as the daily program, separately from aggregate tests; missing credentials are unverified, not a simulated pass.


## Frontend interaction revision

`src/ui/selection.test.tsx` covers controlled values, cancellation of superseded animation, rejected changes, labels and radio keys. `src/ui/waveform.test.ts` covers independent time slices, silence, stale frames and task reset. SpeechStore tests verify20Hz meter notifications do not notify page subscribers except visible seconds.

`tests/history.test.tsx` covers20-item fixed pages, deleted anchors, empty middle/first pages, late requests, deferred background refresh after navigation success/failure, search membership and cache eviction. Native `bounded_reading_pages_reread_only_requested_summaries_and_share_search_rules` covers the actual bounded repository query. Existing50-item consumers remain covered.

`tests/e2e/ui-interaction.spec.ts` saves short videos and screenshots for controls and live-meter simulation. The history and dictation browser suites cover1120×760/960×640, focus restoration, stable geometry and reduced motion. Actual device capture is separate: `cargo test --manifest-path src-tauri/Cargo.toml real_microphone_meter_acceptance -- --ignored --nocapture` reads the default microphone for6seconds, emits only numeric feature statistics, creates no file/network request and stops capture. This proves real capture features, not a human judgment of speech dynamics. Device-specific loud/quiet interaction should be recorded separately when observed.
# Installed startup diagnosis with an existing package

Set `diagnose_recovery=true` on the existing `diagnose` workflow to test only the restart boundary after a real stop timeout. It verifies both original signed packages, seeds an explicitly synthetic signed-cache journal while the application is stopped, and requires the old process lifetime to end and a new owned process/window/page/endpoint to appear. A renderer-closed IPC result alone never passes. This uses no application build, HTTPS server or certificate import; it is not complete update-mechanism evidence.

Optional `diagnose_metadata=true` downloads the same candidate's verified ordinary tooling, refuses existing component registrations, installs the already-signed package's ordinary component on the disposable runner and reads bounded raw UTF-16 registry values plus `ITfInputProcessorProfiles::GetLanguageProfileDescription`. The read-only helper cannot register, activate or modify a profile. Its receipt records `metadataExpected` separately from successful query completion; a false value remains a publication blocker. This optional component preparation is separate from the default registration-free startup diagnosis.

The tooling stage also verifies raw registry and TSF API descriptions for the newly built ordinary DLL before application packaging. It injects an unknown current-user preference and requires removal to reject it without changing registration, then checks native cleanup without the older test-only deletion fallback. A second registration is explicitly disabled; its observed Enable DWORD0 must also be completely removed by the product. A failure injected after profile enablement must roll back the native registration. A separate machine fixture uses two copies of the same verified DLL in explicit synthetic protected paths: it corrupts only the description, forces a failure after replacement writes, compares the exact prior registry, then verifies successful metadata repair with enablement/category relationships retained and complete native removal. The failure hooks are compiled only into the acceptance probe's library; the ordinary DLL and normal application expose none of these commands. Metadata stdout/stderr and receipts are bounded and archived independently of application-package logs.

The Windows workflow has a `diagnose` phase for a frozen installer from a failed startup run. Supply `reuse_run` and its complete `candidate_sha`. It records the product candidate and current acceptance-tool commit separately, rejects changes outside the explicit diagnostic-only source list, verifies the original full minisign signature and signed version, and derives the EXE hash by static extraction of that signed package before installation. The original package manifest and failed receipt remain unchanged.

This short job compiles only the existing shared signature verifier in a temporary minimal crate. It creates no application package, signing key, certificate trust or TSF registration. Process, window, page, owned listener, CDP, native renderer and build identity are separate results. Its bounded diagnostics contain only the requested WebView debug environment fields, command-line availability, process lifetimes and safe startup logs. Failure timestamps and subsequent process-close timestamps distinguish the failed readiness check from cleanup termination.

Genuine package builds record the generated NSIS `MAINBINARYSRCPATH`, verify the complete installer signature/version, and statically extract its actual EXE. The payload must match the restored build output except for Tauri 2.12.1's single NSIS bundle-type token patch. Stored executable identity is the signed payload hash; the unpatched build-output hash is recorded separately. Earlier manifests and failure receipts remain frozen.

A successful diagnostic is an installed-startup proof for that original signed package. It does not resolve a recorded package-metadata mismatch, produce mechanism evidence or authorize a formal release. Repair affected tooling, then run the appropriate full candidate acceptance. Repair-branch pushes run short tooling only; explicitly dispatch `startup` or `full` for package work after reviewing the current diagnostic outcome.

Current WebView2 runtimes ignore environment/HKCU browser-argument overrides for elevated hosts ([Microsoft security documentation](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security#for-an-elevated-host-app-use-appropriate-override-flags)). These disposable elevated jobs use a temporary, app-specific HKLM policy lease instead. It refuses existing app/wildcard values and running hosts, records a unique lease identity before changing anything, and verifies the exact owned string/type before removing only that value. Parent keys are retained and reported until the disposable runner is destroyed, avoiding deletion of unrelated concurrent writes. Confirmed value removal is required for a pass. Process-token elevation is captured with the owned lifecycle; production source and debug configuration stay unchanged.

## 安装器自动部署验收

新候选的首次安装和升级证据必须由安装器自动部署组件取得，不能再调用设置按钮或 `--sayweave-text-service=install` 补齐。旧版环境的准备单独记录。验证缺省启用、显式停用保持、重复安装、旧 DLL 占用、异账户提权、程序复制后的失败、安装器中断及恢复，并核对程序/包/helper/DLL 哈希、事务阶段、账户和原始数据/凭据。

浏览器测试通过 `SAYWEAVE_E2E_PORT` 指定独立端口，并禁止复用已有服务器；已有官网预览不能被当作产品。正常空结果测试覆盖首页和外部任务，真实错误、存储失败及原位输入效果不确定仍保留提示。已通过的相同候选产物继续按原身份复用。

## 口述编辑后结束

### 首次输入诊断夹具

`scripts/test-prefix-input.mjs` 与 `scripts/test-richtext-block.mjs` 复用真实 `SessionDraft`、TSF 适配器及本地受控模型。`SAYWEAVE_BLOCK_INITIAL_STREAM=1` 检查逐次正文，`SAYWEAVE_BLOCK_FIRST_ONLY=1` 只运行首次流式及结束；普通完整场景继续覆盖三次手动修改，`SAYWEAVE_BLOCK_BACKSPACE=1` 增加直接删除后续说。每一步核对宿主事件处理后的正文，并在下一次更新前及释放后复核。

`SAYWEAVE_BLOCK_FLOATING_PREDICTION=1` 启用自有非编辑浮动建议，用于保留首次输入取消组合的失败回归。该问题仍为已知问题。`SAYWEAVE_BLOCK_COMMIT_EARLY=1` 保留本轮实验的首次写入屏障夹具，依赖归档实验核心的 `SAYWEAVE_INITIAL_BARRIER`，当前稳定核心没有该屏障；它不属于稳定候选的通过记录。屏障只证明原生回执尚未返回，不能证明跟踪尚未建立。原始故障、重复正文和提前编辑超时报告继续保留原产物身份。

复用 `npm run test:composition` 的自有编辑器入口核对实际正文：编辑后立即结束、短延迟交接、恢复空范围、恢复后无新文字、继续输入再结束和迟到旧识别。受控原生答复测试另行锁定准备成功／拒绝与结束的准确先后顺序；延迟实机样本不代替准确竞态证明。组件保持普通已验证 DLL，核对目标实际加载身份。Codex 和物理麦克风由用户后续试用，不能由这些夹具记录为通过。

## 同进程核心切换

`npm run test:tsf` 保留现有输入检查，并调用 `scripts/test-core-switch.ps1`。轻量自有宿主通过同一代理 COM 对象，在同一 PID/STA 从普通核心 A→B→C 读回实际正文，核对哈希和代次、活动范围拒绝切换、旧请求隔离、候选激活失败恢复、引用保活与多 STA。普通管道回执检查覆盖 Commit 未确认、Deactivate、原控制进程重连读取与精确哈希 ACK，不重放编辑。验收代理的受控导出仅选择测试核心及生命周期故障，普通代理和核心禁止这些导出。输出保留在 `.local/core-switch/current`，不修改系统登记或日常程序。

实际 Codex 验收另行记录首次迁移重开、此后两次更新时 PID/创建身份/STA/代理保持不变、执行核心改变与实际正文。受控宿主通过或模块出现不代替实际输入；Codex 未完成时 Change 保持活动。


## 0.1.11 更新可靠性定向检查

更新入口状态使用 `tests/update-dock.test.tsx`、`tests/update-ui.test.tsx`，保存屏障沿用 `tests/settings-update-barrier.test.ts`。`tests/e2e/update-progress.spec.ts` 仅检查侧栏圆环、原有关于入口、真实进度/验签区分、键盘焦点、150%缩放、减少动态与高对比度。

`atomic_record::tests` 使用自有临时目录检查真实读句柄短暂/持续占用、阶段期限、权限/路径拒绝和目标变化。`deployment::retired::tests::protected_collection_and_journal_contention` 为显式管理员隔离测试：只创建 `Program Files/Sayweave/MaintenanceTest-<UUID>`，验证64项清单、双载荷容量、保留身份、打开/映射文件、哈希不符、中断清理及真实DACL拒绝，不修改系统登记、真实事务、正式程序或用户数据库。该测试默认忽略；需要相应授权时单独执行，不加入普通CI的系统维护。

完整检查/E2E/分发检查由正式发布流程执行。发布及文件实验通过不表示用户应用内更新已经通过，实际升级结果另行记录。

## 实际 IME 学习与首次输入等待

生产 actor 的只读学习用例可启用 `SAYWEAVE_ACTOR_LEARNING_IME=1`，与 `SAYWEAVE_ACTOR_LEARNING_CASE=delete-word`、`SAYWEAVE_ACTOR_LEARNING_FIELD=1` 配合。自有编辑器先取得真实交付收据，逐字删除到空，再通过原生字母／Shift／选词按键产生真实组词。断言短暂变化不提前发布；最近字段稳定15秒后，即使UIA仍缓存组合范围，也应生成单一纠错、自动词条及下一轮热词和整理上下文。实际选词提交后核对完整正文及去重。状态查询用于维持测试驱动存活，不产生键盘活动。Unicode 文本注入不能替代这项覆盖。

`SAYWEAVE_ACTOR_INITIAL_IME=1` 与 `SAYWEAVE_ACTOR_RAW_ONLY=1` 在现有 `test-prefix-input` actor-only入口验证：实际 IME 组合存在时准备输入，等待期间多批 ASR 与音频路由保持；提交原组合后一次交付完整来源，并继续流式。无原字段身份、超时、结束、取消与迟到 Begin 使用受控 actor 测试，实际正文和同窗换框另外核对。测试只操作自有窗口；功能结果仍绑定各宿主实测，不能由模拟回执或单元测试数量代替。

`SAYWEAVE_ACTOR_INITIAL_IME_EARLY_SWITCH=1` 在原生首次拒绝回执返回前设置日志屏障，同窗口切到另一个字段后释放，断言新字段保持空白、原 ASR 来源进入结果交付。`SAYWEAVE_ACTOR_INITIAL_IME_SWITCH=1` 覆盖进入等待后换框；`SAYWEAVE_ACTOR_INITIAL_IME_TIMEOUT=1` 覆盖等待原期限耗尽。暂停时钟测试另覆盖14.9秒发出、15.1秒成功返回，以及成功返回前已观察到超时的两种顺序。普通受控 actor 也使用生产 `Draft::prepare` 入口，避免只测试旧启动入口。

`SAYWEAVE_ACTOR_INITIAL_IME_DIRECT=1` 配合首次组词用例，保留真实 IME 的 `hello`，直接提供多批口述。夹具不按空格或回车提交组词；断言 `hello周三`、`hello周三开会`、`hello周三开会地点在上海`，固定整理回复后结束正文保持，且实际发生一次整理请求。必须使用普通候选核心复核；验收专用核心的结果单独记录。

`SAYWEAVE_ACTOR_LEARNING_RESTART=pending|committed` 配合 `delete-word`、`SAYWEAVE_ACTOR_LEARNING_IME=1`，连续验证首轮交付、真实输入法纠词及下一轮口述。两轮保留同一页面和字段，先核对旧任务退出后的修正词，再检查第二轮准备、Begin、首次更新及最终退役后的全文。两轮任务 ID 必须不同、编辑器 PID 相同；只完成首轮不能通过。`pending` 保留外部组合，`committed` 使用真实空格确认。两轮 actor 是独立进程，不代替主程序内复用观察线程的验收。未提交组合的直接输入仍保持未通过状态；曾出现修正词在准备阶段清空的候选保留为失败证据。

所有 actor 分支必须等待退出码0，并在退出后核对最后一份完整预期正文。场景首个错误与收尾错误分别保留；提前返回、仅采样而未比较正文或等待超时均不能视为退役成功。

`SAYWEAVE_ACTOR_INITIAL_IME_END_SWITCH=1` 保留在原组词真实 compositionend 回调中切到同窗第二字段的交错夹具，检查原文、新字段及结果交付。该夹具曾在已撤回的外部组词自动完成候选上运行；取消／连接变化后的捕获发布、编辑和焦点通知撤销提交资格的原生单元结果也属于该历史候选。当前原生源码恢复至 `9ef8f54`，相关自动完成路径及其单元测试已撤回；当前覆盖以回退后的执行记录为准。

`SAYWEAVE_ACTOR_LEARNING_SUBMIT=enter` 配合 `delete-word` 或 `recording-word`，使用原生键盘逐字删除、重打纠词，在15秒前由自有宿主处理真实 Enter 并清空输入框。断言单份证据、自动词条及下一轮热词／整理上下文；录音中提交另核对停止后正文仍为空。与停键15秒、录音中修改后继续说分别验证。物理监听用例须显式设置 `SAYWEAVE_ACTOR_PHYSICAL_INPUT=1`、`SAYWEAVE_ACTOR_OBSERVE_INJECTED=1`；脚本为 actor 提供独立验收 UUID，监听器只接受授权的自有前台进程。缺少标记时的注入不能被记为真实编辑监听覆盖。

`SAYWEAVE_BLOCK_CONTINUITY=prediction-revision` 配合空白浮动建议，依次提供同一未确认来源的 `周`、`周三`、`舟三开会`，再提供新片段 `地点在上海`。封存首字保持为 `周`；后续正文必须持续变为 `周三开会`、`周三开会地点在上海`，最终整理为 `周三开会，地点在上海。`。固定整理回复只覆盖可修改的新块。只有首字或音频仍在运行均不通过。来源单元另覆盖缺少上下文、重复锚点、片段替换、字素分界及已确认片段刷新后的吸收保持。
