# 本地数据验证

[数据架构](local-data.md) 说明存储、归一化、索引、迁移、备份与恢复边界。验收使用独立磁盘测试库；浏览器适配器不能证明持久化。

## 自动检查

`npm run check` 包括前端数据流程与 Rust 数据测试；`npm run test:e2e` 覆盖两类词条及现有骨架回归。

数据测试覆盖空白、长度、控制字符、Unicode、NFC/大小写去重、全角不合并、分类NULL、纠错冲突、过期编辑/删除及失败事务。迁移测试覆盖新库、升级、SQL失败回滚、校验不一致、未来版本、损坏/未知库保留、备份失败阻止升级、成功备份保留策略与独立恢复副本。

容量64队列、忙反馈、目录故障后重试与同目录独占均有测试。表单测试覆盖失败保留草稿、显式读取重复记录、搜索乱序丢弃、提交期间取消和重复写入保护、焦点返回。

## 桌面复验

准备环境并停止 Vite 后执行：

```powershell
npm.cmd run desktop:build -- --features desktop-acceptance
node scripts/test-dictionary-desktop.mjs
npm.cmd run test:desktop
npm.cmd run desktop:build
node scripts/test-dictionary-desktop.mjs --regular
```

带验收 feature 的程序使用随机UUID独立测试目录。三次启动验证新增、编辑、删除重启后仍然生效；第二实际进程被锁拒绝，悬浮窗词典调用被拒绝。测试只结束自己启动的进程。普通构建最后恢复；`--regular` 只读检查用户数据，不改写词条。

## 查询与性能

```powershell
cargo test --manifest-path src-tauri/Cargo.toml dictionary_query_benchmark -- --ignored --nocapture
```

独立库分别包含1,000、10,000和100,000条记录。测量分页、末页、中文/英文包含搜索、无结果和FULL提交写入；EXPLAIN检查精确唯一键与排序索引。COUNT与分页计入完整Repository时间，首查不是OS冷缓存承诺。

10,000条的常规查询/写入p95预算50ms、包含搜索100ms；最近组合候选在当前电脑上通过。100,000条仅为压力观察，不作为承诺容量。原始机器报告在本机保存，不用单机数字声明跨设备SLA。

真实桌面另外测量IPC往返、数据线程排队/执行与界面防抖/渲染。各层p95不可简单相加或相减；一次端到端观察不是p95。日志不记录词条正文。

## 验证边界

当前实机为1120×760、Windows150%，无横向溢出，白色主体和中性文字。100%/125%、多显示器、断电/系统崩溃、安装/卸载/签名与跨电脑性能未验证。托盘共用处理器有实测，实体鼠标路径未自动控制。

没有ASR、实际纠错算法、自动学习、模型/密钥、录音历史、应用场景或云同步。通过数据验证不等于完成产品验收。
