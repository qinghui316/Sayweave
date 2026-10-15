# Sayweave

**把话说出来，文字自然成稿。**

Sayweave 是开源的 Windows 语音输入工具。按下快捷键，文字会实时出现在当前输入位置；补充和改口，可以继续整理当前草稿。手动改过文字，也能接着说。

**[下载 Windows 版](https://github.com/qinghui316/Sayweave/releases/latest)** · **[反馈问题](https://github.com/qinghui316/Sayweave/issues)**

## 下载与安装

支持 **Windows x64**。从 [最新发行版](https://github.com/qinghui316/Sayweave/releases/latest) 下载 `.exe` 安装器并运行，安装过程中可能需要 Windows 管理员确认。

正式版支持在 **设置 → 关于** 中检查、下载更新。下载并验签完成后，点击“重启并更新”。主窗口关闭后会留在托盘，可通过托盘菜单重新打开或退出。

## 开始使用

1. 下载并安装 Sayweave。
2. 打开 **设置 → 模型 → 语音识别**，选择服务、填写自己的 API Key 并保存。
3. 将光标放入要输入文字的输入框。
4. 按默认快捷键 **右 Alt** 开始口述，按当前录音方式结束。

在 **设置 → 设备与快捷键** 中，可以更换麦克风、录入口述快捷键，并选择“按一次开始／结束”或“按住说话／松开结束”。

需要自动整理时，在 **设置 → 模型 → 文字处理** 中配置服务，并开启“整理口述”。未启用整理时使用识别原文。

## 主要功能

- **边说边输入**：文字流式出现在当前输入位置，悬浮语音控件通过真实波形反馈录音状态。
- **前后一起整理**：后续补充和明确改口可以修正当前草稿中的前文，不因中间停顿就固定前面的文字。
- **改完还能接着说**：手动修改或删除草稿块内的文字后，可以继续口述并整理当前块。
- **积累自己的表达**：个人词典提供识别热词，自动入词从真实纠正中学习，个性化按应用积累表达偏好；两个学习开关可分别控制。
- **历史与主题**：搜索本地口述历史；保留了录音的记录可回放或重新识别。界面提供浅色和石墨银灰两套主题。

自动整理只作用于**当前活动草稿块**。输入框原有正文、已经交付的其他块，以及其他输入框中的文字不会自动纳入。

## 模型与数据

语音识别支持千问 Qwen Audio 3.1／3.0 和豆包 Seed-ASR 2.0。文字处理支持配置 OpenAI 兼容服务的地址、API Key 和模型；语音识别与文字处理可以分别指定默认配置。

你需要自行准备相应服务的凭据，使用费用按供应商规则计算。服务是否可用取决于账户权限、模型及网络情况。

- **密钥**保存在 Windows 系统凭据中。
- **历史、词典和偏好**按各自保存设置保存在本机；录音是否保留由历史设置控制。
- **云端处理**会将相应音频或文本发送到你配置的服务供应商。

配置说明见 [模型与凭据](docs/models.md)、[录音与历史](docs/speech.md) 和 [本地数据](docs/local-data.md)。

## 开发与文档

桌面应用使用 Tauri 2、React、TypeScript 和 Rust。准备 Node.js 24、npm、Rust/MSVC、Visual Studio C++ 构建工具与 WebView2，完整步骤见 [开发环境](docs/development.md)。

```powershell
npm ci
npm run desktop:dev
```

前端支持热更新；原生代码修改后重新编译。`npm run dev` 只用于浏览器界面辅助开发，数据与原生能力使用模拟实现。源码及日常开发构建不接收正式版自动更新。

- **使用与配置**：[跨应用口述](docs/dictation.md) · [历史记录](docs/history.md) · [词典与个性化](docs/personalization.md)
- **开发与构建**：[开发说明](docs/development.md) · [Windows 发布与更新](docs/release.md)
- **架构与测试**：[模块与依赖](docs/architecture.md) · [测试方法与覆盖边界](docs/testing.md)
- **界面与动效**：[视觉规范](docs/design.md) · [交互动效](docs/motion.md)

## 反馈与许可

遇到问题或有功能建议，欢迎提交 [GitHub Issue](https://github.com/qinghui316/Sayweave/issues)。报告问题时请说明 Sayweave 版本、Windows 版本、目标软件、操作步骤及实际表现；分享日志或截图前，请移除密钥和私人内容。

Sayweave 采用 [Apache-2.0](LICENSE) 开源。第三方字体、图标及动效依赖的许可见 [第三方许可](src/assets/licenses) 和 [NOTICE](NOTICE)。
