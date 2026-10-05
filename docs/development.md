# 开发与构建

## Windows 环境

- Node.js 24 与 npm（版本约束见 package.json）。
- Rust，通过 rustup 安装；项目 rust-toolchain.toml 固定工具链及 MSVC 目标。
- Visual Studio 2022 C++ Build Tools，包含 Windows SDK 和桌面 C++ 工具。
- Microsoft Edge WebView2 Runtime。

将 Node、npm、Cargo 与 rustup 放入当前终端 PATH，确认 `node --version`、`npm --version`、`cargo --version`、`rustup show` 可用。不要提交个人工具安装路径。

## 启动

在项目根目录执行：

```powershell
npm ci
npm run desktop:dev
```

Vite 使用 127.0.0.1 的固定端口；端口占用时报错。前端修改热更新，Rust 修改需要重新编译。结束开发使用启动终端的 Ctrl+C；应用内托盘退出关闭该应用。

浏览器辅助开发使用 `npm run dev`。浏览器适配器不提供原生窗口或磁盘词典持久化。开发组件页面只在开发模式开放，正式构建没有该入口。

模型面板在桌面开发版使用真实系统凭据和磁盘配置；浏览器只模拟数据与测试结果。文字测试会向填写的服务发出真实短请求，可能产生少量费用；保存不会自动测试。实现和恢复规则见 [models.md](models.md)。

## 检查

```powershell
npm run check
npx playwright install chromium
npm run test:e2e
```

单项命令及桌面测试见 [测试说明](testing.md)。npm 与 Cargo 锁文件固定依赖；不为每次启动重复安装。

## 构建

```powershell
npm run build
npm run desktop:build
```

桌面 dev/build 包装先编译独立 TSF DLL，再通过构建期变量嵌入主程序；构建仍执行 `tauri build --no-bundle`。通常生成 `src-tauri/target/release/sayweave.exe`；显式 `--target x86_64-pc-windows-msvc` 时位于对应目标子目录。关闭开发服务后可以独立运行，它不是应用安装包。应用签名及整体自动更新尚未配置。

在「设备与快捷键 → 原位输入」中启用当前用户的组件。首次安装、版本更新及移除经 Windows UAC 确认，由主程序固定 helper 操作受保护的 Program Files 目录；不替换默认键盘输入法。更新使用 DLL 内容哈希目录，保留正在被输入软件加载的旧版本；更新后重开输入软件再验收。移除先注销登记，仍被使用的组件文件显示待清理，可在关闭输入软件后重试。直接 Cargo 检查不嵌入安装载荷，仍能使用已安装的组件；标准桌面命令负责可交付程序的载荷。

## 数据与资源

字体随依赖打包，普通构建不访问在线字体。开发和发布使用不同的数据子目录，详见 [本地数据](local-data.md)。不要在用户数据目录运行破坏性测试。

提交前检查 `npm run check:distribution`，审核新增源码和文档的用途；测试生成的 `.local/`、截图、报告、数据库及构建产物保留在本地。
