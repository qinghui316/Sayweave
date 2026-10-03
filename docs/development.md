# 开发与体验

## 环境

项目选定 Node 24.21.0、Rust 1.99.0 / x86_64-pc-windows-msvc，使用本机 VS 2022 C++ 和 WebView2。版本由 setup-tools.ps1 与 rust-toolchain.toml 固定；Node ZIP 校验官方 SHA256。首次下载需要网络，随后常规启动不安装依赖。

加载 `scripts/environment.ps1` 仅影响当前进程和它的子进程；Cargo/Rustup 也使用项目目录。package-lock.json 和 Cargo.lock 提交，源码与文档 UTF-8。

## 日常入口

| 入口 | 行为 |
|---|---|
| 启动开发版.cmd | 隐藏辅助进程，显示真实桌面窗口，首次等待编译 |
| 停止开发版.cmd | 核验 PID、启动时间、运行标识与根路径，再结束所属 Job |
| 重启开发版.cmd | 先停所属进程，再重新启动 |
| 查看开发日志.cmd | 打开当前 UTF-8 运行日志 |

记录位于 `.local/dev/current.json`，日志每次新运行单独保存。端口固定 5173；占用时报告并停止启动，不终止占用者、不换端口。重复启动不创建第二组进程。记录失效时核验真实进程；身份冲突拒绝清理。

主窗口关闭隐藏到托盘。托盘可以恢复窗口、打开/隐藏悬浮窗、退出整个应用。前端热更新；Rust 改动触发重新编译。退出托盘应用与停止开发进程是不同操作。

## 检查命令

先加载环境，再用 npm.cmd：

| 命令 | 内容 |
|---|---|
| dev | 浏览器辅助开发 |
| desktop:dev | 桌面开发 |
| doctor | 项目工具和必要依赖检查 |
| typecheck | 严格 TypeScript |
| lint | ESLint、依赖边界/循环、UTF-8 与乱码检查 |
| test:ui | 组件、浏览器适配及状态同步测试 |
| test:native | 原生协调器与序列化测试 |
| test:e2e | Playwright 页面流程、截图和文字颜色 |
| test:desktop | 可选原生验收构建：真实窗口、托盘共享处理器、会话与退出清理 |
| check:native | Rust fmt / check / Clippy / test |
| build | 前端生产资源 |
| desktop:build | tauri build --no-bundle |
| check | 类型、lint、UI、原生及前端构建 |

首次浏览器测试运行 `npx.cmd playwright install chromium`。完整验证还需要 Windows 桌面实测及 no-bundle 产物实开，浏览器不能代替原生窗口验证。

无安装包产物为 `src-tauri/target/release/sayweave.exe`；不是安装/卸载/分发验收。开发期不需反复安装。真实功能阶段再验证系统能力，首个可试用版本才交付测试安装包。

`test:desktop` 编译 `desktop-acceptance` Cargo feature，使用真实 Tauri 窗口和托盘回调共用处理器；不模拟任务栏鼠标点击。脚本核验应用退出和其已记录的直属 WebView 子进程清理。运行前会结束此项目路径下的单个 release 实例；开发实例及其他应用保持不动。日志在 `.local/desktop-acceptance*.log`。验收后运行 `npm.cmd run desktop:build` 恢复普通构建；普通构建不编入验收入口。当前交付产物已经恢复普通构建。

## 初始窗口与设计画布

1440×1024 是 Figma 对照基准。当前启动尺寸为 1120×760 **逻辑像素**，会根据当前显示器工作区限制到约 90%，保留最小保护值 960×640。Windows DPI 会换算物理像素，所以不能把逻辑尺寸与截图像素直接相等比较。缩小使用局部布局和滚动，窗口仍可手动调整；不承诺该值为最终产品最小尺寸。

桌面实现采用固定字阶：主页标题32/44、普通标题24/32、模式标题16/24、正文14/22；侧栏184，导航40高，按钮36高，设置面板最多920×620。1120×760首页首屏包含快速试说操作。参数与冻结Figma值的区别见 `DESIGN.md` 和 `design-map.md`。

## 状态图标与动效验收

交互实现及组件契约见 [motion.md](motion.md)，本轮结果和覆盖边界见 [motion-verification.md](motion-verification.md)。正常与减少动态场景由 `npm run test:e2e` 覆盖；跨窗口收拢和快速重启用 `npm run test:desktop` 检查。后者仍需随后构建普通 `npm run desktop:build`，不把验收feature版本当作交付产物。

复制和连接样例只在开发组件页，不写真实剪贴板、不联网；正式使用无需该页面。语音和模型真实服务继续按后续Change接入。

## 手动词典与本地数据

词典页现在通过 DictionaryPort 读写真实 SQLite；浏览器辅助开发使用内存实现，刷新会重置。开发版和普通发布版数据目录隔离。日常仍双击启动开发版，不需要反复打包安装。

架构、数据目录、迁移、索引与恢复步骤见 [local-data.md](local-data.md)，检查证据见 [local-data-verification.md](local-data-verification.md)。桌面词典验收脚本使用独立 UUID 测试目录，只连接自己启动的应用；普通构建检查为只读。所有测试结束后恢复普通 no-bundle 构建，不将验收 feature 或测试数据库交付为用户运行版本。
