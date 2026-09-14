# 市场系统：当前设计

> 这份文档描述**现在**的模型（commit `38b5686` 之后的语义）。
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

搜索空间是 `e^{±44}` 的对数网格（129 格 + 黄金分割细化）。

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

- `basket_value(policy)` → `unit_sell` / `unit_buy`（都取 `q = 1` 件）→ 收入、成本、篮子需求；
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
| 部门决策 | 自己的两条曲线 | argmax / argmin |
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
2. **部门估值的量取 "1 件"**：`best_*_revenue(stock, 1.0)`。如果要让兑现率曲面的非线性
   进来（比如"这次计划要卖一篮"），把 `1.0` 换成 policy 的对应数量。
3. **报价搜索带宽还是 `e^{±44}`**：单个交易者仍可以在十几个数量级里挑报价，本地价的
   离散度（实测 1e-8…1e6）主要来自这里。要收成"上一个成交价附近的一个区间"的话在这里动。
4. **12 条测试仍红**：都是记录**旧口径**的（`execution`、上界盒子、价格水平跟着 wedge 走）。
   重基线时价格读数应改成**本地价**，不是银河指数。

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

实测（5 个新抽种子 × 20000 轮）：`log10` 三商品 index 的 max/min = **2.9–8.4 / −8.2 – −4.5**
（计价物 = `BASE_PRICE` = 1）——有界的经济波动。
