# 国内市场：资源预算 → 货币预算 → 真实交易

> 状态 `[x]`（第一版 + 福利并轨 + 逐城货币预算控制叶已实现；默认 `config.domestic_market.enabled = false`，旧世界逐字节不变）
> 关联：`spec.md`（控制属性：势力各类资源开发/建造/福利预算；城市福利权重；建造区建造权重；建筑目标面积/开发权重）、
> `trade-and-sanctions.md`（国际市场）、`site-supply.md`（本地库存/运输）

## 0. 为什么要做

`spec` 的控制结构是：

- 势力：各类资源开发预算、建造预算、福利预算（都是资源/时间向量）；
- 建筑：目标面积 + 开发权重；
- 建造区：建造舰船类型 + 建造权重；
- 城市：福利权重。

开发/建造有 **recipe**（每单位活动消耗固定比例的资源），福利没有方向要求。
中央 allocator 若每回合解一个 recipe 约束下的全局凸问题，能得到精确均衡，但：

1. 每回合求解会随势力/叶子数量变慢；
2. 价格是影子价格，不是“真实市场”，读面上不好解释“为什么没买到”；
3. `AGENTS.md` 要求“一切数值用动态平衡/博弈产生”，静态 solver 与该原则相性一般。

因此本 note 给出**国内市场**方案：上层把“当回合投多少国库资源进市场”和“给下层拨多少钱”作为控制，
下层用钱按自己的 recipe 买资源，价格由市场逐回合反馈。

## 1. 数学核心：市场是分布式 allocator

中央 allocator：

```text
max Σ_i w_i log x_i
s.t. Σ_i a_i x_i ≤ b
```

一阶条件：

```text
x_i = w_i / (p · a_i)
Σ_i a_i x_i = b
```

其中 `p` 是影子价格。

国内市场：

- 国库向市场投放资源向量 `g`（对应原预算 `b`）；
- 给下层单位 `i` 拨货币预算 `m_i`（对应原权重 `w_i`）；
- 下层面对价格 `p`，按 recipe `a_i` 买资源：

```text
x_i = min(u_i, m_i / (p · a_i))
d_i = a_i x_i
```

市场出清：

```text
Σ_i d_i(p) = g
```

这与中央 allocator **同形**：`m_i ↔ w_i`，`g ↔ b`，市场价格 `p ↔ 影子价格`。
区别是：如果市场每回合精确出清，结果等价；如果只更新一次价格，结果是动态近似，但
每回合只需 `O(n d)` 的聚合与一次价格更新。

### 1.1 单资源退化

`d = 1`、`a_i = 1` 时：

```text
x_i = m_i / p
Σ_i x_i = g  ⇒  p = Σ_i m_i / g
x_i = g · m_i / Σ_j m_j
```

即“拨钱比例 = 活动份额”，与旧权重分配完全一致。

### 1.2 多资源：价格内生

`p` 不再是一个可以外生点积乘出来的总预算。对开发/建造：

```text
p · g = Σ_i m_i - Σ_i μ_i u_i
```

`p·g` 是权重和容量租的镜像，不是可自由设定的总量。真正的物理总量是 `g`。
所以：

- 要标量总量旋钮：把 `g` 定义成 `B · σ`（`B` 标量、`σ` 资源结构 simplex）；
- 不要用 `p·g` 当外生总预算。

福利是例外：它无 recipe，`p` 可以取外生市场价，`p·g_welfare` 才是真总福利预算。

## 2. 国内市场规则（第一版）

### 2.1 上层：国库

每回合：

1. **资源投放**：把开发/建造预算向量 `b_dev`、`b_con` 投放到国内市场（福利暂留旧路径，后续可并入）。
2. **拨钱替代权重**：
   - 第一版：总货币 `M = p · b`（按当前国内价）；城市 `c` 拿到的钱
     `m_c = M · w_c / Σ w_c`；
   - 城市权重来自：开发 = 该城所有在造建筑的 `invest_weights` 之和；
     建造 = 该城所有建造区的 `build_weights` 之和。
   - 第二版再把 `m_c` 做成可直接写的控制叶（`development_money` / `construction_money`）。

### 2.2 下层：城市作为买方

每个城市只算自己的 aggregate demand：

- 开发：遍历在造建筑，计算 `desired = min(area - deployed, speed)` 与 `recipe = per_area_cost`；
  `d_city += recipe * desired`。
- 建造：遍历建造区，计算 `desired = area * productivity * labor` 与
  `recipe = ship_build_cost / build_points`；`d_city += recipe * desired`。

给定价格 `p` 和货币 `m_c`：

```text
cost = p · d_city
if cost > m_c: d_city *= m_c / cost
```

这一步是需求，不是最终分配；市场再配给。

### 2.3 市场：价格反馈 + 库存

对每个势力、每个类别独立市场：

```text
D = Σ_c d_c
p ← clamp(p · ((D + eps) / (g + eps))^damping, p_min, p_max)
```

重复 `iterations` 次（默认 4，可 0 = 不做价格更新）。

第一版的 `g` 是**预算速率**，不是实物库存；未用额度只记为 `unspent`，
**不跨回合累加**（否则会把月预算变成无限累积的提货权）。真正的本地库存
仍由 `stock_at` 守门。

结算：

- `D ≤ g`：全部成交，剩余进 `unspent`（观测用）；
- `D > g`：按城市需求比例配给。
- 下层付出货币，国库收钱，货币循环。

**货币投放**：第一版用 `config.domestic_market.money_multiplier`：

```text
M = money_multiplier × 预算按基价的价值
```

`1.0` = 货币恰好背书当期预算；>1 = 货币宽松、名义价格上移；
第二版把 `money_multiplier` 换成逐城货币预算叶。

### 2.4 物理约束

国内市场只分配**预算/提货权**，不凭空造货。
`build_city` 仍然走 `site_affordable` / `stock_at`：本地没有的货必须靠运输。
这样：

- 控制面：资源预算 + 货币预算；
- 市场面：价格与配给；
- 物理面：本地库存 + 运输。

三层各司其职，不互相替代。

## 3. 落地到代码（第一版）

### 3.1 新增/修改

- `config.domestic_market`：
  - `enabled`（默认 `false`，避免改变现有世界 digest）；
  - `iterations`、`damping`、`price_floor`、`price_ceiling`。
- `MarketState.domestic: BTreeMap<FactionId, DomesticMarket>`：
  - `price`、`inventory`、`last_demand`。
- `src/sim/domestic_market.rs`：
  - `plan_faction(state, config, fid, &b_dev, &b_con) -> DomesticPlan`
  - `DomesticPlan { development: BTreeMap<CityId, ResourceMap>, construction: BTreeMap<CityId, ResourceMap>, prices... }`
- `step_construction`：
  - 若 `config.domestic_market.enabled`，把全局 `investment`/`construction` 交给
    国内市场，得到每城预算，再调用 `build_city`；
  - 否则走旧路径（逐字节不变）。
- `SCHEMA_VERSION` 23 → 24，`migrate` 把 23 并进“只推号”一档。

### 3.2 本轮已补：福利并轨 + 逐城货币控制叶

- **福利并轨**：
  - 势力级新增 `welfare_budget: {资源 → Control<f64>}`（`spec.md` 的「各类资源福利预算」）。
  - 城市旧叶 `loyalty_budget` **语义改成福利权重**（不再是每城市场价值预算）：
    `welfare_budget` 先按市场价值求总池，再按各城权重分给城市，进入忠诚目标里的娱乐项。
  - 默认路径（没有 Player 福利叶）严格等于旧行为：总福利价值 = `default_entertainment × 城数`，
    不让库存/浮点重算改变确定性世界。
- **逐城货币预算控制叶**：
  - 新增 `development_money: {城市 → Control<f64>}` 与
    `construction_money: {城市 → Control<f64>}`，单位是市场价值/回合。
  - 国内市场价格反馈里，城市货币预算优先读这两片叶；缺叶/Auto 时按原有建筑/建造区权重自动折算。
  - `Player` 写入后，该城的货币预算不再被自动折算覆盖。
- 控制面从 **14 叶** 扩到 **17 叶**（+ `welfare_budget` / `development_money` / `construction_money`），
  web `views.json`、`--control-schema`、投影 `derived.control` 三端同步。
- 城际双向交易、福利专用市场仍留作后续。

## 4. 验证

- `cargo check`。
- 单元测试 `src/tests/sim/domestic_market.rs`：
  1. 单资源、同 recipe：市场分配 = `g · m_i / Σm`；
  2. 双资源：铁稀缺、硅过剩时，铁密集型需求被配给压低，硅密集型活动更高；
  3. 价格反馈：需求 > 供给时价格上升；供给过剩时价格下降；
  4. `enabled = false` 时 `step_construction` 与旧路径 digest 一致（用现有 spending 测试）。
- 长局 A/B：`config.domestic_market.enabled = true`，seed 1/7/42 × 400 回合，比较产出/建成面积/舰数/库存。

### 4.1 本轮全门结果

- `cargo nextest run -P full`：206 passed / 31 skipped。
- `cargo nextest run -p planet_x_web`：24 passed。
- Python 四组：`g1 33/33`、`g2 60/60`、`g3 26/26`、`g4 15/15`。
- 默认 `enabled = false` 下，世界 digest / 长局数据与旧行为一致。

## 5. 开放问题

1. 城市 vs 建筑作为下层粒度：第一版选城市（状态少、和本地库存契合）。
2. 货币是否可跨回合保存：第一版用“当回合预算”，未花完作废，避免囤积震荡。
3. 福利是否并入同一市场：建议单独市场，避免福利烧掉战略资源。
4. 价格是否跨存档持久：建议持久，便于连续价格发现。
