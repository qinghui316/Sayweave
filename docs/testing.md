# 测试与覆盖边界

安装环境见 [开发说明](development.md)。测试在项目根目录执行，不依赖个人启动器或私有工具安装脚本。

## 常规检查

| 命令 | 检查范围 |
|---|---|
| npm run check | TypeScript、ESLint、模块边界/循环、UTF-8、UI测试、Rust检查与前端构建 |
| npm run test:ui | 公共组件、适配器、词典页面与序列化契约 |
| npm run check:native | Rust格式、check、Clippy和单元测试 |
| npm run test:e2e | 浏览器导航、弹窗、动效与词典流程 |
| npm run check:distribution | Git跟踪文件的明确源码分发范围 |

E2E前运行 `npx playwright install chromium`。截图、报告、运行数据不跟踪到Git。浏览器内存词典不证明磁盘持久化。

## Windows 桌面检查

```powershell
npm run test:desktop
```

脚本构建专用 desktop-acceptance 二进制并检查窗口/托盘调用、隐藏恢复、共享会话、旧缩窗拒绝和退出清理。它可能关闭该项目路径下已有的发布实例，应先保存个人工作。只清理核验为本次所有的进程，不按名称或端口批量结束应用。日志写入 `.local/`。

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

模型配置原生闭环（隔离数据库、测试凭据与受控 HTTPS）：

```powershell
npm run desktop:build -- --features desktop-acceptance
node scripts/test-models-desktop.mjs
npm run desktop:build
node scripts/test-models-desktop.mjs --regular
```

测试自行生成 UUID 范围的证书和凭据，最终清理显式记录的引用；截图、日志留在 `.local/`。普通构建检查只读，不改变个人配置。测试专用信任不编入普通构建。Rust 单元测试额外启动真实子进程，在凭据事务各阶段退出并验证恢复。供应商凭据未提供时不能用上述受控服务代替供应商实测。

Rust数据测试覆盖Unicode、约束、版本冲突、事务、迁移拒绝、备份/恢复副本、锁与损坏。显式基准使用1k/10k/100k独立数据库，检查查询计划和耗时；10k常规查询/写入p95预算50ms，包含搜索100ms，100k仅压力观察。具体性能依赖机器和磁盘，不能把单次结果当作容量保证。

## 人工验证

检查默认1120×760和收缩窗口的中文排版、焦点、导航几何、中性文字、白色主体及语音状态。正常/减少动态、隐藏恢复和快速取消分别观察。原生自动化覆盖命令与窗口状态，不能替代物理任务栏操作、所有DPI、多显示器、掉电、安装/卸载、签名及真实语音服务验证。

源码分发检查使用显式文件范围；新模块、构建脚本或文档需同时审核其用途并更新检查器。可用 `node scripts/check-distribution.mjs --tree REV` 审核某个提交，或 `--snapshot DIRECTORY` 检查解压源码。
