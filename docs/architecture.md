# 骨架架构与边界

DictionaryPort、主窗限定命令及专用SQLite线程独立于Runtime，详见 [本地数据架构](local-data.md)。Runtime不持久化指窗口/语音演示快照；词典是真实持久化，两者职责分离。

## 目录与依赖

- app：入口、Hash 路由、错误边界、适配装配、跨功能协调。
- features：home / history / dictionary / preferences / settings / onboarding / voice；内部实现由模块 index 导出。
- ui：无业务数据和原生调用的按钮、字段、导航、模式行、Dialog、VoiceControl。
- design：语义 CSS 参数、基础样式、动画及减少动态。
- contracts：Runtime v1 类型／快照验证与独立 DictionaryPort／数据响应验证。
- adapters：唯一允许导入 Tauri API 的位置；浏览器模拟遵守同一端口。
- dev：仅开发环境可达的组件和演示场景，生产构建不会引入。
- src-tauri：内存协调器、窗口、托盘、调用者权限及独立数据线程／Repository。
- tests：组件、合同、集成与 e2e；Rust 测试就近位于 coordinator 和 data。
- scripts：源码边界、编码与分发检查，以及可复现桌面测试。

应用层组装功能；功能只读公共 UI/契约，UI 只依赖设计/文案/素材。功能不越界读取其他功能内部文件。AST 边界检查覆盖导入、重导出和动态导入，并检查循环。未创建插件/HTTP 服务或通用全局状态容器。

## 状态协议

Rust 持有唯一跨窗口 Snapshot：protocol=1、单调 revision、main/overlay 显隐、session(id,phase)。窗口显示、演示动作都由原生校验后提交；局部输入和设置选项归本窗口前端。整个应用退出重置，不做业务持久化。

DesktopRuntimePort 提供 getSnapshot、subscribe、windowAction、demoAction、dispose。错误码独立于中文文案，底层异常不直接显示。

连接先订阅，再读快照，前端只接受更高 revision。焦点/可见恢复重新读取。前端卸载释放监听；开发 StrictMode 的异步连接也会清理。取消、新会话、退出均让旧结果失效。Rust 锁只保护短修改；窗口操作、emit、异步等待不持锁。延迟任务要求 id 与 phase 都匹配。

演示：idle → connecting(650ms) → recording → processing → 900ms后completed → 完成停留约1000ms → idle（从finish提交起1900ms）。录音待显式完成；取消/失败终止动态反馈，失败可重试。波形仅前端表现，不按帧 IPC。browser mock 生命周期只用于浏览器流程，不作为原生证明。

快照固定样例 tests/contracts/snapshot.json 同时经过 Rust serde 和 TypeScript parser。原生单元测试覆盖旧结果、隐藏主窗、禁止悬浮窗操纵主窗和无效操作不推进版本。

## 窗口与能力

主窗口 main；独立 voice-overlay 一次创建后复用。主窗关闭隐藏，托盘恢复现有窗并聚焦；悬浮窗显示不抢焦点。托盘退出先使会话失效再退出进程。

主窗允许受控目标窗口操作；悬浮窗只可隐藏自身。仅这两个标签可触发演示。命令不接受文件路径、shell 或任意网络；capabilities 按窗分别启用事件监听，主窗额外拖动标题栏。生产 CSP 禁止远程字体/资源和任意网络。

原生窗口包含 12px 外缘以容纳阴影：44×44 控件对应 68×68 窗口、84×40 对应 108×64、216×52 对应 240×76、失败 176×44 对应 200×68。尺寸提交校验 revision，迟到回调不能调整新状态。固定位置是初始演示选择，后续验证多显示器/工作区边界。

## 状态反馈与缩窗时序

AnimatedIcon 不持有业务状态；CopyFeedbackButton 只隔离外部异步请求代次，AsyncStatusFeedback 纯受控展示。useMotion 结合窗口快照、页面可见、交叉观察和减少动态。VoiceControl 在处理/完成/失败之间保留 SVG 实例，波形在 CSS 内，不按帧 IPC。

publish 在扩大前保留 current/target 各轴最大值；320ms 后仅当前 revision/session 可以缩到最终大小。无新增命令、权限、业务协议或持久化。完整规则与验证入口见 [motion.md](motion.md) 和 [测试说明](testing.md)。

窗口显示由应用层通过级联 MotionScope 传入通用 UI；它只有布尔可用性，不存储或推进业务状态。设置前景仍可播放自身反馈，仅背景路由暂停。document可见但native窗口已隐藏时，复制/连接和通用加载指示也停止循环。
