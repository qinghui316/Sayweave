# Sayweave

Windows 桌面语音输入应用，使用 Tauri 2、React、TypeScript 与 Rust。

目前已实现桌面窗口、托盘、状态动效、本地手动词典和 API 模型配置。词典支持增删改查与 SQLite 持久化；模型面板支持多份配置、分用途默认选择、系统凭据存储、直接填写文字服务及显式连接测试。

本开发分支的模型面板提供 Qwen Audio3.1/3.0及豆包 Seed-ASR2.0具体选择和文字 URL、密钥、模型直接输入。另增加真实麦克风录音、千问及豆包实时识别接口、只读字幕与复制、历史播放、重识别版本及保留清理。原生流程通过隔离桌面测试，真实供应商质量和延迟仍待应用内配置实测。浏览器是明确标注的内存模拟。快捷键、跨应用输入、编辑、文字处理及本地模型尚未实现；表达偏好仍为预留入口。

## 开发

准备 Node.js 24、npm、Rust/MSVC、Visual Studio C++ 构建工具与 WebView2，详见 [开发环境](docs/development.md)。

```powershell
npm ci
npm run desktop:dev
```

前端支持热更新；原生代码修改后重新编译。主窗口关闭时隐藏到托盘，通过托盘菜单恢复或退出。无需反复打包安装。

```powershell
npm run dev             # 浏览器界面辅助开发，词典仅保存在内存
npm run check           # 类型、lint、UI/Rust 测试与前端构建
npm run test:e2e        # 浏览器流程测试
npm run desktop:build   # 桌面发布构建，不生成安装包
```

## 项目资料

- [目录、依赖与窗口状态](docs/architecture.md)
- [开发与构建](docs/development.md)
- [视觉参数与排版](docs/design.md)
- [本地词典、索引、迁移与恢复](docs/local-data.md)
- [API 模型、凭据、默认选择与请求边界](docs/models.md)
- [录音、实时协议、历史与恢复](docs/speech.md)
- [状态图标与动效](docs/motion.md)
- [测试方法与覆盖边界](docs/testing.md)

应用源码位于 `src/`，原生层位于 `src-tauri/`，测试位于 `tests/`。依赖与锁文件随源码提交；数据库、运行日志和构建产物不属于源码。

## 许可

Sayweave 采用 [Apache-2.0](./LICENSE) 开源。第三方字体、图标及动效依赖的许可见 [src/assets/licenses](src/assets/licenses) 和 [NOTICE](./NOTICE)。

## Windows 发布与更新

公开安装包发布在 [GitHub Releases](https://github.com/qinghui316/Sayweave/releases)。安装后的稳定版会在后台检查新版本，下载时显示圆形进度；下载完成后由用户点击“重启并更新”。源码构建用于开发和验证，不会接入稳定版更新通道。
