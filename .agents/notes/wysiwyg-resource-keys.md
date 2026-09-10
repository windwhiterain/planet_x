# 权威 schema 贯彻：WYSIWYG 资源 key + 拆掉 `AgentState` 镜像（在 auth-schema worktree）

> 状态 `[x]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §14

> 用户拍板（方案 X）：**资源 key 直接用可读中文名（WYSIWYG），不再有 raw key ↔ 显示名翻译**；
> 并且「既然都是 jq 查询了」，**拆掉手写的 `AgentState` 镜像投影**，让 agent 视图直接复用
> 权威模型类型（单一来源、schema 自描述、无镜像）。Web 仍为玩家界面，不动。

- `[x]` **资源 key 全局改成可读名（WYSIWYG）**：`config/game.ron` 的 `resources` 表、各
  `build_cost`/`cost`、story `GrantResources`；`world.rs` 的 `deposit`/`stockpile`；
  `sim.rs`/`longhorizon.rs`/`web.rs` 测试里的资源字面量——全部由 raw（`water_ice`/`iron`…）
  改成中文名（`水冰`/`铁`…）。`config.resource_name` 变成恒等，`meta.resources` 的
  raw→中文 翻译表**删除**；agent 看到的资源 key 就是它在 `--apply` 里能写的 key。
- `[x]` **拆掉 `AgentState` 镜像，agent 视图 = 权威 `Trajectory`**：删除
  `AgentState`/`AgentFaction`/`AgentBody`/`AgentOrbit`/`AgentSettlement`/`AgentDeposit`/
  `AgentCity`/`AgentBuilding`/`AgentShip`/`AgentOrder`（10 个镜像 struct）+ `from_state` +
  `game_event_value`/`faction_name`。改为一个
  `#[derive(Serialize, JsonSchema)] pub struct Trajectory`，字段直接用权威类型
  `Vec<Body>/Vec<City>/Vec<Faction>/Vec<Ship>` + `Vec<GameEvent>/Vec<ChronicleEntry>`
  （不含控制面 `control`/`scope`）。`schema_value()` 用 `schemars::schema_for!(Trajectory)`
  ——schema 是权威类型派生的自描述，与发射 JSON 同源、不会漂移。
- `[x]` **给模型类型加 `#[derive(JsonSchema)]`**：`Body`/`Orbit`/`Settlement`/`ResourceDeposit`/
  `City`/`Building`/`Faction`/`Ship`/`GameEvent`/`ChronicleEntry`。新增 `use schemars::JsonSchema;`。
- `[x]` **守卫测试**：
  - `meta_value_covers_every_config_section`：`meta` 必须覆盖每个 config 段（key 集合是
    config 的超集）——拦住「配置加了字段、meta 漏了」这类漂移（就是 `component_spill` 那类）。
  - `agent_view_is_self_described_by_schema`：`state_json` 发射的顶层 key 都在 `schema_value()`
    的 `properties` 里，且实体数组被声明——schema↔发射一致性显式化。
- `[x]` **验证**：`cargo build` 全目标无 warning；`cargo test --lib` 39 passed（37 + 2 守卫）；
  `cargo test --test longhorizon` 5 passed / 4 ignored。smoke（`--seed 7`）：`--round 12`
  的 faction[0]（联合国）资源 key 是 `氦-3/碳/铁`（中文）、relations 按 id、events 带
  `type`；`--schema` title=`Trajectory`、顶层 key 与发射一致（bodies/chronicle/cities/
  events/factions/round/ships/time_month）；`--traj` pack（traj_len=7、story_len=4、
  meta.resource_value 存在）。city/ship 用权威字段（body_id/faction_id/loyalty/buildings/
  components/class）。
- `[ ]`（须知）**行为变化**：agent 视图不再预计算派生字段——舰的 `panel`（attack/range/
  speed/upkeep）、城 `armor`/`gov_distance`/`owner_name`/`body`名、`relations` 从「按名字」
  变「按 id」、`events`/`chronicle` 保留。agent 需用 jq 现场 join/计算（`--meta`/`--schema`
  提供规则与字段；若要省事，可后续加几个 Layer-2 语义视图命令）。
