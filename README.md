# Sayweave

Windows 桌面语音输入应用，使用 Tauri 2、React、TypeScript 与 Rust。

目前已实现桌面窗口、托盘、状态动效、本地手动词典和 API 模型配置。词典支持增删改查与 SQLite 持久化；模型面板支持多份配置、分用途默认选择、系统凭据存储、模型列表和显式文字测试。

语音控件和快速试说仍为演示；真实录音、识别、全局快捷键及跨应用输入尚未实现。语音默认配置可以保存，识别能力尚未验证。历史、表达偏好等页面目前为预留入口。

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
- [状态图标与动效](docs/motion.md)
- [测试方法与覆盖边界](docs/testing.md)

应用源码位于 `src/`，原生层位于 `src-tauri/`，测试位于 `tests/`。依赖与锁文件随源码提交；数据库、运行日志和构建产物不属于源码。

## 许可

项目自身的开源许可尚未确定。第三方字体、图标及动效依赖的许可见 [src/assets/licenses](src/assets/licenses)。
