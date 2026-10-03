# Sayweave 状态图标与交互动效

2026-10-03，Change `sayweave-interaction-motion-v01`。本轮是现有前端候选的交互修订；不修改冻结 Figma，也不接入真实业务。

## 视觉与节奏

保持 1120×760 桌面比例和现行字阶。白色主体、浅灰辅助、黑色文字及外壳、#75FB91 图形反馈；文字和数字不用绿色。语音完成是重点，普通按钮不普遍回弹。导航位置、字号、模式行和下拉/开关行为保持原样。

图标使用内联 SVG，实际描边 2px，普通显隐与结果变化使用 Morphicons 的 snappy 弹簧。首次绘制直接显示当前目标；快速切换从当前几何转向最新目标。现有静态图标继续用 SVG mask。麦克风、取消复用现有源路径；Check、Eye、EyeOff、Copy、LoaderCircle、Link、CircleAlert、RotateCw 按需读取 Lucide 数据。

| 表现 | 参数与归属 |
|---|---|
| 胶囊形状 | CSS 280ms，现有 ease-shape |
| 波形 | CSS 720ms 交替，条间错开 50ms；确定的演示数据 |
| 处理指示 | SVG 图形，900ms 局部旋转；结果到来即变形 |
| 完成确认 | 绿色部件、黑色勾选；scale 1→1.06→1，180ms；每次实际可见的完成切换播放一次 |
| 原生收拢 | 先保留各轴较大画布，320ms 后才设置目标尺寸；验证 revision 和 session |
| 减少动态 | 图标即时切换，取消空间变形、回弹和循环；可见图标变化作 120ms 的 0.8→1 轻淡入 |

减少动态策略显式传给 Morphicons，不能使用其默认 never。文档不可见、组件离屏、窗口快照隐藏或设置遮挡首页时停止循环。隐藏时直接设置最新图标，恢复不重播旧完成反馈。持续波形仅存在于录音展示，无持续待机光效；局部柔光维持原有范围。

## 公共组件契约

`src/ui/AnimatedIcon.tsx`：icon、size、tone(primary/inverse/signal)、enabled、spinning。只负责图形，不决定会话或业务结果。稳定 SVG 实例跨状态保留；useMotion 同时观察媒体偏好、document.hidden 和 IntersectionObserver。应用层通过仅含可见布尔值的 MotionScope 级联原生显隐；嵌套区域不能覆盖隐藏的父范围，设置弹窗前景与背景路由分别处理。通用加载指示同样暂停循环，业务结果仍继续更新。

`PasswordInput`：20px Eye/EyeOff，原位切换输入类型，保存 selectionStart/End/Direction；输入 DOM 节点、值、尺寸和按钮焦点保留。禁用输入同时禁用显隐按钮。

`CopyFeedbackButton`：异步 onCopy(): Promise<void>、disabled、motionEnabled。点击产生新请求代次并清理旧定时器；等待操作返回，成功显示“已复制”1500ms，再恢复；失败显示“复制失败，重试”，直到重试成功。新操作、过期回调、卸载和 StrictMode 重挂载不能覆盖新状态。组件不写剪贴板；后续调用方提供真实操作并负责它自己的副作用。

`AsyncStatusFeedback`：受控 status(idle/pending/success/error)、message、motionEnabled。成功/失败一直显示，直到外部更改；不启动连接、不访问网络、不判断业务结果。

## 语音与窗口

| Phase | 控件 | 原生画布（含外缘） |
|---|---|---|
| idle / cancelled / completed | 44×44 | 68×68 |
| connecting / processing | 84×40 | 108×64 |
| recording | 216×52 | 240×76 |
| failed | 176×44 | 200×68 |

首页及独立悬浮窗都使用 VoiceControl。处理、完成、失败使用同一个持续挂载的状态 SVG。完成按钮和错误重试也使用真实图标。动画不改变 Phase、快照 protocol/revision、原生命令、取消判断或业务停留时间。

原生 Runtime 继续决定：连接650ms后进入录音展示；完成操作进入处理，900ms后完成，1000ms完成停留后待机。取消/失败由既有协调器校验。扩大画布在通知和命令返回之前提交；320ms延迟缩小必须仍为当前 revision/session。纯 Rust 测试和真实 Tauri 验收分别检查各轴留白、旧缩窗拒绝和最新目标尺寸。

## 开发样例与交付边界

现有 `/dev/components` 包含密码、模拟复制成功/失败、连接成功/失败/重置及共享语音演示。模拟复制不写系统剪贴板，连接不发请求、不收密钥。仅 DEV 入口和动态导入存在；普通构建无该页面及正式导航。

依赖精确锁定 morphicons 1.7.1（MIT）和 lucide 1.51.0（ISC）；许可证保留于 src/assets/licenses。许可获取来自安装包，版本可由 package-lock.json 复核。

验证见 [motion-verification.md](motion-verification.md)。真实音量、跨应用输入、模型业务页、连续材质光效和其他显示器/缩放覆盖由后续 Change 处理。

## 检测例外：语音外壳形状转换

2026-10-03 hook 复核：voice.module.css 第1行的 width/height 告警为同一有限状态转换的两条提示。用户已确认外壳约280ms在圆形与胶囊之间转换；直接缩放整个外壳会连同图标和点击区域变形，因此保留尺寸过渡。波形循环只改变 transform/opacity，隐藏与减少动态关闭尺寸过渡；原生先留画布、320ms后按版本及会话收拢。

通过 hook-admin 持久化 layout-transition 规则的 src/ui/voice.module.css 文件范围例外，不关闭其他规则，也不忽略整个文件。该例外不代表消除布局成本或证明性能达标；原生录音 CPU 偏高仍列为性能专项。后续若增加其他布局动画，需要重新判断其合理性。
