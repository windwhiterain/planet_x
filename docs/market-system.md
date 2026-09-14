# 市场系统：当前设计

> 这份文档描述**现在**的模型（commit `38b5686` 之后的语义；产线估值的量见
> `docs/local-price.md` §21）。
> `docs/local-price.md` 是实验日志（§1–§20 记录了怎么一步步走到这里的），
> 里面的数字多数是旧口径的，**不要引用**；当前设计的"为什么"见它的 §20。
> 本文件只讲"现在是什么"，不重复那段历史。

---

## 0. 一句话

**价格不是一个存量数。** 每个交易者/部门有**学习曲线**（模型），凡是要用到价格的地方，
就地在曲线上做一次 **argmin / argmax**；撮合出来的报价只被**聚合**成读数（本地价、银河价），
读数**不回喂**给任何人。

---

## 1. 三层，别混

| 层 | 是什么 | 谁产生 | 谁读 |
|---|---|---|---|
| **模型** | 学习曲线（函数） | `observe` 每轮学 | 决策（做 argmax/argmin） |
| **决策** | 一个绝对报价（货币/件） | `sale_price` / `purchase_price` / 部门估值 | 撮合 |
| **读数** | 本地价、银河价（数） | `update_books` / `aggregate_index` | 只有显示 |

一条曲线不是"一个价格"，一个报价也不是"曲线上的一个存值"——它是当场算出来的**最优解**。

---

## 2. 交易者（`warehouse`）

### 2.1 状态（`Stock`）

```
volume, previous_volume          库存
taken                            本轮被部门取走的量（目标水位锚在它上面）
target_volume, target_floor      目标水位 = max(3 × taken, 初始目标)
declared_gap                     申报前的原始缺口（仪表）
marketing_volume                 申报量（正=卖，负=买）
marketing_price                  **卖/买要报的绝对价**（不会被子系统当"价格"读）
buy_response, sell_response      兑现率曲面（Response）
buy_price_curve, sell_price_curve 价格曲线（PowerLaw）
```

两个学习器都是**曲线**：

```
sell_price_curve / buy_price_curve :  报价 → 成交价        （绝对值，RLS）
sell_response  / buy_response      :  (申报量, 力度) → 成交量 （饱和 Hill 曲面）
力度：卖方 = 1/报价，买方 = 报价
```

### 2.2 报价怎么来（argmax / argmin）

```
卖方：price = argmax_p  sell_response.get(available, 1/p) × sell_price_curve.get(p)
买方：price = argmin_p  volume(p) × buy_price_curve.get(p)
              s.t.       buy_response.get(volume, p) ≥ need
                         volume × 价格 ≤ cash          ← 绝对货币，价格水平在这里被钉住
```

搜索空间是 **上一个成交价 × `e^{±quote_band}`** 的对数网格（129 格 + 黄金分割细化）。
`quote_band` 默认 **1.0**（≈ 0.37×–2.7×），见 §7.3；`LOG_LIMIT`（44）= 全 f32 范围，
是改这一条之前的历史行为。

### 2.3 申报量

```
target_volume = max(3 × taken, target_floor)
gap           = volume − target_volume + min(volume − previous_volume, 0)
申报量         = |gap| × (u/(1−u))^fluctuation × sign(gap)      u ~ U(1e-6, 1-1e-6)
```

`fluctuation` 默认 **0.05**（`Spec::symmetric`），所以随机化是默认开的。

---

## 3. 部门（`department`）

部门**不读任何挂牌价**：它在 `plan` 开头用自己的两条曲线现算估值。

| 助手（`warehouse::step`） | 语义 |
|---|---|
| `best_sale_revenue(stock, q)` | 报价维度 **argmax**：卖 `q` 件的最好收入 |
| `best_purchase_cost(stock, q)` | 报价维度 **argmin**：买 `q` 件的最少花费（买不到 → ∞） |
| `affordable_quantity(stock, cash)` | 报价维度 **argmax**：这笔现金最多换几件 |

用它们算：

- `basket_value(policy)` → `unit_sell` / `unit_buy` → 收入、成本、篮子需求。**读的量是
  `traded_prices` 给的**：`q_sell = max(上一轮申报卖出量, 1)`、`q_buy = max(上一轮申报买入量, 1)`，
  即**本部门上一轮实际经手的量**，而不是固定的 `q = 1`（见 §7 第 5 条；为什么不按整篮数量估，见 §7.2）；
- `production_score` = `(收入 − 成本) × 产能/原料天花板`（亏损为 0）；
- `consumption_potential` = `motive / 成本`（产能不限时用 `motive / 产能占用`）；
- `material_ceiling` 的买入力 = `affordable_quantity(stock, currency)`。

政策之间按 `exp(score / best / 0.25)` 分配，生产族/消费族各自归一。

---

## 4. 一轮的流程

```
departments.plan        部门用曲线估值 → 选政策 → 决定 intake/output
  warehouses.step
    declared_volumes    申报量（含涨落）
    sale_price / purchase_price   学习曲线 argmax/argmin → marketing_price
    写进 market.traders[].price / .volume
  market.step           撮合（软成交：无论价差多大都成交，价差决定成交多少）
    写进 deal_price / deal_volume
  departments.settle    成交收入进仓库
  departments.reclaim   每轮清零再拨款（reclaim 口径）
  warehouses 收尾
    observe             用（本轮报价, 成交价）更新价格曲线；用（申报量, 力度, 成交量）更新响应曲面
    update_books        把报价聚成**地方账本**（bid = 最高买价、ask = 最低卖价，带 0.8 记忆）
Lab::step
  aggregate_index       银河价 = 各地方本地价的成交量加权几何平均
  local_readout         逐政权本地价 = 该政权地方账本中间价
  anchor_prices         可选：把银河读数按"三商品几何平均 = 1"归一化（纯显示）
  update_diagnostics    诊断读数：逐政权 vwap、内部/跨境成交量、现金（不写回任何决策）
```

---

## 5. 谁能读什么（审计）

| 谁 | 读什么 | 性质 |
|---|---|---|
| 交易者决策 | 自己的两条曲线 | argmax / argmin |
| 部门决策 | 自己的两条曲线，**读在上一轮自己经手的量上**（`traded_prices`） | argmax / argmin |
| 学习器 `observe` | 本轮自己的报价 + 成交价 | 观测（学习用） |
| 撮合 `market.step` | 交易者报价、申报量 | 匹配 |
| `books`（账本） | 交易者报价 | **读数** |
| `aggregate_index` / `local_readout` | `books` | **读数** |
| `polity.level` | 本地价读数 | **显示** |
| `update_diagnostics` | 成交明细 | **读数**（`vwap`/`internal`/`external`/`cash`） |

**没有一条路径**把账本 / 本地价 / 银河价读回去当输入。

---

## 6. 三个读数的定义

```
地方账本中间价   Book::mid = sqrt(bid × ask)                 （bid/ask 是该地方的最高买价/最低卖价）
逐政权本地价     polity.level[k] = 该政权地方账本中间价
银河价           market.merchandises[k].price =
                     exp( Σ_locality 成交量_locality × ln(本地价_locality) / Σ 成交量 )
```

`--anchor` 会把银河读数整体乘一个常数，使三商品的几何平均 = 1 —— 这只是显示约定。

---

## 7. 残留 / 未决

1. ~~**`wedge` / `LevelRule` / `--rule` / `--forgetting` / `--gain` / `--recenter*` 是死状态**~~
   —— **已删**。`update_levels` 改名 `update_diagnostics`，只写 `vwap` / `internal` /
   `external` / `cash` 四个读数；`Polity` 不再有 `wedge`，CLI 也没有那五个旋钮
   （JSON 的逐政权字段从 `wedge` 换成 `level`）。
2. ~~**部门估值的量取 "1 件"**~~ —— **判过：不做**（本轮）。试过把 `best_*_revenue(stock, 1.0)`
   换成 policy 的对应数量（整篮口径）：兑现率曲面在**大批量上饱和**，于是它**系统性惩罚
   高吞吐工艺**——
   - 探针（默认先验、天花板 `share_max × depth_max = 4×4 = 16`、带宽 1.0）：慢工艺
     （进 0.8 / 出 4）边际利润 +3.68 → **+2.75**；快工艺（进 9.6 / 出 12）
     **+4.48 → −13.98**（收入 13.22 → 5.84、成本 8.74 → 19.82）。`production_score`
     见 `margin ≤ 0` 就返回 0，于是 `a_process_choice_follows_whichever_resource_is_tight`
     里"产能紧张就选快工艺"直接变成份额 **0**。
   - **与带宽无关**：`quote_band = 44` 下快工艺仍是 **−13.25**（不是 §7.3 收窄造成的）。
   - 完整模拟（4 种子 × 5000 轮）不会崩（指数甚至更紧），但部分种子成交从 ~90 掉到
     ~48–51 件/轮：它是在**改经济**，不是在修 bug。
   - **不做的理由**：产出侧那一半是概念错配——部门**不直接把产出成批卖掉**，产出进仓库、
     由仓库按 `target = 3 × taken` 自适应申报；"整篮卖出去"把下游并不存在的市场冲击
     提前算进了工艺选择。买入侧那一半（`best_purchase_cost(stock, consumption)`）与部门
     实际采购一致、是成立的，但不足以单独改工艺选择的相对优劣。
   - 另有 `margin × ceiling` 对凹估值的线性外推不自洽。原型补丁已回退。
3. ~~**报价搜索带宽还是 `e^{±44}`**~~ —— **已落地（本轮）**。每个 `Stock` 记住上一轮
   自己的**成交价** `last_deal`（带宽中心，`observe` 更新），报价搜索落在
   `last_deal × e^{±quote_band}` 里；`quote_band` 默认 **1.0**（`Warehouses::DEFAULT_QUOTE_BAND`，
   CLI `--quote-band`，取 `LOG_LIMIT`=44 即逐位回到历史行为）。
   - **量出来的效果**（seed 11、5000 轮、`modern/capacity 12`，本地价的逐轮 log 跨度均值）：

     | `quote_band` | 本地价 log 跨度 | 成交/轮 | log10 index max/min |
     |---|---|---|---|
     | 44（旧） | **48.2** | 88.8 | 8.4 / −5.0 |
     | 3 | 6.7 | 99.5 | 5.2 / −3.9 |
     | 1.5 | 4.0 | 101.8 | 2.7 / −1.9 |
     | **1.0（新默认）** | **1.45** | 90.4 | 2.1 / −1.1 |
     | 0.5 | 0.79 | **0.66（冻死）** | 0.3 / −0.5 |

   - **长时程**（5 个种子 × 20000 轮，默认 1.0）：`log10` index = 0.39–2.54 / −1.33…−0.43，
     成交 74–96 件/轮；同一批种子在 3.0 下有一个漂到 `10^10.7`，0.5 会把成交压到 0。
   - **附带修好一条**：`a_targeted_sanction_opens_a_local_gap` 里"壁垒越重、价差越深"
     的单调性在旧带宽下是被挂价噪声打断的，收窄后重新成立（w = 1.0/0.5/0.0 →
     0.9957 / 1.0577 / 1.0926）。
   - 带宽只压**步长**，不是水平锚：它把本地价离散度收下来，长时程漂移仍需 §20.7 说的
     那条水平锚（仍未做）。
4. ~~**12 条测试仍红**~~ —— **已重基线**（本轮，**129 过 / 0 失败 / 1 ignore**）：
   - 价格水平类改成读**逐政权本地价**：`premium` 读稀缺政权（seat 0）的 `polity.level`
     （指数是各地方本地价的加权几何平均，会把"0 号政权缺、其余富余"抹平，实测 ≈ 0.99）；
     `the_anchor_leaves_the_relative_premium_of_a_scarce_good` 同时验证**锚只动指数、
     不动本地价**（开/关锚下 `polity.level` 逐位相同）；
   - 部门结算类改成对 `settlement` 的 KKT **独立二分求根**：`basket_root`（单政策多商品）
     与 `shared_good_root`（两条政策抢同一商品），不再撒旧口径的加权执行率；
   - `a_cheaper_policy_takes_the_larger_share` 改成用新加的测试钩子
     `Stock::set_buy_price_curve` 注入常数买入曲线——部门**不读挂牌价**（§20.13），
     "便宜"只能由它自己的曲线给出；
   - `a_targeted_sanction_opens_a_local_gap` 改成读被制裁政权本地价与邻居之比，并如实
     记下**符号**（被切断的是消费部门 ⇒ 它要买的商品本地价被顶高）；§7.3 收窄带宽之后
     "壁垒越重、价差越深"的单调性重新成立，该测试随之改成断言单调。
5. ~~**部门估值取 `q = 1` 件**~~ —— **已修（本轮）**。`q = 1` 读出来的是**单件最优**：
   收入端是"只卖一件能榨到的最高价"（兑换率曲面在小量上饱和 ⇒ 比流量成交价高约 2 倍），
   成本端是"只买一件能压到的最低价"（部门从不买那样货时那条曲线整场没更新过 ⇒ 先验，
   比本地价低约一半）。两个偏差把"工业品价 ÷ 粮食价"这个换挡信号压掉约 2.5 倍：
   阶梯的理论换挡点 1.104，实际跑到 **2.7**，而且在 2.7 时快工艺还在满负荷开工。
   现在 `traded_prices` 把曲线读在**本部门上一轮经手的量**上，换挡点回到 1.7 附近、
   输入太贵时两条工艺一起关停。见 `docs/local-price.md` §21。
   - **成本端只修了一半**：`best_purchase_cost(stock, q)/q` 在可行区间里与 `q` 无关
     （曲面那一支返回的收盘量正好是 `q`），"最便宜的可行报价"这个偏差要靠部门**真的去买**
     来纠正；"部门不买 ⇒ 曲线是冷的 ⇒ 读数偏乐观"这条**学习**上的鸡生蛋还开着（§21.8）。
   - **另修一条硬 bug**：`basket_value` 里 `0.0 × ∞ = NaN`（系数为 0 的商品也读了一次估值）。
     `NaN` 骗得过 `margin <= 0`，于是部门只要对**任何**一样商品估出"买不到"，**所有**政策
     （包括根本不消耗那样货的免费一产）都归零、整个部门停摆。回归测试
     `department::step::tests::a_zero_coefficient_never_reads_an_infinite_price`。

6. **读数是否还会走到极端？** v2 实测：同一配置下 `seed 11` 只用 2000 轮就把两个商品读数
   打到 `0`、第三个打到 `2.4e8`，所以"有界"只是多数轨迹的性质。收窄报价带宽（§7.3，
   默认 1.0）之后**这个案例不再复现**（合并树复测 `seed 11` × 2000 / 5000 / 20000 轮：
   `log10` index = −0.73…1.35，没有一条非正）。但那条**代码路径**还在——`Lab::step` 只在
   `price > 0` 时覆盖读数，`aggregate_index` 返回 0 时保留旧值，`anchor_prices` 对 0 也
   无能为力；要断言"有界"得先把它处理干净（下一步）。


---

## 8. 复现

```bash
# 一条轨迹（不给 --seed 就每次抽新种子，并打到 stderr）
cargo run --release -p planet_x --bin local_price -- \
  --scenario modern --capacity 12 --specialty 2 -n 2000 --every 200

# 多种子 + 长时程看有界性（看三个 index 的 max/min）
cargo run --release -p planet_x --bin local_price -- \
  --scenario modern --capacity 12 --specialty 2 -n 20000 --every 400 --json \
  | grep -o '"index":[0-9.eE+-]*'
```


实测（默认 `quote_band = 1.0`，**5 个种子 × 20000 轮**）：`log10` 三商品 index 的
max/min = **0.39–2.54 / −1.33 – −0.43**，成交 74–96 件/轮（计价物 = `BASE_PRICE` = 1）。

⚠️ **`modern` / `sectors` 一跑就是两档**（`motive_ladder = false` 与 `true`），stdout 会
把两档的快照**接在一起**。按上面的 `grep` 汇总等于把两条轨迹混起来统计——要分开就得按
行数对半切（`docs/local-price.md` §21.7 的表就是这么测的）。

选工艺那条链的复现：

```bash
# 阶梯：慢/快两条工艺的份额随 工/粮 换挡（理论换挡点 1.104）
cargo run --release -p planet_x --bin local_price --   --scenario ladder --seed 11 -n 120 --every 120
```

旧的 `quote_band = 44` 下同一批种子会到 `10^8`–`10^14` / 本地价 log 跨度 ~48——
§7.3 收窄之后从"十几对十几个数量级"变成 ±2.5 个数量级。

⚠️ 但"有界"仍**不是保证**：§7.5 那条 `index = 0` 的代码路径还在，
只是默认参数下没被触发。

