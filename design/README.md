# Sayweave 当前设计入口

当前快照：**2026-10-03 v0.1**。首页保持 T022 暂定结果，其余页面保持 T023；T024 保存本地资料并接入项目 Skill，未改画布。项目非 Git、无业务实现，整体设计仍待逐页评审。

## 本地资料

- [离线浏览入口](exports/2026-10-03-v0.1/index.html)：页面、弹窗、长表单、语音状态及品牌图标，可搜索节点。
- [完整 Figma 源文件](exports/2026-10-03-v0.1/source/Sayweave-v0.1.fig)：三个页面的官方本地副本；验证范围见交付说明。
- [交付说明](exports/2026-10-03-v0.1/specs/delivery.md)与[文件清单](exports/2026-10-03-v0.1/manifest.json)。
- [当前规范维护源](../DESIGN.md)；[冻结详细规范](exports/2026-10-03-v0.1/specs/visual-spec.md)包含配色、透明度、承托背景、字体、尺寸和材质。
- [组件与布局](exports/2026-10-03-v0.1/specs/components.md)、[点击与滚动](exports/2026-10-03-v0.1/specs/interactions.md)、[语音动效](exports/2026-10-03-v0.1/specs/motion.md)。
- [规范与实际差异](exports/2026-10-03-v0.1/specs/differences.md)：实际标题行高、未设计状态、实机验证事项。

## 在线设计

[设计文件](https://www.figma.com/design/q8AclFa2pVUWTShfpovPHJ?node-id=18-2339) · [首页原型](https://www.figma.com/proto/q8AclFa2pVUWTShfpovPHJ?node-id=18-2340&scaling=scale-down&content-scaling=fixed) · [Figma 状态与历史](figma-status.md)

页面 0:1 为首页与组件，18:2339 为功能页面，19:135 为首次使用与输入状态。原始首页 3:227、原型入口 18:2340。实际页面/节点数量由快照清单提供，不能把组件与演示状态的数量当成独立功能数。

## 使用与维护

设计按当前原生画布导出，规范值和实际覆盖值分别记录。后续修改先查差异清单，不擅自归一化已暂定画面。更新后创建新日期快照，并更新本入口、DESIGN.md 和现有设计 Change。静态资料可离线查阅；编辑 .fig 或运行在线原型仍需要 Figma。语音、API、下载与系统输入均为演示。

## 历史资料

candidates/、早期 ImageGen 提示和 design/figma/ 各轮记录保留为演进证据。第三轮概念图不再代表当前精确布局、字阶和交互。旧 720×64 悬浮条仍保存在 Figma 中，当前基础语音以 71:2985（17 变体）、142:462（11 个减少动态变体）为准。

[产品界面与模式](product-surfaces.md) · [流程与需求](flows.md) · [参考来源](references/README.md)
