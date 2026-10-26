# 本地词典数据层

当前数据库版本为schema10。持续草稿以增量迁移保存块、来源和处理尝试关联，继续由唯一DataWorker与既有归档所有者处理；旧记录保留。迁移后回退使用能够读取schema10的当前构建，不用旧程序或旧数据库覆盖新资料。具体契约见 [持续草稿](revisable-draft.md)。

本地识别开发分支使用 schema11：云端和本地配置共享默认关系，本地行不保存 URL 或凭据；历史另存本地资产及引擎身份。迁移保留既有配置 ID、凭据引用、默认选择、词典和历史，先在独立资料验证；不以日常正式库试迁移。schema11 资料不能用旧 schema10 程序降级打开，代码回退不覆盖用户资料。模型文件状态由资产所有者核验，数据库记录不证明文件完整。

本页规定手动词典和共享 SQLite 数据层的行为。模型配置与录音识别、历史存储分别见 [模型配置](models.md) 和 [语音与历史](speech.md)；自动入词与表达学习见 [个性化](personalization.md)，账号与同步尚未接入。

## 数据与职责

应用层组装 DictionaryPort；页面仅使用该端口；Tauri 适配器是唯一原生调用入口。Rust 命令先校验调用窗口，再交给词典服务／Repository。容量 64 的队列由专用数据线程消费，连接只归这个线程。Runtime v1 的内存快照不保存词典内容，窗口／演示状态不依赖数据库锁。

接口提供 status/retry、list、get、create、update、delete。get 是重复／冲突反馈的读取入口，用户明确选择后才加载已有记录。更新及删除携带记录版本；冲突不覆盖，保存成功只在提交后显示。搜索有请求代次保护，写入没有自动重试；刷新窗口后重新读取已提交状态。

原生命令只接受 main 调用，voice-overlay 被拒绝。不接受路径、SQL、shell 或网络地址。浏览器适配器仅内存存储，刷新重置，页面明确标注；不可用于证明持久化。

## 文件位置与进程锁

Tauri app_data_dir 下：普通发布版 `data/dictionary.sqlite`，开发版 `data-dev/dictionary.sqlite`。只有专用验收构建打印数据目录；普通诊断日志不记录绝对用户路径。Windows 当前应用标识为 `local.sayweave.skeleton`，不要直接改名而丢失数据入口。数据库不在项目源码、安装目录或浏览器存储中。

每个目录用 `instance.lock` 的操作系统锁保护，文件存在不代表锁已占用。第二个同目录实例提示已运行后退出，不写库、不执行迁移。锁由数据线程持有，进程退出释放。初始目录不可写时不阻止演示窗口；恢复权限后可重试初始化。

测试使用独立临时目录。只有编译了 desktop-acceptance 的测试二进制接受合法 UUID 的 SAYWEAVE_DATA_ACCEPTANCE_ID，将数据放在项目 `.local/data-acceptance/<uuid>/`。普通构建不包含这条目录覆盖路径。

## 模型与归一化

两张 STRICT 业务表：dictionary_terms（term、normalized_term、category/category_key），correction_rules（heard、corrected、normalized_heard、corrected_key）；另有 STRICT schema_migrations（版本、名称、SHA-256、应用时间）。所有记录使用 UUID，revision 从 1 递增，创建／修改时间为 UTC 毫秒。文本与派生键同事务写入。

归一化 v1：Unicode 首尾空白裁剪 → NFC → 无区域依赖的小写转换 → NFC。显示保留原始写法；Figma/figma 重复，组合重音与预组合重音重复；不合并无重音、全角／半角或简繁体，也不是完整 Unicode case folding。Rust 工具链与 unicode-normalization 锁定，跨语言 fixture 检查关键样例。以后更改算法或 Unicode 行为需要新迁移、检测键冲突。

词条、听成、改为：单行 1–128 Unicode 标量字符；分类最多 64，空值为 NULL。控制字符与 NUL 禁止，内部空格保留。两种词条分别去重；纠错目标不是专有名词外键。同一个全局 heard key 只能有一个目标，重复新增不静默覆盖。以上纠错字段仅保留旧记录的存储兼容，产品不再提供映射管理，文字处理也不使用这些映射。正常词条的自动入词与来源见 [个性化](personalization.md)。

删除是当前库物理删除，不清除 SQLite 空闲页、系统备份或升级备份中的副本，不表示安全擦除。

## 查询与索引

| 查询 | 对应约束／索引 |
|---|---|
| ID 读取、编辑、删除 | 主键 |
| term/heard 归一化重复 | 各表唯一键 |
| 最近修改分页 | updated_at DESC, id ASC 联合索引 |
| 包含搜索 | 参数化 instr；扫描对应搜索字段 |

每页 50，COUNT 和列表在同一读事务。页码越界校正，搜索变化／成功写入回到第一页。百分号和下划线按普通文字搜索。无 FTS、无未使用分类索引；普通 B-tree 不负责任意包含查询。单连接 DELETE journal、FULL synchronous、5 秒 busy timeout，无连接池。以后增加并发读写时再基于测量评估 WAL。

## 迁移、备份与恢复

迁移 SQL 在源码内嵌入、随 Git 提交；已应用文件不改写。启动检查账本版本／名称／校验值和 quick_check。发现未来版本、校验不一致、未知数据库或损坏，关闭词典访问并保留文件；没有删库重建或自动降级。

新账本的 SHA-256 以 LF 换行计算；校验同时接受旧版本按 CRLF 计算的同一份 SQL。只兼容这两种换行表示，不改变已有账本，也不忽略其他内容变化。Git 检出方式改变后，已保存的数据仍可正常打开。

已有库升级：SQLite Backup API 创建 `.protected.sqlite` → 检查备份 → 事务应用所有待办迁移及账本 → 提交并检查 → 将备份标记 `.successful.sqlite`。保留最近三份 successful；失败的 protected 备份不自动删。首次空库不备份。备份创建／验证失败不启动迁移；迁移中断通过 SQLite 事务回滚。备份是本机明文文件，随用户目录权限保存。

人工恢复先退出所有同目录实例，保留当前故障文件，复制候选备份到另一临时目录，使用兼容版本检查 quick_check 和迁移账本，再验证词条内容。确认后由后续明确恢复操作替换工作副本；本轮不提供一键覆盖恢复。不要直接复制仍打开的数据库，不把新版库交给旧版程序写入。测试包含独立副本恢复后词条可读、失败回滚、未来版本与校验拒绝；不是自动恢复功能。

用户错误码：invalid、duplicate（带 existingId）、notFound、conflict、busy、unavailable、forbidden。底层诊断记录操作类别、排队／执行微秒与 SQLite 错误码，不记录词条正文。数据故障不改变语音演示状态。

## 开发与验证

环境准备见 [开发说明](development.md)，使用标准 npm／Cargo 命令。

```powershell
npm.cmd run check
npm.cmd run test:e2e
cargo test --manifest-path src-tauri/Cargo.toml dictionary_query_benchmark -- --ignored --nocapture
npm.cmd run desktop:build -- --features desktop-acceptance
node scripts/test-dictionary-desktop.mjs
npm.cmd run test:desktop
npm.cmd run desktop:build
node scripts/test-dictionary-desktop.mjs --regular
```

桌面词典脚本只连接自己启动的 WebView2 验收端口；端口占用则报错，不结束其他程序。测试进程启动时隐藏辅助窗口，应用窗口正常显示，结束后断开测试连接并结束本脚本所属进程。`--regular` 仅检查普通构建的可用性，不新增或删除用户词条。生产二进制没有开发组件路由及测试数据目录覆盖。无需安装包验证。

测试方法和覆盖边界见 [测试说明](testing.md)。普通词条用于识别热词与文字整理；自动入词和应用偏好见 [个性化](personalization.md)。旧纠错映射不参与运行时索引、优先级、有效词典指纹或模型请求，已有数据库记录保持原样。同步尚未实现。

## 取消整理的恢复边界（开发分支）

当前开发分支的取消归档在数据库事务中裁剪原文、处理单位与整理正文，并登记录音清理。取消只保存有提交／交还收据的前段；可能修改但无法确认的范围明确保留为恢复记录。未提交生成请求及其候选正文不进入取消后的历史。

数据层在有界队列准入前登记受限的取消恢复日志，位于当前用户数据目录的cancel-recovery，文件身份为已登记UUID。内容只有本次合法保留的识别数据和收据，不含目标软件正文、剪贴板、密钥或提示词。拒绝重解析点、非普通文件、身份不符与超限内容。数据库线程及重开在读之前回放；完整处理收据先于原文后备裁剪，事务已提交而日志尚未删除可安全再次结算。它只恢复存储，不重新向外部应用输入。

数据库忙、只读或队列满时，保留待恢复取消意图，旧记录在恢复完成前不通过读取路径曝光。损坏或部分日志导致明确恢复错误，不按猜测删除或展示旧取消正文。全部存储不可写时只能报告失败，不能承诺持久化清理。原生文字与SQLite仍不是原子事务；保存失败不会撤回目标文字。
