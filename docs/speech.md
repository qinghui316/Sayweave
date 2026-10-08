# 录音、实时识别与历史恢复

本开发分支实现应用内麦克风录音、千问及豆包 WebSocket 识别、只读字幕、复制、历史播放和重识别。原生麦克风、Windows 凭据、磁盘历史及播放使用隔离桌面测试；协议使用受控 TLS 服务。真实供应商的质量、延迟和长段表现仍需应用内配置实测，受控文字不替代供应商证据。

## 配置与使用

设置 → 模型 → 语音识别，选择 Qwen Audio3.1、3.0或豆包 Seed-ASR2.0的实时识别模型、填写 API 密钥并保存，再回首页试说。内置模型 ID 分别为 `qwen-audio-3.1-asr-flash-streaming` 和 `qwen-audio-3.0-asr-flash-streaming`，并非服务器模型列表；两款千问模型共用一个适配器。豆包使用独立双向流式协议，配置模型 ID 为 `Doubao-Seed-ASR-2.0`。名称和源站可在更多设置调整，既有自定义模型原样保留。

协议的部署地址要求：北京源站形如 `https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com`，兼容源站可明确填写 `https://dashscope.aliyuncs.com`。不要附加 Chat 的 `/v1` 路径。客户端生成 WSS `/api-ws/v1/inference`，正常校验证书，不跟随重定向。

保存不发请求；主动录音/确认重识别才上传音频，可能产生费用。取消不能保证撤销服务端费用。官方资料：[接入](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-websocket-api)、[客户端事件](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-client-events)、[服务端事件](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-server-events)。

设备支持默认或明确麦克风，明确设备失效不会换到另一支。识别文字只读，可选择、复制；复制只在点击后写入系统剪贴板，不读取剪贴板。

## 架构与会话

`SpeechPort / HistoryPort → adapters → speech/commands → SpeechService`。采集在 `speech/audio.rs`、协议在 `speech/qwen.rs` 与 `speech/doubao.rs`、会话在 `speech/mod.rs`、存档/文件/播放在 `history/`。Repository 复用容量 64 的单连接数据线程。文件、采集、网络不占数据库线程，Runtime 锁不跨异步等待。

开始冻结配置及版本、密钥、设备和保留设置。全应用同时一个录音或重识别，与模型检查共享四个网络名额。任务 ID/代次过滤迟到回调，演示不能推进真实任务。Runtime v1 仅管理窗口和阶段投影；Speech v1 独立传递任务/尝试、句段、内容序号、阶段版本与存档反馈。音量最多 20Hz、字幕合并最多 10Hz、最终立即发布，确认句段约每秒落盘。内容更新不改变布局版本或重启 320ms 缩窗；以后编辑草稿单独消费识别事件，不能回写原始结果。

CPAL 实际设备格式通过有界环形缓冲送给处理线程；回调只下混和移交，不执行 SQL、文件、网络或等待。Rubato 转为 16kHz 单声道 PCM16，补偿初始延迟并排空尾部。溢出明确失败。本地 10 分钟保护上限不代表供应商已验证该容量。采集前停止本应用回放，采集期间播放返回忙。

千问等待 `task-started` 后才采集和发送。结束先停止/排空，再发 `finish-task`。连接10秒、准备总预算15秒、结束等待30秒。连接或服务准备超时返回 connectionTimeout；识别中的等待仍返回 timeout。连接和准备日志记录阶段、耗时、协议及稳定错误，不包含地址、密钥或正文。消息 1MiB、单句 64KiB、累计文本 1MiB、最多 10,000 句段。暂定结果替换、最终冻结去重、心跳不进正文；错误与空文本明确反馈，无自动重传。

## 协议扩展边界

模型预设定义显示名称/模型 ID/默认源站/协议；用户配置独立保存身份和凭据引用。`models/types.rs` 与前端 `contracts/models.ts` 集中定义可执行用途、实时字幕、文字测试和地址校验，固定的 `tests/contracts/model-capabilities.json` 同时被两端测试。历史重识别、默认替代、首页和配置使用同一判断，Rust负责最终校验。

`SpeechService → RecognitionSession → 静态协议分派`。业务只处理供应商无关的 `Segment / Transcript / Event`，统一会话负责准备、PCM发送、事件读取、排空结束和取消释放；Socket、认证、原始响应与压缩留在协议/传输模块。读写分离，音频和事件队列各32项、控制响应队列8项；队列溢出明确失败，取消丢弃会话并终止传输任务。音量、窗口及历史不因新增协议而复制。启动组装独立网络协调器，文字和语音共享四个名额；只有一个语音任务。

数据库检查点后积攒的音频按每轮最多16包送出，先处理取消和已到达的服务事件；发送队列允许最多500ms的有界等待，让传输任务消费短暂突发。持续背压仍明确返回缓冲溢出，采集回调不等待网络，音频不会静默丢弃。

采集持有独立的结束/取消信号监听，收到请求立即停止麦克风，不等待数据库检查点或发送批次返回。已采集音频仍按正常结束流程排空；监听在采集对象释放时清理。

同协议新模型增加预设和映射测试；同协议新服务增加源站/帮助；新协议增加适配器、能力定义、固定样例和必要迁移。保持静态分派，不引入通用注册系统。

## 豆包双向流式协议

默认源站 `https://openspeech.bytedance.com`，生成 WSS `/api/v3/sauc/bigmodel_async`。握手只使用本配置的 `X-Api-Key`、UUID `X-Api-Request-Id` 与2.0资源。先请求 `volc.seedasr.sauc.duration`，仅准备阶段明确45000030资源未授权时尝试 `volc.seedasr.sauc.concurrent` 一次，共用15秒预算。认证、网络、超时及一般服务错误不切换资源。

发送 `bigmodel`、PCM16/16kHz/单声道、标点、ITN、full结果和utterances。收到有效准备响应后采集；最后排空音频并发送结束包。二进制协议校验版本、动态头长度、类型、序号符号、payload长度、gzip及原始/解压后1MiB限制。full结果替换暂定后缀，确定分句冻结，最终包先发布文字再发布完成事件；不会重复追加累计文本。服务失败不自动上传。原始错误正文不进入提示或日志。

官方依据：[双向流式识别](https://docs.volcengine.com/docs/DoubaoVoice/bidirectional-streaming-automatic-speech-recognition-websocket?lang=zh)。只支持新版单API Key鉴权。供应商识别质量、账户开通情况和长段能力需真实配置验收，隔离服务不能证明这些事实。

## 数据与恢复

迁移 003 成组重建模型/默认表并保留旧 ID、凭据、版本、默认；001/002 不变。004 增加 STRICT 记录、尝试、音频、设置。005成组重建模型及默认关系并增加豆包协议约束，保留旧ID、凭据引用、版本、默认及历史，重新检查外键。升级沿用一致性备份、校验和回滚。每次重识别新增尝试，成功才更新当前结果；失败/取消保留旧成功。统计按当前成功的本地留存记录计一次，删除会减少统计。

历史列表每批50条，使用创建时间降序/ID升序的游标摘要；完整文字与版本按需读取。数量与该批摘要在同一读事务中取得，旧分页接口继续兼容。搜索命中旧版本时，展示、展开和复制绑定该版本。记录排序、尝试所属记录/时间、资源状态/到期有查询索引；包含搜索扫描归一化字段，不声称 B-tree 加速任意包含搜索。阅读缓存、行内操作、播放查询和更新规则见 [history.md](history.md)。

当前数据环境的 `audio/`：数据库准备 → UUID `.part` 写入/刷新 → WAV 完成/校验/同步 → 同目录 `.wav` 改名 → 提交可用状态和结果。识别与存档成功分开反馈。前端只收资源 ID；路径拒绝链接/重解析点。

启动只恢复登记资源：修复可恢复 PCM 部分文件、标记中断、继续删除/到期；不上传、不清理未知文件。缺失/损坏资源标记不可用。播放/重试占用资源，清理跳过活动资源。

历史/录音默认开启，30 天保留，可选 7/30/90 天或永久；到期仅删录音，文字/版本保留。设置仅影响新任务。启动及每日清理，失败保留意图和恢复操作。关闭历史不永久保存录音；仅存文字时当前音频可内存重试，退出后不可播放/重试，新任务替换上一份临时音频。

取消新录音（含处理中）丢弃该次记录/音频；取消历史重试只停止新尝试。退出停止采集/网络，数据/凭据收尾最多 5 秒。异常退出依靠文件状态和事务恢复。数据库备份不包含整个音频目录，恢复副本时缺失明确不可用；当前删除不抹除备份文本。

当前录音重试绑定任务的内部记录身份；历史重试失败不会取另一段临时音频。新操作准入与退出信号串行登记，排空等待已准入操作，包括完整后台恢复及到期清理周期；每日等待不占用准入计数。退出后拒绝新设备、偏好、历史、播放和复制请求，既有录音允许完成事务收尾。取消遇到存储失败仍清空当前内存音频/文字，并提示清理未完成，不能显示保存成功。

## 权限、动效与验证

主窗口拥有完整受控接口；悬浮窗只能读必要语音状态、开始默认口述、结束/取消及隐藏自身，不能访问历史、密钥、文件路径。密钥留在 Windows 凭据。日志仅白名单操作、耗时、错误码、采集格式/缓冲峰值，不记录正文或原始服务错误。

有限 280ms 外壳尺寸转换保留既定设计，原生预留画布并延迟收拢，音量使用 transform；不是其他组件动画布局属性的通用许可。减少动态取消循环/回弹，保留明确状态文字，见 [motion.md](motion.md)。

隔离数据、截图、音频、日志和凭据不进 Git。仅 `desktop-acceptance` 加 UUID 环境可注入 localhost 证书，普通构建不能。复现见 [testing.md](testing.md)。供应商语义质量、持续 10 分钟、掉电、所有设备格式/DPI/多屏需单独证据。未实现快捷键、跨应用输入、编辑合并、热词上传、润色/翻译、本地模型和其余 ASR 协议。
## Terminal recognition responses

Doubao completion is driven by the last-package flag or negative terminal sequence after payload validation. Positive update ordinals are used to reject stale interim responses; a terminal sentinel is handled independently. A valid terminal packet can contain final text or just acknowledge the already received text. The adapter confirms retained segments and emits completion before the business session stores a successful attempt. Malformed packets, supplier errors, timeout, cancellation and connection loss still follow their respective failure paths.

Connection diagnostics distinguish DNS, TCP, TLS/WebSocket upgrade and overall service preparation. Transport entries share a generated connection ID and contain only stage, elapsed time and stable outcome. TCP_NODELAY avoids batching small handshake/audio frames. The microphone still starts only after service preparation succeeds; the UI then enters recording. The ten-second transport allowance within the fifteen-second preparation budget is a failure threshold, not expected startup latency. Zero-byte missing audio with zero recorded duration is shown as uncaptured audio; missing recordings that contained samples retain the missing-file label. Unavailable audio has no displayed expiry.

The main and floating voice controls receive an explicit speech/demo presentation mode. Real sessions use recording and recognition status labels, independently of whether a microphone level is available. Decoder diagnostics contain only stage, stable error, frame type, flags and byte count; they exclude credentials and response content.

首页试说仅显示从首页启动的任务和该任务的重识别结果。语音快照携带 trial、external 或 history 来源；全局语音服务与任务互斥继续共享，首页投影只接收 trial 的状态与音量。外部口述、历史重识别不会覆盖试说文字和反馈，本机用量仍随全部存档更新。试说结果保留到下一次主动试说；页面切换、隐藏恢复不会清空，前端重载期间如全局任务来自外部则试说恢复待机。后续任务替代旧录音后，旧试说仍可复制，重新识别入口停止使用。

## Recording-time sentence boundaries

Qwen-Audio 3.0/3.1 Streaming explicitly selects VAD sentence boundaries (`semantic_punctuation_enabled=false`, `max_sentence_silence=800`) while keeping heartbeat and the duplex session. Doubao sends `end_window_size=800` together with full utterance results. This is sentence endpointing, not a request to end recording or invoke the text model; silence PCM continues through the ordinary audio queue. No two-pass, semantic smoothing or speculative confirmation is enabled by this revision.

`sentence_end` and `definite` make a source segment available to the existing draft scheduler. Task completion remains independent. The existing 150ms merge budget and source-bound model requests apply; model completion settles a prefix with the latest remaining ASR tail. Normal completion drains final recognition and remaining cleanup. Diagnostic boundary and dispatch-state changes contain task/turn identity, counts and timing, never recognized content.
