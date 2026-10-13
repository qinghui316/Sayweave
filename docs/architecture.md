# 骨架架构与边界

持续草稿分支的依赖、协议与迁移契约见 [持续草稿与编辑接管](revisable-draft.md)，验收和合入状态以该 Change 为准。

DictionaryPort、主窗限定命令及专用SQLite线程独立于Runtime，详见 [本地数据架构](local-data.md)。Runtime不持久化指窗口/语音演示快照；词典是真实持久化，两者职责分离。

模型配置通过独立 ModelConfigPort 接入。模型服务复用数据线程，以系统凭据和异步客户端管理密钥及请求；词典与模型互不依赖。迁移、默认关系与恢复详见 [models.md](models.md)。主窗口退出协调网络取消与数据/凭据排空，最多等待 5 秒；Runtime 锁不跨这些操作。

## 目录与依赖

SpeechPort/HistoryPort 与 Runtime 分离，原生 SpeechService 管理录音/重试、固定配置快照、采集和历史服务，通过统一 RecognitionSession 使用千问或豆包适配器，见 [speech.md](speech.md)。Runtime 只投影阶段；音量/字幕有独立内容序号，不改变布局版本。业务历史持久化、窗口快照仍为内存。生产入口为真实语音，演示仅开发/专用验收构建可用。

- app：入口、Hash 路由、错误边界、适配装配、跨功能协调。
- features：home / history / dictionary / models / preferences / settings / onboarding / voice；内部实现由模块 index 导出。
- ui：无业务数据和原生调用的按钮、字段、导航、模式行、Dialog、VoiceControl。
- design：语义 CSS 参数、基础样式、动画及减少动态。
- contracts：Runtime v1、DictionaryPort、ModelConfigPort、SpeechPort、HistoryPort 的类型与响应验证。
- adapters：唯一允许导入 Tauri API 的位置；浏览器模拟遵守同一端口。
- dev：仅开发环境可达的组件和演示场景，生产构建不会引入。
- src-tauri：内存协调器、窗口、托盘、调用者权限及独立数据线程／Repository。
- tests：组件、契约、集成与 e2e；Rust 测试就近位于 coordinator、data、models、speech 和 history。
- scripts：源码边界、编码与分发检查，以及可复现桌面测试。

应用层组装功能；功能只读公共 UI/契约，UI 只依赖设计/文案/素材。功能不越界读取其他功能内部文件。AST 边界检查覆盖导入、重导出和动态导入，并检查循环。未创建插件/HTTP 服务或通用全局状态容器。

协议能力在前后端契约集中定义，固定JSON样例验证一致。独立 network::RequestCoordinator 由启动层创建，供文字与语音共用请求名额、取消及退出信号。业务会话和字幕类型不依赖某个供应商的原始数据，新增协议不改采集、历史或窗口。

## 状态协议

Rust 持有唯一跨窗口 Snapshot：protocol=1、单调 revision、main/overlay 显隐、session(id,phase)。窗口显示、演示动作都由原生校验后提交；局部输入和设置选项归本窗口前端。整个应用退出重置，不做业务持久化。

DesktopRuntimePort 提供 getSnapshot、subscribe、windowAction、demoAction、dispose。错误码独立于中文文案，底层异常不直接显示。

连接先订阅，再读快照，前端只接受更高 revision。焦点/可见恢复重新读取。前端卸载释放监听；开发 StrictMode 的异步连接也会清理。取消、新会话、退出均让旧结果失效。Rust 锁只保护短修改；窗口操作、emit、异步等待不持锁。延迟任务要求 id 与 phase 都匹配。

演示：idle → connecting(650ms) → recording → processing → 900ms后completed → 完成停留约1000ms → idle（从finish提交起1900ms）。录音待显式完成；取消/失败终止动态反馈，失败可重试。波形仅前端表现，不按帧 IPC。browser mock 生命周期只用于浏览器流程，不作为原生证明。

快照固定样例 tests/contracts/snapshot.json 同时经过 Rust serde 和 TypeScript parser。原生单元测试覆盖旧结果、隐藏主窗、禁止悬浮窗操纵主窗和无效操作不推进版本。

## 窗口与能力

主窗口 main；独立 voice-overlay 一次创建后复用。主窗关闭隐藏，托盘恢复现有窗并聚焦；悬浮窗显示不抢焦点。托盘退出先使会话失效再退出进程。

主窗允许受控目标窗口操作；悬浮窗只可隐藏自身。演示命令仅在开发或专用验收构建开放，并校验窗口及会话来源。Runtime 窗口命令只接受枚举操作，不接受文件路径、shell 或网络请求；capabilities 按窗分别启用事件监听，主窗额外拖动标题栏。生产 CSP 限制渲染器网络与远程资源；用户配置的 HTTPS/WSS 模型请求由 Rust 客户端执行。

原生窗口包含 12px 外缘以容纳阴影：44×44 控件对应 68×68 窗口、84×40 对应 108×64、216×52 对应 240×76、失败 176×44 对应 200×68。尺寸提交校验 revision，迟到回调不能调整新状态。固定位置是初始演示选择，后续验证多显示器/工作区边界。

## 状态反馈与缩窗时序

AnimatedIcon 不持有业务状态；CopyFeedbackButton 只隔离外部异步请求代次，AsyncStatusFeedback 纯受控展示。useMotion 结合窗口快照、页面可见、交叉观察和减少动态。VoiceControl 在处理/完成/失败之间保留 SVG 实例，波形在 CSS 内，不按帧 IPC。

publish 在扩大前保留 current/target 各轴最大值；320ms 后仅当前 revision/session 可以缩到最终大小。语音内容不改变此布局版本。真实语音接口和持久化与上述窗口协议分离，见 [speech.md](speech.md)。完整动效规则与验证入口见 [motion.md](motion.md) 和 [测试说明](testing.md)。

窗口显示由应用层通过级联 MotionScope 传入通用 UI；它只有布尔可用性，不存储或推进业务状态。设置前景仍可播放自身反馈，仅背景路由暂停。document可见但native窗口已隐藏时，复制/连接和通用加载指示也停止循环。

## 后端实现与扩展规则

后端功能修改先读本节，再按功能读取 [词典与数据库](local-data.md)、[模型与凭据](models.md)、[语音与历史](speech.md)。这些文档说明当前源码行为；运行验证的方法和覆盖范围见 [testing.md](testing.md)。

### 目录责任

共享数据类型和验证位于 `contracts`，不依赖业务服务或数据库工作线程。模型、历史和快捷键的 SQL 位于 `data/repositories`，继续由同一个 DataWorker 分派；原类型入口保留兼容重导出。请求协调器使用自己的基础错误，业务服务映射为现有接口错误，不反向依赖模型服务。

SpeechService 发布原生快照、生命周期和音量事实；启动层的 speech_projection 负责前端事件与 Runtime 映射。前端收到每次来源转换及终结结果，当前状态 watch 只用于可合并的投影。Runtime 私有任务归属防止旧原生收尾影响演示；真实任务准入由协调器占用决定，不读取延迟的显示阶段作为原生忙状态。

Tauri 规定应用入口、命令注册、State 和窗口机制；以下业务目录由项目按当前职责组织。

| 位置 | 应放内容 | 边界与示例 |
|---|---|---|
| src-tauri/src/main.rs、lib.rs | 框架启动、服务组装、命令注册、窗口、托盘、日志与退出协调 | [lib.rs](../src-tauri/src/lib.rs) 不新增供应商解码或业务 SQL；已有词典命令在这里做权限和数据操作转发 |
| models/commands.rs、speech/commands.rs | Tauri 请求入口、实际调用窗口校验、参数转交、稳定响应 | [模型命令](../src-tauri/src/models/commands.rs)、[语音命令](../src-tauri/src/speech/commands.rs) 不拥有数据库连接或麦克风循环 |
| data/model.rs、models/types.rs、history/types.rs | 序列化类型、字段约束、归一化、错误码及协议能力 | [词典规则](../src-tauri/src/data/model.rs)、[模型规则](../src-tauri/src/models/types.rs) 不依赖页面、凭据实现或网络响应正文 |
| data/mod.rs、data/repository.rs、data/migrations/ | 唯一 SQLite 连接、64 容量队列、事务、查询、迁移、备份与进程锁 | [数据线程](../src-tauri/src/data/mod.rs) 转发静态业务操作；SQL 由 Repository 执行，不把网络或文件录制放进数据线程 |
| models/mod.rs、models/repository.rs | 配置规则、默认关系、版本检查、凭据操作意图与数据库提交 | [模型服务](../src-tauri/src/models/mod.rs) 协调提交；[Repository](../src-tauri/src/models/repository.rs) 只在共享数据线程执行 SQL |
| models/credentials.rs、models/provider.rs | 系统凭据访问、OpenAI 兼容请求与响应解析 | [凭据](../src-tauri/src/models/credentials.rs) 不向前端返回密钥；[文字客户端](../src-tauri/src/models/provider.rs) 不决定默认配置 |
| speech/mod.rs、speech/audio.rs | 单任务生命周期、配置快照、采集、转换、状态发布与收尾 | [会话](../src-tauri/src/speech/mod.rs) 不操作编辑器光标；[音频](../src-tauri/src/speech/audio.rs) 回调不执行网络、SQL、文件写入或等待 |
| speech/recognition.rs、qwen.rs、doubao.rs、transport.rs | 中立识别会话、协议认证/封包/解码、连接与有界网络传输 | [识别分派](../src-tauri/src/speech/recognition.rs) 输出中立字幕；供应商适配器不另建历史、窗口或录音流程 |
| history/mod.rs、repository.rs、files.rs、playback.rs | 历史规则、结果版本、SQL、音频资源恢复/清理、播放 | [历史](../src-tauri/src/history/mod.rs) 按资源 ID 操作；[文件](../src-tauri/src/history/files.rs) 只处理登记资源，不接受前端路径 |
| network.rs、coordinator.rs | 共用请求名额/取消/退出、窗口与演示快照 | [请求协调](../src-tauri/src/network.rs) 不解析供应商协议；[Runtime](../src-tauri/src/coordinator.rs) 不保存词典、配置或历史正文 |

字段校验不能仅留在前端。Rust 在类型/服务/Repository 的实际操作入口校验，SQL 约束负责最后一层一致性。窗口标签来自 Tauri 的 WebviewWindow，不采用前端传入的身份。

### 框架复用与项目封装

- 复用 Tauri Builder、manage/State、command、事件、窗口及插件；Serde 负责序列化，Tokio 负责异步与取消，rusqlite 负责事务/备份，网络库负责 TLS 和协议传输。
- Port 与适配器隔离 React 和原生接口，同时支持浏览器内存模拟；模拟不证明磁盘、凭据或供应商能力。
- DataWorker 让一个线程拥有 SQLite 连接，并限制请求积压；没有连接池或通用数据访问框架。
- RequestCoordinator 让文字测试与语音共享四个请求名额及退出信号；音频资源与凭据操作记录处理跨文件/数据库、凭据/数据库的非原子提交。
- RecognitionSession 对两个实际协议做静态分派。增加新协议时扩展这里；相同协议的新模型只增加预设和映射验证，不提前建立插件注册系统。

新增关键依赖、目录职责或错误语义时，同步修改本节及相应功能文档。自定义封装应对应当前调用者或可靠性需求。

### 最小扩展路径

以增加一个实际 ASR 协议为例，文件放置和调用关系为：

1. 在 speech/ 增加适配器，完成认证、标准 PCM 输入及供应商事件解析。
2. 在 recognition.rs 接入静态分派，输出已有中立识别事件；采集、历史、窗口调用方式保持原契约。
3. 在 models/types.rs 与前端模型契约补能力和校验，在模型预设中补名称、地址和请求标识；只有 SQL 协议约束改变才新增迁移，旧迁移不改写。
4. 添加能力一致性、协议消息、取消/超时及受控原生闭环验证；真实供应商结果单独记录。

请求依次经过页面 → Port → Tauri 适配器 → 原生命令/窗口校验 → 服务规则 → 协议适配器或 DataWorker/Repository。继续复用当前设置、录音、回放和历史，不增加独立 HTTP 服务。

### IPC 返回与错误示例

Tauri invoke 成功时 resolve 具体结果，Rust Result 的 Err 由 Promise reject 返回。当前没有 HTTP 状态码、登录接口或统一 data 外壳。以下是基于类型和契约夹具的格式示例，不是生产数据或实测报告。

词典列表与空列表使用同一形状：

```json
{"records":[{"id":"6dcb1d53-88a0-421e-9c46-0375cddc9d00","kind":"term","original":"Figma","category":"设计工具","replacement":null,"revision":1,"createdAt":1791030000000,"updatedAt":1791030000000}],"total":1,"page":1}
```

```json
{"records":[],"total":0,"page":1}
```

详情与新增返回完整记录；更新保留 ID 并递增 revision，例如：

```json
{"id":"6dcb1d53-88a0-421e-9c46-0375cddc9d00","kind":"term","original":"Figma","category":"界面设计","replacement":null,"revision":2,"createdAt":1791030000000,"updatedAt":1791030001000}
```

词典删除等 Result<(), Error> 成功值为 JSON null，前端端口暴露 Promise<void>。模型保存/删除返回新的完整 Overview，含 configs、defaults、cleanupPending，不返回密钥。词典状态示例：

```json
{"state":"ready","error":null,"schemaVersion":5,"normalizationVersion":1}
```

结构合法但值无效、无权限、资源不存在、数据库不可用分别 reject 下列对象；模型字段错误可附 field：

```json
{"code":"invalid","field":"baseUrl"}
```

```json
{"code":"forbidden"}
```

```json
{"code":"notFound"}
```

```json
{"code":"unavailable"}
```

词典重复可附 existingId，模型请求错误可附 requestId，过期修改使用 conflict。字段限定在所属端口定义中，错误码由功能契约和 Rust 类型共同维护，中文文案留在 locales。Tauri 参数缺失/类型错误可能产生框架字符串错误，适配器将未知错误转换为所属端口的安全后备码，不把底层原文直接显示。列表、状态和快照由 TS parser 检查结构；Rust/TS 固定夹具和原生测试验证兼容性。

### 启动、配置、数据与日志入口

历史列表使用独立的轻量游标读取接口。应用层持有阅读缓存和跨页面修改保护，行组件按需读取详情；菜单、复制反馈等公共组件不保存历史业务数据。接口、缓存更新和播放生命周期见 [history.md](history.md)。

首页启动及聚焦时读取选定模型和设备；短暂的 busy/preparing 读取最多尝试10次，每次间隔100ms。新的聚焦、设置切换和卸载使旧读取失效；写入仍不自动重试。

安装和启动使用 [development.md](development.md) 的 npm ci、npm run desktop:dev；Vite 固定为 127.0.0.1:5173，仅服务前端开发。普通 EXE 不依赖它。后端没有另一个监听端口或 REST 健康接口，状态检查通过 dictionary_status、model_status、history_status 和 speech_snapshot。

配置来自 SQLite 中已保存资料与 Windows 系统凭据；不从前端 .env 保存 API 密钥。数据位于 Tauri app_data_dir 下的 data-dev/ 或 data/，日志位于对应数据目录 logs/sayweave.log；专用验收使用独立 UUID 目录及凭据命名空间。迁移、备份和恢复入口见功能文档。

诊断仅记录操作、稳定错误码、阶段、耗时和允许的关联标识。例如：

```text
SAYWEAVE_DATA operation=list queue_us=42 execution_us=405 result=ok
SAYWEAVE_SPEECH operation=transport_connect connection_id=<UUID> stage=tls_websocket elapsed_ms=10000 result=connectionTimeout
```

这些是脱敏格式示例；没有用户内容、密钥、认证头、完整地址或绝对用户路径。日志失败不阻止应用核心能力；连接保护时限也不是正常就绪耗时承诺。启动、真实磁盘、日志后备、故障恢复和普通构建的验证命令见 testing.md。供应商质量、长期容量、所有 DPI/设备及真实连接耗时需要各自的执行证据。

## Windows 口述模块

### 更新与退出

`UpdateService` 拥有更新任务、原生快照、缓存、验签和安装事务；前端是绑定方法的订阅投影。`Lifecycle` 原子封闭新口述及修改准入，已经取得许可的工作可以完成。更新先等待前端设置与表单保存，再执行原生屏障。`application_exit` 共用托盘退出和更新退出的生产者收尾；归档与各服务返回当前工作的结果，数据库在全部写入生产者结束后才封闭。停止不完整时更新不安装，进入重启恢复状态。构建渠道与正式资产身份约束见 [release.md](release.md)。

### 生成、输入与归档的独立收尾

`SessionDraft` 继续是唯一文字状态所有者。生成请求和原生操作各最多一个在途，另可保留一份提前生成结果；前段生成完成后，下一段已确认来源可以与前段原生执行重叠。原生答复只结算对应操作的来源水位和实际写入修订，不覆盖期间到达的新 ASR 尾部。正文相同可以省略组合区写入，确认状态变化仍进入调度。

语音终结直接携带本任务的最终识别快照。输入完成与保存完成分别投影；后台保存事件带任务 ID，不能改写新任务或首页试说的文字。实际归档完成后才刷新历史和用量。失败在主窗口提供一次可关闭的保存反馈，不改变输入结果。

`archive` 在现有 DataWorker 前提供有界顺序队列：普通队列最多64项／2MiB，允许同键检查点合并，另保留一个终结位置；全局最多16个存活队列。每个队列还可能有一个执行中的写入。容量耗尽明确报告保存失败。音频文件另由有界工作线程写入，最多64块、每块1600个PCM样本；停止采集和音频发送不等待文件 flush／finalize。

取消保留必要的持久意图：先登记恢复日志，延迟到本任务普通写入及文件收尾完成后执行裁剪。启动恢复执行已有意图；旧检查点不得恢复取消尾部。记录租约跨越识别、处理和文件保存，直到对应收尾完成才释放。取消登记失败不会留下永久内存屏障。常规归档与另一任务的取消互不依赖。

退出继续使用最多5秒协调，包含后台归档所有者；异常终止仍可能丢失尚未落盘的普通检查点，恢复不会重放外部输入。数据库009和输入协议8保持兼容。

本分支的口述协调器管理 Session 身份与结束手势，语音服务管理采集及识别连接，SessionDraft 管理 Turn、累计草稿与编辑交接，Windows 输入模块管理 TSF 范围与提交。原始识别、存档、外部输入和展示结果分别维护，详见 [职责与边界](dictation.md)。组件只在目标线程操作组合范围；录音、密钥及数据库留在主程序。设置、网络与系统输入不占用数据库线程。组件维护与口述共用准入锁，维护期间不能启动新任务。有外部目标的任务在组件就绪及草稿建立后才启动录音；设置与准入共用当前用户状态查询。


### Interaction presentation boundaries

Shared selection controls own interaction and transient animation only; feature pages own current values, persistence and rollback. A standalone task-bound meter subscription feeds the waveform leaf without page-level frame updates. Native meter extraction and renderer envelopes are independently testable; microphone, recognition and input lifecycles remain owned by their existing services.

History reading owns a bounded three-page summary cache and lightweight page identity/cursor bookmarks. Browse(pageSize≤50, default50) and reread(ids≤20) share repository summary selection and a transactional count. Reading navigation never loads full attempt bodies; detail and copy retain their existing explicit paths. Background changes reconcile IDs and mark new records without rearranging reading boundaries.

## 安装与组件部署事务

`installer` 是独立原生编排模块，负责固定程序文件、NSIS 元数据和事务记录；`text-service::deployment/registration` 保持组件文件、机器登记和 TSF 用户启用的所有权。前端只读取 `deploymentState`（required/pending/ready/failed），不接收 DLL 或登记路径。维护入口不启动 Tauri、不打开数据库或凭据服务；源码边界检查限制 installer 对业务模块的依赖。

锁定 CLI 2.12.1 的 NSIS 模板由包内原生绑定提取，来源和散列在 windows/installer-source.json。当前用户安装只覆盖 sayweave.exe、卸载器、固定 helper、桌面与开始菜单根目录的两个产品快捷方式，以及两处固定 HKCU 安装元数据。包的额外资源、关联、非空开始菜单目录或其他安装范围必须先扩展并验证备份契约。手动与应用内升级均覆盖安装，不调用旧卸载器。

Cargo 构建目标与安装清单分别管理。CLI 自动枚举的 helper、验收和签名验证程序不由 `binaries` 循环安装；未知额外目标在模板编译时拒绝。维护入口只从冻结资源复制，临时入口与正式文件必须逐字节一致。签名包在执行前按逐条 archive inventory 拒绝重复文件和额外安装文件，再核对程序以及维护入口身份；提取覆盖后的单个文件不能证明清单唯一。

普通用户控制器在任何覆盖之前持久备份；机器服务持全局互斥锁，核验安装器 PID、会话、创建时间、SID 和包哈希，暂存固定 payload 后再提交机器登记。机器登记和清理不启停提升管理员的资料。普通用户启用前先持久化意图，明确缺省与 DWORD0/1 的区别。先核对 before/known-after 值，再恢复半写元数据；未知修改保留恢复锚点。receipt 文件锁和活跃安装器身份禁止第二个安装器接管。

任一必要阶段失败不得显示成功。中断后的下一次安装先恢复未完成事务；无法确认原文件、登记或恢复效果时保留备份并拒绝叠加。旧 DLL 按哈希保留，已加载它的目标软件需重新打开。卸载也先持锁、按已提交收据核对和备份，只删除本安装的程序与元数据。全机组件及 TSF 启停选择是共享资源，不能证明没有其他消费者时保留并记录原因。数据库、凭据、录音与学习资料不进入备份或卸载清理。

更新日志继续读取 schema1。新进程清除日志前必须同时核对当前 EXE/helper/卸载器身份、组件 payload、已提交部署事务，以及已验签候选包的 SHA256 与安装器收据；相同版本号不足以确认完成。

## 输入核心生命周期

固定代理承接 Windows 持有的 TSF 服务对象，核心保留组合、正文、观察和清理。`current-engine.json` 只声明受保护部署的代理／核心哈希与 ABI，前端不能指定加载路径。控制1准备身份后，写协议9将整轮操作固定到代理实例及核心代次；业务协调仍归原生口述协调器。部署和失败恢复继续归现有安装事务，独立组件维护增加版本化更新及移除账本，已知旧值才可恢复；用户数据与默认键盘不参与核心切换。边界、首次迁移与验收见 [跨应用口述](dictation.md)。
