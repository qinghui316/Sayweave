# Sayweave

Windows 桌面语音输入应用，使用 Tauri 2、React、TypeScript 与 Rust。

支持快捷键口述、跨应用流式输入、键盘交接、忠实文字整理、口述历史及录音恢复。词典提供手动维护、ASR 热词和真实纠正后的自动入词；个性化按应用积累表达偏好，两个学习开关独立控制。

模型面板提供 Qwen Audio3.1/3.0 和豆包 Seed-ASR2.0，文字服务可直接填写 URL、密钥和模型。多份配置支持分用途默认选择和显式连接测试；密钥保存在 Windows 系统凭据中，历史、词典与偏好保存在本机 SQLite。浏览器开发使用内存模拟，原生流程和真实供应商的验证范围见[测试说明](docs/testing.md)。

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
- [跨应用口述与键盘交接](docs/dictation.md)
- [应用感知个性化与自动入词](docs/personalization.md)
- [Windows 发布与自动更新](docs/release.md)
- [状态图标与动效](docs/motion.md)
- [测试方法与覆盖边界](docs/testing.md)

应用源码位于 `src/`，原生层位于 `src-tauri/`，测试位于 `tests/`。依赖与锁文件随源码提交；数据库、运行日志和构建产物不属于源码。

## 许可

Sayweave 采用 [Apache-2.0](./LICENSE) 开源。第三方字体、图标及动效依赖的许可见 [src/assets/licenses](src/assets/licenses) 和 [NOTICE](./NOTICE)。

## Windows 发布与更新

通过验收的安装包发布在 [GitHub Releases](https://github.com/qinghui316/Sayweave/releases)。故障版本 v0.1.3、v0.1.5 和 v0.1.6 已撤下；公开发行列表为空时暂停下载。已安装这些旧版的用户需要手动覆盖安装修复版，后续正式版本提供应用内更新。源码及日常开发构建不接收公开自动更新。
