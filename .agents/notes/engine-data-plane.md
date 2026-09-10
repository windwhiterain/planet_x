# 引擎=数据平面，Python kit=策略平面

> 状态 `[~]`（分界线已裁决、读面缺表已开工） ｜ 索引：[notes.md](../notes.md) ｜ 关联：
> `python-control-authoring.md`（客户端篇：kit 的职责与落地）、`control-live-layers.md`（控制面的形状：
> 三态 + 链 + 写值即接管）、`ship-blueprint.md`（**"还不存在的实体"的规则住那里**）、
> `lazy-index-pandas.md`（投影 schema 契约）、`agent-play-friction.md` §45（`--control-plan` 的
> 成本→收益预览：也是"引擎算、Python 只读"的既成例子）

## 0. 一句话

**引擎只负责两件事：产出某种标准的统计学数据格式（tidy 表，按名字 join），以及接受同一形状的
diff。** 通配、编制表、统计筛选、配方、verify——全部是 Python kit 的事，引擎不为它们加任何功能。

这条分界线**和项目里已有的裁决同源**：`play/planet_xq/README.md` 的「Semantic views」一节写的就是
「Python 视图只打包模拟**已经算好并写进 `metrics`/懒表**的量（纯检索，永不与规则漂移）；任何需要
游戏公式的判断都留在 Rust」。本 note 是它的**写侧对偶**，加上一条更硬的分界（见 §1）。

## 1. 分界线（用户裁决）

| 层 | 管什么 | 为什么 |
| --- | --- | --- |
| **引擎（链 + 模板）** | **还不存在的实体**的规则：`叶 → 舰队默认 → 势力 → 全局` 的归属解析、新舰继承什么意图、`写值即接管` 的隐含规则、以及舰船模板（出厂快照，见 `ship-blueprint.md`） | kit 只能对**现在存在**的舰说话；"新下水的舰自动继承意图"只有在链上才成立 |
| **Python kit** | **现存**舰/城/建筑的批量与统计操作：通配、名字表、筛选、聚合、配方、`verify` | 这是 pandas 的活儿；引擎不该长出一堆一次性命令 |

### 1.1 已否决（别再提一遍）

* ❌ **引擎侧通配 `{"ship":"*"}`** —— 不需要。写面已经够表达：`{"ship":"长城","mode":"Auto"}` 合法，
  且 `behavior` 缺省时**只改 mode、不动值**（`src/control.rs:742-767`）。"一键把中国的舰全设成自动"
  = kit 展开成 N 条 `{ship, mode}`，与手写 N 条叶是同一个东西，只是不用手抄。
* ❌ **`clear_ship_orders`（清叶动词）** —— 同上，`mode:"Inherit"` 就是"这片叶不再说话"。
* ❌ **`NOTE_APPLY_EXPANDED`（引擎报"展开了几艘"）** —— `fanned_out` 是 kit 打印给自己的，不是回执。
* ❌ **引擎侧通配铺到建筑权重** —— 顺带记一笔坑：`InvestKey = BuildKey = (CityId, BuildingId)`，
  而 `BuildingId = u32` 是**城内下标**（`src/model/control.rs:47`）。跨回合拼下标必错；kit 必须
  **同回合变换**（读 ckpt → 出 diff → 应用到同一个 ckpt），这样下标才自洽。

### 1.2 按舰级默认落在哪

* 要**新造的**护卫舰自动守家 → 引擎（链上加一层 / 舰船模板）——用户裁决：**这就是舰船模板的功能**，
  所以并进 `ship-blueprint.md`，本 note 不另立概念。
* 只要求**现存**护卫舰 → kit 一句 `ships.query("class=='护卫舰'")` 展开成叶（代价：diff O(N)、新舰不跟随）。

## 2. 引擎欠的表（读面）——这就是"标准的统计学数据格式"

投影已经是项目选定的标准：`idx/*.jsonl` 一行一个 `(round, 实体)`、列名固定、**按名字 join**
（`FactionId`/`CityId`/`ShipId` 全是 `String`）。缺的只是"引擎内部中间量"没有出门：

| 欠的东西 | 数据在哪 | 今天的状态 |
| --- | --- | --- |
| **`flow` 表**（`idx/flow.jsonl` 回合×势力：各资源产出 / `upkeep` / `governance.total` / `coverage`；`idx/city_flow.jsonl` 回合×城×资源产出） | `RoundFlow`（`src/model/metrics.rs:103`，文档原话："各 step 计算并应用、**不落到持久状态、原本不对外暴露的量**"） | `write_round`（`src/projection.rs:160-334`）**一张 flow 表都没发**——`city_production`/`faction_production`/`upkeep`/`governance` 四张按名字键的表 join 不到 |
| **`pre` 段**（回合×势力：本回合依赖 rng 的随机决策） | `RoundState.pre: Derived`（`src/model/state.rs:85`） | 投影只用 post（r0 用 `derived_from_state`，之后用 `advance` 的返回值）；"AI 这回合掷了什么"**任何读面都没有** |
| **控制面 tidy 表 + 每实体 `effective`** | `State.control` / `State.scope` | `--control` 有（读面即写面），但**不是投影里的表**，Python 要 join 得再起一次进程；`effective` 没有 → Python 只能自己重实现 `resolve_chain`（`src/model/state.rs:179-211`），**那是漂移源** |
| **`--derived` 单点导出**（一个 ckpt → `{pre, post}`） | 同上 | 没有；不想为一次 join 跑整个 `--index` |

**硬约束（写成测试）**：`post` 是 `state` 的函数（同一 state 恒定），所以 **`--derived` 与 `--index`
里的同一个数必须逐字节一致**。两个读面各说各话，比缺数据更坏。

投影 schema 是发射端与 Python kit 共享的契约（`projection_schema()`，有测试断言 `LAZY` 与 schema
一一对应：`src/projection.rs:480`）——**每加一张表，两处都要动，别漏 schema**。

## 3. 引擎接受的形状（写面）：不变

`{control:[…], scope:{…}}` 的 presence-aware 多级补丁，读面即写面（`--control` 的模板原样回传安全），
`--control-schema` 给机器可读定义，回执在 stderr（`WARN_APPLY_SKIPPED` / `NOTE_APPLY_TOOKOVER`）。

**核实结论：写面没有"表达不出来"的东西**（含"交回上层" = `mode:"Inherit"`）。这是这套设计少见的
干净处——所以本轮**不动写面**。

## 4. 一个必须写进 kit 文档的取值坑

「交回上层」（`mode:"Inherit"`）**只在舰队默认是 `Player` 时才是干净的**。`ship_behavior`
（`src/model/state.rs:186-193`）：

```rust
if leaf.mode == Inherit {
    if let Some(d) = &c.default_ship_order { if d.mode.is_player() { return Some(d.value) } }
}
leaf.value      // ← 否则落到叶子上那个可能已经过期的记录值
```

* 归属是 `Auto` 时**自愈**：系统下一回合按自己的逻辑重写这片叶，陈旧值只是短暂的。
* 归属是 `Player` 而舰队默认不是 `Player` 时**会长期显示旧值**。
* kit 的处理：凡"释放到上层"，通常**同时**把 `default_ship_order` 写成 `Player`（一片叶），语义闭合。

## 5. 引擎侧落地顺序（用户授权自行决定）

1. `[ ]` **读面补表**：`flow` / `city_flow` / `pre` / 控制面 + `effective` + `--derived` + schema 同步 +
   一致性测试（`--derived` vs `--index` 逐字节）。
2. `[ ]` **风格活层**（`control-live-layers.md` §4.1+4.2+4.4：`default_doctrine`/`default_kiting` +
   `Ship.doctrine/kiting` 降级为记录值 + 读面 `effective`）——与第 1 步同在 `src/control.rs`，
   所以**串行**做。
3. `[ ]` **舰船模板**（`ship-blueprint.md`，含按舰级默认）。
4. 引擎侧**不加**通配/清叶动词（§1.1）。

## 6. 复现 / 验证

```bash
cargo test --workspace
cargo run --bin planet_x -- --seed 7 --round 6 --index out/     # 看 idx/ 是否多了 flow/pre
cargo run --bin planet_x -- --start ../planet_x/play/exp2/ckpt_r12.ron --derived
```
