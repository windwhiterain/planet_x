# Agent 游玩体验优化（游玩摩擦点）

> 状态 `[~]`（见 `agent-play-polish.md`：闭环视角已做，语义指令助手仍未做） ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §18

> 本轮实测（`--seed 7`，agent 用 `--index` + `planet_xq` 分析、`--apply` 下指令）暴露的
> 「agent 玩起来难受」的摩擦点。按「痛点强度 × 影响面 × 改动量」排序。多与 `agent-control-api.md` / `semantic-view-api.md` / `coarse-trajectory-views.md` /
> `lazy-index-pandas.md` 已有的候选点子重叠，这里补**闭环**视角（读完→决策→下发→观察→复现）与**语义护栏**。

### 18.1 指令下发太难写（最高优先级）
agent 为下一次 `--apply` 决策，要手写 `control/scope` 且背一堆整数 id（`city`/`building`/
`ship`/资源 key）。实测为「让中国在地球造战列舰」要查出 building id=3=船坞、city id=0、
`ship_type` 是 `battleship`…… 纯手搓，易错。

- `[ ]` **语义指令助手**：把 `--apply` 的叶子 diff 生成包成**意图式**高层命令，由 Rust 解析成
  叶子（复用 `agent-control-api.md` 的 `order-fleet-to-hold <body>` / `invest-loyalty <city> <budget>`）：
  - `--fight <faction> <enemy_faction>`：把该势力现有舰全部 `TargetShip{attack:true}` 指向最近的
    敌对舰（`pick_target` 已有逻辑）。
  - `--build <faction> <ship_class> [<city>]`：给该势力（某城）的某个建造区 `ship_type` 设为该
    舰级（+ `build_weights` 加权）。
  - `--colonize <faction> <body>`：下令 `Colonize{body}`。
  - `--hold <faction> <body>`：把舰队调去守卫/驻守该天体（复用 `dock`/`guard`）。
  - `--loyalty <faction> <city> <value>`：`loyalty_budget`。
  - 原则：**每个命令只出它改的那几片叶子**，返回「实际落地的 diff + 每条的稳定可读 id」，让
    agent 不用记整数 id 与 JSON 树。
- `[x]` **`--schema-control`（落地为 `--control-schema`）**：`--schema` 现只描述 `Trajectory`
  （状态视图），**没有描述控制面/diff**（`CommandReq`/`FactionControlPatch`/`scope`/behavior 的
  两种写法）。给控制面也派一个 JSON Schema（`schemars` 派生，同 `schema-query-refactor.md` P1 思路），agent 据此写
  diff 而不是靠背。落地：`web.rs` 给 `CommandReq`/`FactionControlPatch`/`ShipOrderPatch`/
  `BudgetPatch`/`InvestWeightPatch`/`BuildWeightPatch`/`LoyaltyBudgetPatch`/`BuildingPatch`/
  `ScopeView` 加 `#[derive(JsonSchema)]`，`model.rs` 给 `ControlMode`/`ShipBehavior` 加
  `#[derive(JsonSchema)]`；`web::control_schema_value()` = `schemars::schema_for!(CommandReq)`；
  `main.rs` 加 `--control-schema`（与 `--schema` 镜像：读面 vs 写面）。**注释跟着 `///` 走**：
  补了字段 doc comment 的，schema 自动带 `description`（agent 视图 `--schema` 同理——`round`/
  `time_month` 等没写 `///` 的字段就没有 description）。

### 18.2 预算语义黑盒、无「成本→收益」预览
实测把 `construction_budget` 拉满 → 维护费 79/月 > 产出 65/月 → 经济崩、共和国溃散（第 15 月
城清零）。agent 只能靠试错才知道「这个预算养得起几艘舰」。

- `[x]` **`--control-plan <faction>`**：给当前控制面算**稳态剖面**——该势力在此预算/权重下
  「每回合产入 vs 维护 vs 治理开销 vs 可造舰上限」，并给出 `fleet_value`/`upkeep`/`city_count`
  的近似平衡点，让 agent 在写 diff 前看到代价，而不是提交后观崩盘。落地：`sim::control_plan`
  ——**干跑一轮真实 `advance`**（克隆 state + 固定 RNG 种子，产出/维护/治理都不吃 RNG），
  取模拟自身上一步的 `RoundFlow` 经 `round_metrics`，零漂移；另读 `read_budget`（含造舰
  维护保留上限）+ 报告 AI 保守上限以对比是否过度投入。字段：`production_value/upkeep/
  governance_cost/net_flow/stock_market_value/construction_budget_value/investment_budget_value/
  ai_construction_cap/over_committed_construction/fleet_upkeep_cap/fleet_overextended/
  rounds_before_insolvent/verdict(healthy|over-committed|bleeding)`。`main.rs` 加
  `--control-plan [<faction>]`（**不带参数 = 给所有势力各出一份 map**，带参数 = 单势力；可与
  `--apply <diff>` 连用：先叠候选 diff 再看后果）。
  `sim::tests::control_plan_balances_and_flags_over_committed_construction` 守卫。实测：把
  construction_budget 拉满（mode=Player）→ round 0 即标 over-committed，run 到 r16 upkeep
  67.5 > 产出 64.8、库存跌破 0、城 4→1（正是本节报告的溃散）——预览在提交前抓到它。
- `[x]` **预算语义入 `--meta`**：`meta.economy` 已有 `upkeep_reserve_mult`/`invest_fraction`/
  `production_rate`；把「造舰预算 = 库存×invest_fraction，但先留 upkeep×4」这一换算写进 `meta`
  的说明或示例，agent 能直接按公式推，不必反推代码。落地：`agent::meta_value` 增加顶层
  `notes` 数组（budget 换算 / 每回合净流=产出−维护−治理 及其后果 / 生产与治理的公式 /
  WYSIWYG 身份约定）。

### 18.3 从「aggregate」到「该干嘛」缺一座桥
`metrics` 告诉 agent 城数/产出/份额，但要"哪座城是我的短板（低忠诚/高治理距离）""哪个邻居是
我该抱团扁的霸权""我缺哪种关键矿物"，agent 得自己 join/手算。

- `[ ]` **语义视图命令（`semantic-view-api.md` Layer 2 落地）**：`--view sitrep` / `--view economy <faction>` /
  `--view fleet <faction>` / `--view frontier`（边缘失稳城：按 `governance_distance`+`loyalty`
  列出）/ `--view threat`（威胁评估：最近的敌对战力）/ `--view market`（富余/稀缺矿物）。由
  模拟直接生成（复用 `balance_picture`/`governance_distance`/`resource_value`），带参数校验与
  版本化，agent 不需知道 JSON 树。这是把「裸 jq 探 JSON」正式降为逃生舱的关键一步。
- `[ ]` **`--focus <faction>`**：把观察面收敛到某势力（其城市/舰/关系/预算），大幅降 token
  （现 `--index` 无此裁剪；§旧 REPL `control 3` 曾把 7578→1700 字符）。语义视图命令天然是
  单势力/局部视角，可一并做。

### 18.4 观察→决策→下发→观察 的闭环松散
要玩一段（30 月）需跨 4 条命令（`--index`/`--round`→写 python→写 diff→`--apply --save`），
且没有「当前计划」的持久化位置。

- `[ ]` **`--play <session>`（脚本化回合循环）**：一个会话目录/主键（`--start`+`--save`+一段
  `--round`），agent 每回合循环「读 metrics → 写 diff → apply → 存」；命令 `--play` 批量跑一段
  + 记录每步 diff（做成 `--traj` 同源的 steer 日志），让「讲一段被干预的故事」如 `--traj` 一样
  可复现、可检查。优先做一个**轻量脚本**：`planet_x --seed S --plan plan.jsonl`（plan 是
  「到某回合应用某 diff」的序列），一次性重放更省事。

### 18.5 长局 / 统计库（`lazy-index-pandas.md` 候选落地）
- `[ ]` **`planet_xq` 统计函数库**：`series(faction, metric, every)`、`rolling_mean/max`、
  `leader_rotation()`、`hegemon_timeline()`、`war_durations()`、`gini(stockpile)`、`frontier()`、
  `threat()`。让 agent 直接调用而非每次手写 pandas；这是「上千回合仍多极」这套调研的自然工具。
- `[ ]` **`--profile <K>`（`coarse-trajectory-views.md` 极致画像的 Rust 版）**：每 K 月一行极简标量时序
  `{round,cities,ships,fleet_value,hegemon,power_share,wars}`，几乎零上下文先看走势，再放大。
- `[ ]` **lazy 加重型字段**：`ships[].components/component_hp`、`cities[].buildings`、超大
  `events`/`chronicle` 也标 lazy（`lazy-index-pandas.md` 候选），让 agent 默认面更小。

### 18.6 已知「玩成什么样」的护栏该喂给 agent
agent（尤其想「称霸」的）会踩「单极→被联合制裁→反噬」。宜把设计护栏写进游玩文档/`--meta`
说明，让 agent 的决策与「上千回合多极博弈」这一目标对齐。

- `[x]` **策略护栏入文档**（**已做，见 `agent-play-polish.md` §26.2**）：`.agents/agent-play.md` §6.1「红线是量出来的」
  给出**实测阈值**（`upkeep/production_value > 0.8` 该收手、`power_share ≳ 0.5` 必招合纵），
  并用**同一 seed 的两条真实轨迹**（被动基线 vs 被引导）证明「两种死法是同一种过度」。
  §6 的定性四条也保留。

> 量化依据（`--seed 7`，`--index` + `planet_xq`）：①agent «中国» 野蛮扩张
> (`construction_budget` 硅3/碳9/铁10) → 13 月 `upkeep 79.5 > production_value 64.8` → 15 月城清零；
> ②稳重局（经济压住 + loyalty 2.0 + 防御）→ 120 月 11 城/份额 0.62 → 触发全球 coalition+sanction →
> 179 月崩回 1 城，世界仅 12 城/10 舰；③基线 AI 全程 20 城/48 舰、霸权在 矿联↔中国↔俄罗斯 轮换。
> 结论：**「玩得太好」会触发系统的均势反噬**——这正是主线想要的，agent 应学会在「份额涨」时
> 主动收手/制衡，而不是硬顶。
