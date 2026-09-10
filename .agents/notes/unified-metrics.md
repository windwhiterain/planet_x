# 统一「总结」= 步进函数的中间计算变量

> 状态 `[x]`（feature/unified-metrics） ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §16

> 问题：agent 除了直接状态，还看到很多**总结**（实力占比/霸权/联盟/制裁/交战、各势力城市·
> 舰队·人口·库存价值、世界总量）。这些总结其实就是**步进函数里算的那些中间变量**，不该由
> `--digest` 或别处**独立重算**一遍——否则容易与模拟漂移、重复劳动。

**已落地（`src/model.rs` / `src/sim.rs` / `src/agent.rs` / `src/main.rs`）**：
- `[x]` **`RoundMetrics` / `FactionMetrics` / `CityMetrics`**（`src/model.rs`，均
  `#[derive(..., JsonSchema)]`）：一回合的总结指标类型。存量/政治：世界级
  `cities/ships/fleet_value/population`、`power_share`、`hegemon`、`coalition_members`、
  `sanctioned`、`wars`（交战对，无序），以及 `factions`（key=faction id →
  `FactionMetrics{city_count,ship_count,fleet_value,population,market_value,at_war}`）。
  **流量**：`factions[].production_value/production/upkeep/governance_cost/
  governance_coverage` 与 `city_production`（key=city id →
  `CityMetrics{population,loyalty,production_value,production}`）。
- `[x]` **`RoundFlow` / `GovernanceFlow`**（`src/model.rs`）：一回合的**流动性中间量捕获**——
  `advance` 在步进时把产出/维护/治理写入它并返回（它是 `advance` 的返回值），使这些量与
  模拟逐回合一致，而非事后从状态反推。
- `[x]` **`sim::advance(state, config, rng) -> RoundFlow`**：在 `step_production`/`step_upkeep`/
  `step_governance` 里把「中间量」记入 `RoundFlow`（每城每资源产出、每势力每资源产出、
  舰队维护费、治理总开销/覆盖率），并在回合末随 `flow` 返回。
- `[x]` **`sim::round_metrics(state, config, &flow)`**（`src/sim.rs`）：**唯一权威**的总结聚合器，
  复用步进函数本身的计算——一次 `balance_picture`（内部 `faction_power_share`+`coalition_of`）、
  一次 `sanctioned_hegemon`、一次 `war_pairs`，再补世界/各势力的城市/舰/兵力/人口/库存价值，
  并把 `flow` 里的产出/维护/治理并入。纯函数、无 RNG，同种子完全复现。
- `[x]` **agent 视图带 `metrics`**（`src/agent.rs`）：`Trajectory` 增加 `pub metrics: RoundMetrics`；
  `state_json(state, config, &flow)`/`render_state(state, config, &flow)` 带 flow，内部调
  `sim::round_metrics`。因此 agent 在每个回合 JSON 里**同时看到直接状态 + 存量总结 + 流量
  总结**，且 schema 自描述（`agent_view_is_self_described_by_schema` 增加对 `metrics` 及
  流量字段的断言）。回合 0（起点未步进）用 `RoundFlow::default()`（流量为 0）。
- `[x]` **`--digest` 与逐回合视图同源**（`src/main.rs`）：`run_digest` 在窗口末态调用
  `sim::round_metrics(state, config, &flow)`，`digest_value` 直接读它（删除重复的
  `active_war_pairs` 与独立的势力/世界聚合逻辑）；并**逐回合累计**各方产出，
  `factions[].production` 为窗口总量。digest 也随之**新增**
  `population/market_value/at_war/sanctioned/production/upkeep/governance_cost/coverage`。
- `[x]` **`-0.0` 规整**：`r2`/`round_value` 加 `+ 0.0`，避免 `f64::round` 保留的负零让 agent
  看到 `-0.0`。
- 验证：`cargo build` 无 warning；`cargo test --lib` 39 passed；`cargo test --test longhorizon`
  5 passed / 4 ignored（含 `same_seed_reproduces_identically` 现用末回合 flow 渲染，验证流量
  同样可复现）。smoke（`--seed 7`）：`--round 5` 的 `metrics.factions[3]` 有
  `production={硅/碳/铁}, production_value=64.75, upkeep=23.5, governance_cost=4.8,
  governance_coverage=1.0`；`city_production[0]` 有 `production_value=28.4`；
  `--digest 12` 的 `factions[].production` 是窗口累计值（如 美国=347.45）。

**候选（留待后续）**：
- `[ ]` 把治理中间量（`governance_total`/`coverage`/距首都距离）也并入 `RoundMetrics`——给
  agent 一个「帝国为何要崩」的预警阅读，代价是每回合多一遍治理公式。
- `[ ]` Web（玩家界面）要不要共享同一份 `round_metrics`（现在刻意不动它）——若玩家也想要
  「世界一目了然的概览面板」可复用，但形状是面向玩家，另行设计。
