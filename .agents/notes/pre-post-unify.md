# 读面统一：只有 `pre` 和 `post`

> 状态：**[x] 已落地**（`feature/pre-post-unify`，worktree `planet_x-roundview`）。
> 相关：[`unified-metrics.md`](unified-metrics.md)（上一次「总结 = 步进中间量」的合并，本篇是它的
> 收尾）、[`engine-data-plane.md`](engine-data-plane.md)（四个读面与 `pre` 段的诊断）、
> [`schema-query-architecture.md`](schema-query-architecture.md)（四套并行投影的总账）。

## 1. 问题：两个名字正交于「什么时候」

旧模型里，**一回合的派生数据**被切成两段，装在一个叫 `Derived` 的结构里：

```rust
Derived { flow: RoundFlow, metrics: RoundMetrics }   // 同一个结构既当 pre 也当 post
```

* `flow` —— 本回合**过程量**（产出/维护/治理/贸易/判定流水，`RoundSink` 不落持久状态）；
* `metrics` —— 回合末的**观测/总结**（世界总量、实力占比、霸权、每势力聚合）。

而 `pre`/`post` 是**什么时候**算的：推进前 / 推进后。这两组概念**正交**，于是：

* **读的人读不懂**：`flow` 与 `metrics` 到底是什么关系？「`pre` 里 `flow` 恒空、`metrics` 有内容」
  这件事本身就在提示：这两个名字不是一对时间标记，而是两个**种类**——没有任何一个名字说了这件事。
* **同一个数存了两份**：产出同时躺在 `flow.faction_production` 与 `metrics.factions[].production`；
  市场同时躺在 `flow.market_net` 与 `metrics.market_net_import`；运费/承运费/治理也各有一份「map 版 +
  每势力标量版」。两份存放 = 两个读面可能各说各话。
* **这个坑真的咬过**：`projection.rs` 里留着一条长注释——零城势力没跑治理步骤，`flow` 里**没有键**，
  而 `metrics` 必须给缺省值，于是 `flow.jsonl` 说「治理覆盖 0%」（读起来像崩了）、同回合
  `metrics.factions[].governance_coverage` 说 100%。**missing ≠ 0**，两套缺省约定必然打架。

## 2. 修法：一段数据、两个时间槽、同形

```rust
pub struct RoundState { schema_version, state, pre: RoundView, post: RoundView }

/// 一回合的**视图**：pre（回合开始）与 post（回合结束 + 本回合过程量）用**同一个类型**。
pub struct RoundView {
    // ── 观测（pre/post 都有）──
    city_count, ship_count, fleet_value, population,          // 世界总量
    power_share, faction_power, hegemon, coalition_members, sanctioned, wars,
    market_price, market_settled, market_offered,
    factions: BTreeMap<FactionId, FactionRow>,                // 每势力一行
    cities:   BTreeMap<CityId, CityRow>,                      // 每城一行
    // ── 过程（只有 post 有；pre 里是 0 / 空）──
    decisions: RoundDecisions,                                // AI 这一回合选了什么、为什么
}
```

三条规矩：

1. **读的人只要记一条**：`pre` 是回合前、`post` 是回合后；`post` 比 `pre` 多的就是「这一回合发生了什么」。
2. **两个槽同形**：过程量在 `pre` 里是 0 / 空——**不做** `skip_serializing_if`（那会让「pre 少几个键」
   变成第二条要记的规矩）。代价是 `pre` 里那几个 0 要靠文档说清楚，收益是 Python/前端不必写
   「键可能在不在」的分支。
3. **每个数只有一个位置**：观测与过程**同处一行**（`FactionRow` / `CityRow`）。`flow.faction_production`
   与 `metrics.factions[].production` 合并成 `view.factions[].production` 一个位置 ⇒ §1 那个
   「两个读面各说各话」的坑**结构上不可能再发生**。

引擎内部只留一个**写入口袋** `RoundSink`（各 step 往里写，形状是「按 step 分组的 map」），回合末由
`observe(state, config, &sink) -> RoundView` 折成视图；`view_from_state(state, config)` = 喂一个空 sink
的那份（回合 0 / `--start` 载入 / 任何「只看世界、不推进」的场合）。

## 3. 读面映射（老 → 新）

| 老 | 新 |
| --- | --- |
| 类型 `Derived` | **`RoundView`** |
| `Derived.flow` / `Derived.metrics` 两段 | 拍平：`view.<字段>` |
| `RoundFlow`（读面的一半） | **`RoundSink`**（引擎内部的写入口袋，**不在读面里**） |
| `derived_from_state(state, cfg)` | **`view_from_state(state, cfg)`** |
| `round_metrics(state, cfg, &flow)` | **`observe(state, cfg, &sink)`** |
| `flow.decisions` | **`view.decisions`**（顶层） |
| `flow.faction_production` / `flow.upkeep` / `flow.governance` / `flow.market_freight` / `flow.market_carrier_income` / `flow.market_net` | **`view.factions[].production` / `.upkeep` / `.governance_cost`+`.governance_coverage` / `.freight_paid` / `.carrier_income` / `.net_import`** |
| `flow.city_production` + `metrics.city_production` | **`view.cities[].production`**（每城一行：population/loyalty/production/production_value） |
| `metrics.cities` / `metrics.ships` | **`view.city_count` / `view.ship_count`** |
| `metrics.market_net_import` | **`view.factions[].net_import`**（那张独立表没了） |
| `--derived` 的 `{flow, metrics}` | `{round, source, pre, post, [note]}`——`pre`/`post` 是**拍平的一整份视图** |
| `idx/flow.jsonl` / `idx/city_flow.jsonl` | **`idx/faction_process.jsonl` / `idx/city_process.jsonl`**（列名与含义不变，只是不再是「flow」） |
| `main.jsonl` 每行的 `metrics` 键 | **`view`** |
| 每回合轨迹（`Trajectory`）的 `metrics` 字段 | **`view`** |
| `SCHEMA_VERSION = 13` | **14**（世界状态 `State` 的字段**一个没动**，变的是派生读面；`migrate` 里 `13` 一档推号即可） |

`derived.control` / `derived.scope` / `derived.decisions` / `derived.blueprints` 四张表、`--control`
读面、`State` 的世界字段、`schema.json` 的 `lazy` 段**都没动**。

## 4. 验收（实测）

| 判据 | 结果 |
| --- | --- |
| 行为中性：`--seed 42 --round 240 --digest 20` 的 SHA-256 | **`657F2DC9…66665`，与重构前逐字相同**（取 `^{` 行、`\n` 连接、UTF-8 无 BOM） |
| 全档 `cargo nextest run -p planet_x -P full` | **189/189 通过，27.4 s**（`[profile.test] opt-level=2` 之下） |
| web：`cargo nextest run -p planet_x_web` | 24/24 通过 |
| `cargo check --workspace --all-targets` | 零 error、零 warning |
| 读面契约（`tests/projection_derived.rs`） | 4/4 通过——而且**两条翻译函数（`res_map` / `gov_of`）被删掉了**：它们存在的唯一理由就是补两套缺省约定的差，现在两个读面读同一份视图，直接逐值相等 |

## 5. 没做的（下一步的料）

* **`pre` 面还是「回合开始时的观测」**：真正的「AI 这一回合**看到/掷出**了什么」（逐舰解算顺序的
  洗牌、各处 `derived_roll` 的骰子、`deterrence`、`war_strength`）**仍然没有记录**——那要改
  `advance` 的返回（让它同时产出 pre 面），见 [`engine-data-plane.md`](engine-data-plane.md) §7.4。
* **一张 36 条候选的「step 中间量」清单已经盘出来了** → 现在**整份落在
  [`step-intermediates.md`](step-intermediates.md)**（每条带 `文件:行号`，已在 `main` = `7e11d32`
  上复核过；粒度、是否吃骰子、能回答什么问题都在表里），
  按批次排好：B1 治理/忠诚（「为什么这座城忠诚在掉」+ 迁都判据）、B2 钱去哪了（预算实花 vs 批的、
  造舰进度 vs 产能、维护费欠费的**生锈比例**）、B3 市场与运输（价格分解/丢货量/禁运三档原因/
  在途状态）、B4 战斗（逐发索敌计划含**玩家舰**、命中折减、点防）、B5 上面那个 `pre` 面。
  新增量一律沿本篇的形状：**观测与过程同处一行**、纯追加（不改行为 ⇒ digest 逐字不变）。
  `unified-metrics.md` §候选第一条（把治理中间量并入读面，给「帝国为何要崩」的预警）就是 B1。
  → **B1 已经落地**（`feature/step-intermediates-b1`：`view.cities[].loyalty_target` 四项分项 +
  `view.factions[]` 的行政/娱乐拆分、人口超载倍率、思潮忠诚惩罚、`capital` 迁都判据），
  digest 逐字不变、全档 192 绿——见该篇 **§6.1**。
