# Sayweave

Windows 优先的 Tauri 2 / React / TypeScript 桌面项目。已实现应用骨架、公共 UI、首页、共享设置壳、状态动效，以及真实保存到 SQLite 的手动词典。语音为确定性演示；真实识别、模型服务和自动纠错尚未接入。

双击根目录的 **启动开发版.cmd**，重复体验无需制作安装包。首次编译请查看 **查看开发日志.cmd**。停止和重启入口只管理启动器所属的 Windows Job。

- [开发和工具链](docs/development.md)
- [架构与状态协议](docs/architecture.md)
- [设计对应与补充](docs/design-map.md)
- [验收记录](docs/verification.md)
- [交互动效与验证](docs/motion-verification.md)
- [本地数据架构与恢复](docs/local-data.md)
- [词典桌面验收与性能](docs/local-data-verification.md)
- [设计资料](design/README.md)

先运行一次 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-tools.ps1`，再在 PowerShell 中执行：

```powershell
. ./scripts/environment.ps1
npm.cmd ci
npm.cmd run desktop:dev
```

项目工具位于忽略的 `.local/tools`，不替换全局 Node/Rust。开发、普通发布与测试数据目录互相隔离；浏览器预览使用内存，不能代替桌面持久化验证。日常不需要反复制作安装包。

GitHub 仓库仅包含已跟踪的业务源码、配置、设计规范、测试源码和必要开发说明。数据库、密钥、依赖、构建产物、冻结大文件导出与本机 Harness 执行状态均不发布。本地未跟踪的设计探索稿也不在仓库内。

项目 Agent 工作流入口在 AGENTS.md；对应 Harness Skill 在本机独立管理，未打包到业务仓库。新机器运行应用可按以上步骤准备环境；Agent 协作还需要安装匹配项目 ID 的 Harness。正式合入情况以分支提交和本机 Integration 记录为准。仓库目前私有；项目自身开源许可尚待确定，公开前遵循 [源码分发规则](docs/publication.md)。
