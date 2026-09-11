# 四系统交叉审查：国内市场 / 预算系统 / 运输系统 / 国际市场

> 状态：`[ ]` **待修**。这是审查方的 P0/P1/P2 清单，给另一个 session 直接照做。
> 审查对象：`HEAD e849699`（`feature/domestic-market-welfare` 合流点）。
> 审查时验证：`cargo test --lib` = 200 passed / 2 ignored；`uv run --project play/planet_xq
> python play/tests/run.py all` = 四组全绿。**下面问题均未被现有测试咬住**。
>
> 关联：[`domestic-market.md`](domestic-market.md)、[`site-supply.md`](site-supply.md)、
> [`trade-and-sanctions.md`](trade-and-sanctions.md)、[`freight-collection.md`](freight-collection.md)、
> [`economy-depth.md`](economy-depth.md)。
>
> **给修复 session 的总则**
> 1. 先按 **P0 → P1 → P2** 修；每个修复都补一条**防空转守卫**（必须证明“这一局真的发生过 X”）。
> 2. 非必要不改设计裁决；必须改行为时，明确写“这是有意的行为改变”，并重跑/重标同一 seed 的 digest 基线。
> 3. 行号是审查时快照，改完用 `grep` 重新定位。
> 4. 全门验收至少：`cargo nextest run -P full`、`cargo nextest run -p planet_x_web`、
>    `uv run --project play/planet_xq python play/tests/run.py all`。
>    若动控制面，再跑 `uv run --project play/planet_xq python play/tests/g4_spec.py`。
>
> **行为改变预告**（先记在这里，免得下一个 session 以为是测试噪声）：
> * `P0-1` 会改变 site-supply 下的实际装货量：出口腿不能再装走本地保留量。
> * `P0-2` 只影响 `config.domestic_market.enabled = true` 的局。
> * `P0-3` 影响**默认开启**的星际市场，会改价格轨迹，进而改世界演化与 digest 基线。
> * `P1-2`/`P1-4` 也会改运输/市场行为，需单独做 A/B 或重标基线。

---

## P0-1 运输：`haul_load` 实际装货没有使用出口保留量

**位置**
* `src/sim/haul.rs:100-109`（实际装货）
* `src/autocontrol/freight.rs:260-276`（`exportable_at` 定义）
* `.agents/notes/site-supply.md` §2.1（保留量设计）

**现状**
`lanes` / `needed_haulers` / `route_for` 都按 `exportable_at(站点) = max(0, 现货 − 保留量)`
计算“这条出口腿有多少活”。但 `haul_load` 在非首都货栈装货时，拿的是**全量货栈库存**：

```rust
let mut avail = state.stock_at(&owner, from).cloned().unwrap_or_default();
// 只有 from == owner 首都时才按 site_deficit 收口；出口腿完全没碰 site_reserve/exportable_at
if from == state.capital_body(&owner) && to != from { ... }
let plan = haul_split(&avail, room);
```

**影响**
* 站点正在留着建楼/造舰的料，会被船一起装回首都；
* 下一回合 `site_deficit` 又把同一批货列为进口需求，船再运回来 ⇒ 往返乒乓；
* 与 `site-supply.md` 明写的“同一件货不可能同时出现在进出口两侧”直接矛盾。

**修法（建议最小改动）**
在 `haul_load` 里把“起点不是货主首都”的 `avail` 与该站点的 `exportable_at` 逐资源取交集：

```rust
if from != state.capital_body(&owner) {
    let exportable = autocontrol::freight::exportable_at(state, config, &owner, from);
    avail.retain(|rt, amt| {
        *amt = amt.min(exportable.get(rt).copied().unwrap_or(0.0));
        *amt > 1e-9
    });
}
```

**新增守卫**
* 建议放 `src/tests/sim/site_supply.rs` 或 `src/tests/sim/haul.rs`：
  构造一处非首都城/货栈，先算出 `site_reserve`，再 `haul_step` 装货；断言对每种资源：
  * 装载量 ≤ `exportable_at`；
  * 装完后货栈剩余 ≥ `site_reserve`（至少在 `exportable_at` 为正的资源上）。
* 必须带防空转判据：这一局真的有过“保留量 > 0 且装货曾发生”。

**验收**
* 旧 haul 守恒用例不应受影响；`a_haul_route_alternates_legs_because_of_the_cargo` 仍应绿。
* 若 site-supply 行为改变导致长局数据变化，按“有意行为改变”更新基线。

---

## P0-2 国内市场：逐城预算被全势力 spent 账本互相吞掉

**位置**
* `src/sim/construction.rs:53-54`（每个势力只建一个 `inv_spent` / `con_spent`）
* `src/sim/construction.rs:77-93`（每个城都传同一对全局 spent）
* `src/sim/construction.rs:340`（建筑预算门用全局 `inv_spent`）
* `src/sim/construction.rs:444`（造舰预算门用全局 `con_spent`）

**现状**
`domestic_market.enabled = true` 时，`market_plan` 给每个城发的是**逐城**
`invest_limit` / `con_limit`；但 `build_city` 里的 `max_affordable_inc` 同时拿全局
`inv_spent` / `con_spent` 去减。前一座城花掉某资源后，后一座城分到的同资源额度会被
全局 spent 扣成 0。

**影响**
* 城市预算变成“先到先得”，依赖 `state.cities` 遍历顺序；
* 国内市场“每城一份提货权”的设计失效；后位城市系统性挨饿；
* `flow.spend` 的全局读数与逐城市场分配口径不一致。

**修法（二选一，推荐 A）**
* A. **双账本**：给 `build_city` 额外传“用于限额判断的逐城 spent”和“用于总账的全局 spent”。
  预算门用逐城账本；`commit_spend` 记账时两边都写。`flow.spend` 继续用汇总账。
* B. 国内市场路径下，每个城传入空 spent 做预算门，然后手工把实际花费累加到全局 `flow.spend`。
  但 A 更容易保持读面对账。

**新增守卫**
* 建议新增 `src/tests/sim/domestic_market.rs`（或扩展现有 inline test）：
  * 开启国内市场的合成世界，至少两座城都有需求；
  * 两城都分到同一种资源的额度；
  * 断言前一座城花完后，后一座城仍能按自己的额度开工；
  * 断言 `flow.spend.investment/construction` 等于逐城实际花费之和；
  * 防空转：真的有 >1 座城、真的发生过实际消费。

**验收**
* `domestic_market.enabled = false` 路径必须逐字节不变。
* 开启路径的旧测试（inline 3 条）继续绿。

---

## P0-3 国际市场：价格发现把“堆在货栈里的产出”算成了消费

**位置**
* `src/sim/market.rs:112-136`（`world_stock` 只加 `Faction::resources`；`consumed` 用的是
  `last_stock + produced − world_stock`）
* `src/sim/production.rs:205-225`（`flow.faction_production` 记的是所有开采量，不管入池还是入货栈）

**现状**
非首都产出会进 `state.depots`，不会进 `Faction::resources`；但 `flow.faction_production`
把它算进 `produced`。于是：

```text
consumed = last_pool_stock + all_production - current_pool_stock
```

会把“还在货栈里等船的产出”当成“已经被消费掉”，系统性抬高 `avg_demand`，进而推高价格。

**影响**
* 在 site-supply 开启、非首都产出占比高、船运不足的世界里，大量资源实际积压却显示“稀缺/高价”；
* 价格信号与真实供需错位，影响建造、组件选装与贸易决策。

**修法（先定口径，再改代码）**
必须二选一并写进注释/读面：
* **口径 A（世界总库存）**：`world_stock` 与 `market.last_stock` 都加上所有 `depots`。
* **口径 B（市场可见库存）**：`RoundSink` 另记“进入首都池的产出”，`market.rs` 的
  `produced` 只用这一份；`world_stock` 继续只算池子。

推荐 A，因为 `market.rs` 现有注释写的是“世界总库存”；但要在读面上明确“可售库存”与
“总库存”的差异。

**新增守卫**
* 建议在 `src/tests/sim/trade.rs` 或新 `src/tests/sim/market.rs`：
  构造“非首都产出持续增加 + 首都池不变”的合成状态，跑市场步；断言 `avg_demand`/`price`
  不因货栈积压被当成消费而爆涨。
* 数据级可在 `play/tests/g2_mid.py` 增加一条：货栈价值上升的回合，对应资源价格不应只由
  `faction_production` 推高（按选定口径重算）。
* 防空转：该局真的出现过 `depots` 增长、且该资源真的在市场价格表里。

**验收**
* 市场默认开启，修完必须重跑长局并重标 digest 基线；这是**有意行为改变**。

---

## P1-1 国内市场：`iterations = 0` 写得到、跑不到

**位置**
* `src/model/game_config.rs:277`（注释：0 = 只投放、不更新价格）
* `src/sim/domestic_market.rs:324`（`iterations.max(1)`）

**问题**
配置写 0 仍会跑一次循环，更新一次价格。

**修法**
允许 0：0 时不进入价格更新循环，但仍用初始价格做一次城市需求/配给结算；把“出需求/配给”
与“价格更新”拆开。

**新增守卫**
* `clear_market` 单测：`iterations = 0` 时价格逐资源保持不变，但 `alloc`/`unspent` 仍正确。
* 配置级测试：从 `GameConfig` 读到 0 时，市场步不改 `DomesticMarketSide.price`。

---

## P1-2 运输：`trip_throughput` 忽略 MOND 导航时间

**位置**
* `src/autocontrol/freight.rs:940-958`（`trip_throughput` 只按距离/速度）
* `src/autocontrol/contract.rs:94-109`（`available_throughput` 直接加总使用）
* `src/autocontrol/contract.rs:132-134`（`decision_value` 用 `available_throughput`）
* `src/model/contract.rs:481-491`（`required_throughput` / `lane_rounds` 用 MOND 掌握度 1.0）
* `src/model/contract.rs:320-360`（`trip_rounds` 已经正确考虑 `mond_arrival_chance`）

**问题**
`trip_rounds` 知道非 master 深空航线要试很多次，但承包市场的**可提供运力/自评**完全
按“直线速度”算。结果：
* 非 master 会高估深空吞吐，接了注定不达标的单；
* `capacity_ledger` / `haul_gap` / 造货船需求信号也会偏；
* 设计里“MOND 是时间优势”没有进入雇佣市场。

**修法**
给吞吐计算加一个带掌握度的版本（或给 `trip_throughput` 加 `mond_control` 参数）：

```text
trip_throughput_with_control =
    cargo_capacity × speed / (2×dist / arrival_chance(depth, control) + 每回合离散修正)
```

优先改造 `available_throughput` / `capacity_ledger` / `decision_value` 的调用点，让它们传
“受雇方自己的 `mond_control`”；`required_throughput` 保持“参考 master = 1.0”的语义。

**新增守卫**
* 同型舰、同深空航线：`mond_control = 1.0` 的吞吐必须显著高于 `mond_control = 0.0`。
* 非 master 对深空单的 `accept_chance` 应低于 master（或 `decision_value` 的自评更保守）。
* 防空转：测试里真的存在 `depth > 0` 的航线。

---

## P1-3 承包市场：撮合时不预留同回合已接订单的运力

**位置**
* `src/autocontrol/contract.rs:175-199`（逐单撮合）
* `src/autocontrol/contract.rs:157-234`（`match_carriers` 只写 `carrier`，不派船）
* `src/autocontrol/contract.rs:365-415`（真正的派工在下一段 `assign_hired_ships`）

**问题**
`match_carriers` 的注释说“前面成交的单会占掉运力”，但同一回合内前面订单只写了
`carrier`，没有 `assignments`，所以后面订单的 `available_throughput` 仍能看到全部空闲船。
同一家受雇方可能接下多张总运力超自身能力的单，之后只能靠 recall/退约修正。

**修法**
在 `match_carriers` 内维护临时 `promised: BTreeMap<FactionId, f64>`（或按 `(faction, from, to)` 维度）：
* 每选中一个承运方，就从它的可用运力里扣掉这张单的 `capacity`；
* 后续订单看到的是扣完后的额度；
* 不要在 `assign_hired_ships` 之前修改 `available_throughput` 的全局语义，尽量把账本做成局部参数。
* 或者更简单：一个受雇方在同一回合最多接一张单，等下一回合派工后再评估后续单。

**新增守卫**
* 合成两张同线/相近线订单，只有一个候选受雇方且运力只够一张；断言它不会同时被两张单选中
  （或总承诺 capacity ≤ 可用吞吐）。
* 防空转：确实存在两个 open contract、确实有过一次选择。

---

## P1-4 国际市场：价格发现用总库存，不用可售挂单

**位置**
* `src/sim/market.rs:128-153`（`coverage = have / demand`，`have` 是世界总库存）
* `src/sim/market.rs:160-184`（挂单只取 `库存 − keep`）

**问题**
总库存很高、但各势力全部囤着不卖时，价格可以很低；买方实际买不到货。价格发现和
“稀缺 → 超高价”的目标之间裂开。

**修法**
把 coverage 的分子改成更接近“市场可用供给”的量。至少可选择：
* `offers` 的挂单总量 + 买方可见量；或
* 用 `stock + on_order` 的思路（`trade-and-sanctions.md` M3 设计原文）。
如果保留“基本面价格”和“可成交价格”两个概念，要在读面拆开，别继续共用一个 `price`。

**新增守卫**
* 合成状态：总库存很高、但全部被 reserve 锁住，`offers` 为空；断言价格不会贴地板
  （或读面明确显示“基本面价 vs 可成交价”）。
* 防空转：该状态真的产生了空挂单。

---

## P1-5 预算 / welfare：资源向量语义与实际扣款不一致

**位置**
* `src/autocontrol/budget.rs:37-63`（维护费 reserve 只对 Construction 生效）
* `src/autocontrol/budget.rs:73-84`（Player 预算绕过 `con_scale`）
* `src/sim/governance.rs:185-243`（`welfare_budget` 最终只折算成 `computed_welfare_value`；`_welfare_budget` 没被使用）

**问题**
* Player 的 `construction_budget` 不受维护费 reserve 约束；长局可能把库存打到维护底线
  以下，和 AI 的保护不对称。
* `welfare_budget` 的逐资源叶实际上只提供“总价值”，实际支付从全部库存按价值比例扣。
  如果控制面/文档把它当“资源向量预算”，这是 WYSIWYG 缺口；如果设计只要“总价值”，
  则应改字段语义/注释，并删掉 `_welfare_budget` 这类误导性中间量。
* Investment 和 Construction 各自都能花 `stock × invest_fraction`，没有联合上限；两者
  同时花会突破 reserve。当前注释说 reserve 只保护 Construction，但 total spending 不设联合闸。

**修法（需要用户口径）**
* 若要“资源向量福利”：实际按叶里的资源结构扣/记，或至少让支付向量与叶成比例。
* 若要“总价值福利”：改名/写清楚只读总价值，保留 `default_entertainment × 城数` 的旧路径。
* 给 Player 预算也加一个显式的“可突破 reserve 吗”的说明或开关；至少读面要能看到
  “批了但会击穿维护底线”。

**新增守卫**
* `welfare_budget` 写入后，断言实际扣款向量与叶的向量成比例（若选向量口径）。
* Player 高额 construction 预算 + 低库存场景：断言能明确观测到“维护费 reserve 被击穿”，
  或有意的保护被触发。

---

## P2 清单（不阻塞，但建议同一轮顺手清）

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| P2-1 | `src/sim/domestic_market.rs:328-369` | `clear_market` 最后一轮 `current/total_demand` 是旧价格算的，但返回新价格 | 价格更新后再算一次需求，或缓存最后一轮 total_demand |
| P2-2 | `src/sim/domestic_market.rs:157-163` | `last_demand` 存的是原始 base demand（未计货币约束/最终价） | 改成 clear_market 返回实际撮合前总需求，或删掉这列 |
| P2-3 | `src/model/market.rs:25-35` + `src/sim/market.rs:175-180` | `Offer.ask` 从没进结算，只用了全局 `price` | 删除字段，或让结算真正读单笔 ask |
| P2-4 | `src/sim/haul.rs:85-118` | 进口腿多个船可能对同一 `site_deficit` 重复装载 | 维护在途/已派工需求账，再取 min |
| P2-5 | `src/sim/haul.rs:220-236` | 承包时 `HaulStep::Delivered.units` 是含抽成的卸出总量，`CargoDelivered.cargo` 是扣抽成后的量 | 文档写清两个口径，或让 step units 也报实得 |
| P2-6 | `src/autocontrol/freight.rs:346-352` | `needed_haulers` 不含航程；远距大积压腿也最多算一艘 | 作为平衡项处理；先加读面/注释，不要硬改配额 |
| P2-7 | `src/sim/market.rs:328-389` | `carrier_income` 记应收，`pay_with_surplus` 不返回实扣量 | 让支付函数返回实付值或做对账守卫 |
| P2-8 | `src/autocontrol/tactics.rs:500-670` | 玩家把正在执行承包单/舱里有货的舰钉成 `War` 时，`tactics` 只读有效 `ship_role`，硬承诺被绕过 | 在 `tactics` 的角色分支前先处理 cargo/contract 强制路径，或在控制写入时守卫 |
| P2-9 | `src/autocontrol/budget.rs:65-88` | AI 预算只遍历当前 stockpile，旧 key 不清理 | write_budget 前 prune，或对缺失 key 写 0 |
| P2-10 | `src/sim/governance.rs:203` | `_welfare_budget` 建出来后未使用 | 随 P1-5 一起处理 |

---

## 验收 / 合并门

按改动范围跑：

```bash
cargo nextest run -P full
cargo nextest run -p planet_x_web
uv run --project play/planet_xq python play/tests/run.py all
uv run --project play/planet_xq python play/tests/_g4_negative.py   # 若动控制面/叶子
```

每个 P0/P1 修复都要带：
1. 一条能精确定位问题的**单测/数据级守卫**；
2. 一条防空转判据（证明这条守卫不是永远为真）；
3. 对行为改变的说明：是默认路径也会变，还是只在 `enabled`/特定 world 下变；
4. 若改 digest 基线，按 `notes.md` 的规则记录新值和取行口径。
