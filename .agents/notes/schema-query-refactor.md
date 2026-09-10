# Schema/查询架构再造（降低「schema 查询维护」心智负担）

> 状态 `[ ]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §11

> 调研设计文档见 `schema-query-architecture.md`（诊断+分层方案俱全）。此节只列
> 「未落地的候选改造」，供下一步直接照做。**现状四痛点**：① `State` 真身之外还有 4 套
> 并行手写投影（`agent.rs` 的 `AgentState`、`meta_value`，`web.rs` 的 `MetaView` 等，
> control/diff 面）；② schema 隐式，agent 要背；③ query 引擎宽容（缺字段→null，漂移
> 静默失效）；④ 无 `schema_version`/迁移，旧 `.ron` 语义变化被静默错载。

- `[x]` **P0 修 `meta_value` 漂移（先止血）**（已落地，`src/agent.rs`）：`meta_value` 的
  `economy/combat/diplomacy/market/governance/mond/balance` 与 `structures/buildings/ships/
  components` 段改用 `config_json`（`serde_json::to_value` + 递归浮点圆整）从各 `GameConfig`
  子结构**直接派生**，删手写字段清单。**已知漂移**（已修）：`combat` 原来漏了
  `component_spill`/`component_repair`/`escort_range`/`pursuit_range`/`pd_radius` 这 5 个
  近期字段，现在都出现在 meta 里。整数型字段（`slots`/`min_members`/id）保持整数不被圆整。
  `resources`（raw key→中文名）与 `story`（平铺 trigger/effects）仍为刻意保留：前者是
  显示名翻译表、后者是真转换。新增守卫单测
  `agent::tests::meta_derives_all_combat_fields_and_keeps_ints`。**未做**：`web.rs` 的
  `MetaView` 仍是独立一套（可后续复用同源）。
- `[x]` **P1 schema 自描述**（已落地，`src/agent.rs` + `src/main.rs` + `Cargo.toml`）：
  `Cargo.toml` 加 `schemars = 0.8`（derive）；agent 视图类型（`AgentState`/`AgentCity`/…/
  `AgentOrder`）加 `#[derive(Serialize, JsonSchema)]`；`agent::schema_value()` 用
  `schemars::schema_for!(AgentState)` 生成 JSON Schema；`main.rs` 加 `schema [<jq>]` REPL
  命令。agent 从「背 schema」变「查 schema」（嵌套类型走 `$ref`/`$defs`）。`meta`/`state`
  顶层**未**塞 schema（避免每回合行肿胀），独立命令即可。
- `[x]` **P2 查询响亮失败**（已落地，`src/query.rs` + `src/main.rs`）：`query.rs` 加
  `apply_strict`/`apply_strict_lines`，strict 模式下 `Path` 访问不存在字段 / 越界索引返回
  `Err(unknown field …)`（而非 null）；CLI 加 `--strict`，REPL `q --strict <jq>`，默认仍
  宽容。新增 3 个守卫单测（未知字段报错 / 存在字段通过 / 越界索引报错）。让 schema 漂移
  立即可见、不静默。
- `[x]` **P3 版本化+迁移**（已落地，`src/model.rs` + `src/config.rs` + `src/world.rs`）：
  `State` 加 `schema_version: u32`（`#[serde(default)]`，旧档为 0）+ `SCHEMA_VERSION = 1`；
  `config.rs` 的 `load_state`/`load_checkpoint` 解析后调 `model::migrate(&mut state)` 逐档
  升级；版本过新则显式报错（宁抛错不错载）。`world::default_state` 写 `schema_version`。
- `[ ]` **P4 收敛并行投影**：把 `agent.rs` 的 `AgentState`/`AgentCity`/… 镜像 struct
  拆掉，改为对真身 `State` 用 serde 属性（`skip_serializing_if`/`rename`/`serialize_with`）
  做用户态裁剪；至少先做到「从真身 derive/生成、不手抄」。工作量最大，建议后续渐进
  （先拆 `AgentShip` 再拆 `AgentCity`…）。P1 已给这些镜像 struct 加了 `JsonSchema`
  derive——若最终拆掉它们，`schema_value()` 的生成目标也要换成新派生视图。
