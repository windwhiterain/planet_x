# 语义化「视图/工具」API（把裸 jq 降为逃生舱）

> 状态 `[ ]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §12

- `[~]` **命名语义工具集**（对应设计文档 Layer 2）：把 agent 主界面从「裸 jq 探 JSON」
  升级为稳定、有版本、参数化的语义操作，由模拟自己生成数据（复用 `sim.rs` 现有的
  `balance_picture`/`governance_distance` 等），不要求 agent 知道底层 JSON 树。候选：
  `view sitrep` / `view economy <faction>` / `view fleet <faction>` / `view frontier`
  （边缘失稳城）/ `view threat`（威胁评估）/ `view market`。`q`/`--query` 降为逃生舱。
  **已落地一个零改动 Rust 的 PoC**（`play/jq/view_sitrep.jq`，另加 `view_profile.jq`）：
  把一回合 33 KB 的裸 JSON 压成 **956 字节**的语义 sitrep
  `{round,world{wars,factions[],top{share}}}`（wars 从 relations≤-20 推断、top 取城数最多者
  及其占比），压缩 ~34×，agent 无需手写 join/计算、也不受 PowerShell 引号坑影响。
  Rust 侧价值更大：把 `top_share` 换成真实 `balance_picture`、`frontier` 用
  `governance_distance`/`loyalty` 列出边缘失稳城、`economy` 用 `resource_value` 加权库存，
  并做成 `--view <name>` / `view <name>` 命令（带参数校验、版本化）。
- `[ ]` **工具层校验**：语义工具做参数校验（如 `order 0 attack 99` 明确报「目标舰不存在」），
  把错误在工具层显式化，而不是丢给宽容的 jq。
- `[ ]` **Layer 3 方向（长期/可选，非现在必须）**：ECS 组件化存储 + 事件溯源投影，把
  「资源/建筑/舰级用字符串键+配置表」这套数据驱动模式推广到实体属性；新增机制=加组件
  +加 system/投影，不动既有结构体。投入大、破坏 `.ron` 可读性，仅当扩展需求真正爆了才做。
