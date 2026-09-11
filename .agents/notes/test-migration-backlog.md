# 测试全搬的施工图：**判据缺什么数据，就往序列化里装什么**

> 状态：**第 1–6 批已落地**（2026-10；分支 `feature/test-migrate-rest`，worktree `C:/resource/planet_x-decoupled`，
> 第 1–6 批**都已合进 `main`**（第 6 批 `fadca4e`，快进））。
> §4 的「故意不搬」清单已写死（第 7 批）——**不用再逐条论证**。
> 计数：**Python 171**（g1 43 / g2 79 / g3 26 / g4 23）；**Rust 205**（+31 探针 ignored）。
> 起点口径（本主题开工时）：Python 75 / Rust 222 单测 + 8 集成。（2026-10 用户裁决：*「总之目标是全搬，有需要的数据没序列化就把他装进序列化里」*）
> ｜ 索引：[notes.md](../notes.md) ｜ 上层：[test-decoupled-suite.md](test-decoupled-suite.md)

## §0 现在的账

| | 数据级（Python） | Rust 侧 |
| --- | --- | --- |
| 起始（本主题开工时） | 75 | 222 单测 + 8 集成 |
| 施工图写下时 | 120（g1 33 / g2 46 / g3 26 / g4 14） | 206（+31 探针） |
| 第 1–5 批后 | 146（g1 43 / g2 62 / g3 26 / g4 15） | 200（+31 探针 `#[ignore]`） |
| **现在（第 6 批 + 同步 `main` 后）** | **171**（g1 43 / g2 79 / g3 26 / g4 23） | **205**（+31 探针 `#[ignore]`） |

> 组数里 g4 18→23、Rust 196→205 里的大部分是**同步 `main` 带进来的**（另一批在扩 g4 纪律，并给
> `sim::site_supply`、`domestic_market`、`market`、`contract` 各加了用例），不是第 6 批搬的；
> 第 6 批自己的增量是 **g2 62→79**（Rust 这边只删了 3 条、留了 1 条——见 §5.6）。

搬走的族（截至第 6 批）：投影审计 6、governance/spending 9、combat 8、blueprints 9、合成场景 6、trade 3、cargo 1、off_capital 1、派单 1、纯函数 5。

### §0.1 第 1–6 批落地记录（每批 §6.2 三条全绿；第 1–5 批已合 `main`）

| 批 | 装了什么 | 搬走 | Python | 合 `main` |
| --- | --- | --- | --- | --- |
| 1 `trade` | 零新增（`market_trades` + 主流 `view.market_settled` 已够） | trade 3 条 → g2 `trade_checks`（6 条） | g2 46→52 | `9f7f197` |
| 2 舰的货舱 | `ships` 加 **`载货`** + **`cargo_capacity`**（派生列） | `haul::cargo_capacity…` 数据级一半 → g2 `cargo_checks`（3 条） | 52→55 | `a5b68ec` |
| 3 `depots` | 新派生表 `depots` | `haul::off_capital_production…` 读面一半 → g2 `depot_checks`（5 条） | 55→60 | `7846c04` |
| 4 派单抽签 | 零新增（用 `round_inputs.rolls`，**否决** `haul_lanes`——§5.4） | `freight::route_lottery…` → g2 `dispatch_checks`（2 条） | 60→62 | `83521f6` |
| 5 `--call <fn>` | 新 CLI 读面（`src/main.rs`） | 纯函数 5 条 → g1 `call_functions`（10 条） | g1 33→43 | `9e1a566` |
| 6 合成场景 | **零新增序列化**；`_harness` 加**拨控制叶**的路（`scenario_apply`，走引擎自己的 `--apply`——§5.6） | 类 C 3 条**整搬** + `build_lines…` 的**读面那一半** → g2 `blueprint_scenario_checks`（17 条） | g2 62→79 | `fadca4e`（快进） |

> §1–§3 的小节标题已标注落地状态；正文里的「装哪儿（建议）」表**保留原计划**，落地口径以 §0.1 / §5 / §5.4 / §5.6 为准。

**剩下的 200 条按「缺什么」分四类**——前三类的解法都是**把数据装进序列化**（照 B1–B5 的规矩：
过程量与观测同处一行、纯追加、**行为中性 ⇒ digest 逐字不变**、「这一步没跑」给**中性缺省**）。

## §1 类 A：数据没序列化（装进去就能搬）——**第 1–3 批已落地**

| 缺的量 | 谁要它 | 装哪儿（建议） |
| --- | --- | --- |
| **舰的货舱**：`cargo`（按资源）+ `cargo_capacity` = 舰级舱容 × `hull / hull_max` | `haul::cargo_capacity_is_class_capacity_times_hull_fraction`、freight 的多条腿用例 | `ships` 表加两列（**状态里已经有 `Ship.cargo`**，投影没发） |
| **产地货栈的逐资源存量**（现在只有 `cities[].depot_value` 一个合计） | `haul::off_capital_production_lands_in_the_depot_not_the_pool`（「金星货栈里有碳、池子里的碳没动」） | 新派生表 `depots`（`round, faction_id, body_id, resource, amount`）——与 `market_trades` 同形 |
| **集货腿（lane）**：出口腿/进口腿、`from/to`、资源、量、派了哪条舰、保留量、需求 | `autocontrol::freight` 全族（~17 条）里「按积压占比抽签派单」那半边 | 新派生表 `haul_lanes`（`round, faction_id, kind, from_body, to_body, resource, amount, ship_id, reserve`）；`haul_steps`（已有）记的是**在途**，缺的是**派单决策** |
| **成交的价格分解逐项**（若 `market_trades` 没发全 `dist_au`/`depth`/`freight_rate`/`mond_extra`） | `trade::trade_price_terms_are_self_consistent` | `market_trades` 补列（先查 `schema.json`：可能已经够了） |

## §2 类 B：**纯函数**（要一条「调用」路）——**第 5 批已落地**（`--call`，见 §0.1 / §5）

`haul_split_is_max_min_fair`、`hit_factor`、`home_defense_mult`、`ship_panel` 的公式、
`spawn_clears_stale_plan` 之类：它们的判据是「给这些数 ⇒ 得那个数」，**没有世界可看**。

两条路（建议 A，因为它同时是**给 agent 的读面**）：

* **A. `planet_x --call <fn> --args <json>`**（一次调用、一行 JSON 回执）：把引擎里**已经写好、
  已经在用**的纯函数直接暴露出来，Python 拿真配置 + 真参数调它。好处：不是「抄公式」而是
  **调同一份实现**；坏处：多一个读面要维护（文档 + 参数校验 + 报错回执）。
* B. 留在 Rust（现状）。⇒ 与「全搬」的目标冲突，除非用户改口。

## §3 类 C：**合成场景**——**第 6 批已落地**（地基缺口与清单见 §5.6）

`scenario()` API（`_harness.gen/edit/state_dump/scenario`）管「改一个**状态字段** ⇒ 看后果」；
第 6 批补的是另一半：**拨控制叶**（`state.control` 是字典型，Python 里没有它的形状镜像）——
新入口 `_harness.scenario_apply(name, seed, rounds, diffs)` 走引擎自己的 `--apply`。
两条路都只吃读面，判据全部落在 g2 的 `blueprint_scenario_checks`（15 条）。

## §4 类 D：**故意不搬**的清单（第 7 批写死）

判据是「**引擎自己的接口**」而不是「世界长什么样」的，留在 Rust 是对的——搬走只会让两边各说各话。
下一轮**不要再逐条重新论证**：

| 留在 Rust 的（代表文件 / 用例） | 为什么**故意**不搬 |
| --- | --- |
| `--apply` 的补丁面报错回执：`src/tests/control/*.rs`（`ERR_CONTROL_*`、`WARN_APPLY_*`、`NOTE_APPLY_REMOVED` 等） | 判的是**写面契约**（补丁能不能收、收下后回执说什么），不是世界；读面只做「模板回传是不动点」那一半（g1 `control_fixed_point`） |
| schema 迁移 / 版本拒绝：`src/tests/config.rs`、`src/tests/model/state.rs` 里 `migrate` 的分支 | 要构造**非法 / 旧版**输入，读面看不到 |
| `model::neutral` 中性值表的**正向**守卫：`src/tests/model/neutral.rs` | 要走 schemars 的**类型 schema 遍历**；Python 侧只做反向一半（`neutral.fields` 的路径都得活着，g1 `neutral_paths`） |
| 序列化往返：`src/tests/json.rs`、`model/state.rs` 的 round-trip | 判的是**引擎自己的存档接口** |
| `debug_assert` / 引擎内部不变量 | 例如「漏了 `kill_ship`」；能写成数据级判据的都已写（如建筑 id 单调 → g2） |
| sink / 中间量级契约（直接读 `RoundSink`、`view_from_state`） | 是**内部**契约，读面看不到；能走 `--call` 的已走（§2） |
| 手工造世界的对轰 / 摆舰队：`src/tests/sim/shots.rs`、`src/tests/autocontrol/*` 里的 `duel(…)` | 要绕开正常开局直接摆边界局面；能走 `--save` + 改档的已搬（§3），摆不出来的留 Rust |
| 探针：`tests/*_probe.rs` 的 31 条 `#[ignore]` | 只打印不断言；真要调平衡时更适合搬 Python（数据面完全够，见 `test-decoupled-suite.md` §10.5） |

## §5 执行顺序（按「装一次数据 ⇒ 能搬一族」排）

1. **[x] `trade`**——3 条，零新增（`market_trades` 已经够）。
2. **[x] ships 加 `载货` / `cargo_capacity`** ⇒ 搬 `haul::cargo_capacity…` + 给 freight 铺路。
3. **[x] 新表 `depots`** ⇒ 搬 `haul::off_capital_production…`。
4. **[x] 派单半边** ⇒ 搬 `autocontrol::freight` 的「按积压占比抽签」。**结论：不建 `haul_lanes`**（见 §5.4）。
5. **[x] `--call <fn>`** ⇒ 搬类 B（已搬 5 条；剩下的见 §4）。
6. **[x] 合成场景收尾** ⇒ 搬类 C 剩下的（地基与逐条清单见 §5.6；只剩「等字段命名批 B/C」那两条）。
7. **[x] 把 §4 的清单写死在本文件里**（§4 就是）。

每批的规矩（与 `test-decoupled-suite.md` 一致）：**装数据的那次提交必须证明行为中性**
（`--seed 42 --round 240 --digest 20` 的 sha256 不变），搬走的判据要带**防空转判据**，
并且**搬一条删一条**（Rust 原件不留在那儿等人重新困惑）。

### §5.4 为什么**没有**建 `haul_lanes`（第 4 批的裁决，别翻案）

`route_for` 在 `step_military` 里**逐舰**调用，`haul_step` 就在同一循环里 ⇒ **前面的舰已经把货搬走、
池子改了**，每艘舰看到的腿都不同。一张「每回合每势力一条」的 `lanes()` 表既不忠实
（**§12 同回合相位错位**）、也会和抽签记录打架。**决策时刻的腿 = `round_inputs.rolls` 里那条
`route` 记录的 `pool`（权重 = 货量）**，而且是逐舰的 ⇒ 零新增序列化就够了。判据见 g2
`dispatch_checks`：3 seed × 400 回合 **909 条** route 抽签逐条精确复算（`value×pool_total` 切段 ==
`picked`），比原来「合成世界掷 4000 次看 3:1」更严。

### §5.6 第 6 批（合成场景 · 拨控制叶）——**已落地**

**地基**（`play/tests/_harness.py`）：按原案 **A**（不选 B）——新增
`scenario_apply(name, seed, rounds, diffs, patch=None, start_round=0)`：
`gen`（可先捏状态字段）→ 逐份 `--apply diff.json --round 0 --quiet --save cur` →
`run_into(out, …, extra=("--start", cur))`。选 A 的理由与原案相同：`state.control` 是**字典型**的
叶库，Python 里抄一份它的形状必然漂移；而 `--apply` 的 diff 形状**已定义好、g4 在逐条对账**
（`src/control/wire.rs` 的补丁结构体）。代价只是多几次 CLI 调用（零点几秒）。

几条坑写进了 docstring，**下一批别再踩**：

* **写值即接管**：只写值、不写 `mode` ⇒ 那片叶归 `Player`。想让 AI 继续管它（例如「这张**自建**图
  该被回收」）必须**显式** `"mode": "Inherit"`——不写，A/B 会整个反过来（Rust 原件里那条注释说的
  就是这件事）。
* **一条 diff 常常就够**：`apply_diff` 里 `blueprints` **先于** `buildings` 应用
  （校验用的是**本份 diff 之后**的意图）⇒「建图 + 把建造区指过去」一次成功，不必写两份。
* **回执不再静默**：`WARN_APPLY_SKIPPED`（叶片没落地）进 `h.warnings`，而 `Harness.report()`
  现在**真的会打印它们**——在这之前 `warnings` 只进列表、没有任何人读，「不静默」是句空话。

**搬走的**（全在 g2 的 `blueprint_scenario_checks`，合计 17 条判据）：前三条**整搬**（Rust 原件删了、
模块头留了对账表），第四条只搬**活回合那一半**：

| 原用例 | 怎么造那个世界 | 数据级断言（读面） |
| --- | --- | --- |
| `a_dangling_pointer_is_left_dangling` | `h.scenario_apply`：**两次 diff**——① 建一张普通的自建图 + 把建造区指过去；② **删掉那张图**（`{"remove": true}` ⇒ 指针悬空） | `cities.建筑[].设计图` 逐回合原样是那个**已删掉**的名字；防空转 = 它真的不在任何图库里 + 同一局 AI 真的给别人建了图 |
| `a_player_pinned_design_and_its_yard_are_left_alone` | `h.scenario_apply`：**一份** diff 里建图（`mode: Player`）+ 把建造区指过去 | `blueprints`：逐回合 `mode=Player`、`选装` 一字不变；`cities`：指针逐回合不变；`decisions` 里那张图**零行**（重估/回收都没碰） |
| `only_unreferenced_selfmade_designs_are_reaped` | `h.scenario_apply`：三张没人指向的图（`Inherit` 自建 / 玩家起的名 / 玩家钉住的 AI 名） | 自建的回合 0 还在、之后没了；另两张 0–3 回合都在；`decisions` 里有 `verdict=reaped`；防空转 = 三张图回合 0 的模式真的各就各位 |
| `build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling`（**读面那一半**） | `h.scenario_apply`：**状态补丁先把国库垫到 `1e6`**（见下）+ **一份** diff——`buildings` 叶同时 **`blueprint: null`（拆指针）+ `ship_type`（钉死舰级）**（免得 `retool_shipyards` 换掉那一行），再把 `construction_budget`/`investment_budget` 拨到 `1e6` / `0` | `city_process.build`：批满 ⇒ 顶到 `increment ≈ rate`（产能封顶）；批 0 ⇒ 键**还在**、`rate > 0`、`increment = 0`（是缺钱不是没船坞）；两边的 `rate` 逐回合相同（产能与钱无关） |

> ⚠ **「批满」不等于「钱管够」——先把国库垫到维护 reserve 之上**（踩了两次，写死在这儿）：
> P1-5 之后 Player 写的 `construction_budget` 还要再乘
> `con_scale = clamp((库存价值 − 维护 reserve) / 建舰上限, 0, 1)`（`autocontrol/budget.rs`）
> ⇒ **库存不到 reserve 时，写 `1e6` 也是 0**。在这条落地之前，「批满 ⇒ `increment ≈ rate`」在活回合
> 里也只撑得住 1 回合（实测 `main@1ccbb2c`：回合 1 是 `10.0 == 10.0`、回合 2 → 7.27、回合 3 → 0）；
> P1-5 落地后**连第一回合都是 0**。所以 g2 与 Rust 原件（`src/tests/sim/spending.rs`）**两边都显式
> 垫库存**——那是**隔离变量**（把「钱」这一个变量孤出来），不是作弊。
> 教训：**从单测搬过来的等式，先问一句「活回合里还有没有第二个瓶颈」**。

> ⚠ **原案的两处已被取代**（写在这里免得下一个人照旧文档重做一遍）：
>
> 1. 原案说悬空指针「`cities` 整表替换即可，**不需要**控制叶」。**不要那么做**——那要往状态里
>    手塞一个不存在的图名，而 `--apply` 的 `remove` 就能造出同一个局面，且**更忠实**：
>    「删**整张图** ⇒ 挂它的建造区随后是悬空指针 ⇒ 停产」（`src/control/blueprint.rs`），而删图
>    本来就是玩家/agent 的动作。写面**拒绝**的只是「凭空写一个不存在的图名」（§4 那半照旧不搬）。
> 2. §6.5 的老交接说「先补字典型深合并或 `--apply`-in-scenario」——**只有 A 需要**（见上）；
>    ④ 的「拆指针 + 钉舰级」也用 `BuildingPatch` 的 `blueprint: null` / `ship_type` 走了 `--apply`。
>    于是**读状态字段只剩两处**，都在 `_yards_of` / `_stock_patch` 里（一个是「认出某势力有哪些
>    建造区」，一个是「把国库垫厚」），后者在下面那个 ⚠ 里有说明。

**跟 `main` 的字段命名批对齐**（合并 `main@2b8a871` 时被它逼出来的）：`_harness` 删掉了手抄的
`_ID_KEY`，改成问引擎（`Harness.identity_keys()` ← `--nouns` 的 `identity.structs` + state schema）。
g2 这边的 `_yards_of` 跟着走：**身份键问引擎**，剩下三个名字（`建筑`/`建筑编号`/`建造舰级`）引擎
还没有声明面，只能写死——但它们错了**不会静默**（找不到建造区 ⇒ 探针那条判据立刻红）。
另外「这个势力有哪些资源」改成读 `--control` 的预算模板（`resource` 是 ASCII 键，且实测与
`state.factions[].资源` 逐一对上）——**资源名不再手抄**，只有「国库那个字段叫什么」（`_stock_patch`）
还得写死。

**还没搬的**（别重新论证）：

* `the_ai_creates_a_design_for_every_yard_it_owns`——要「每个建造区的**有效**归属」，读面只有逐个区
  自己的 `blueprint` 指针，判不出「AI 该不该给它建图」。
* `blueprint` 的角色/姿态那两条（`shipbuilding.rs::player_pinned_blueprint_is_not_retooled` /
  `an_auto_blueprint_is_retooled_as_a_blueprint`）——**等字段命名批 B/C 收口后**再搬。
* `sim/spending.rs::build_lines_separate_…` 的**「不看库存」那份**（直接调一次 `step_construction`）
  ——活回合里库存是第二个瓶颈，见上面那个 ⚠；两份都留是**故意**的，不是漏删。

## §6 接手须知：动手时的工具、命令与坑（照这个做，别重新发现）

### §6.1 三件事分别在哪儿做

| 要动的东西 | 文件 | 注意 |
| --- | --- | --- |
| 加一列/一张**派生表** | `src/projection.rs`：写完 `writeln!` 还要在**同一个文件**的 schema 声明里补 `columns` + `column_docs`（**每一列都要有一句话**，g4 会逐个对账） | 加表还要在 `write_index_seeded` 里开文件、在 `schema.json` 的 `derived` 段登记 |
| 新表的**声明纪律** | `play/tests/g4_spec.py`（静态 + 读面对账 + 认领完整性）+ `play/tests/_g4_negative.py`（**给纪律自己做的量具**：注入错，必须全咬住） | 新列/新表如果没人「认领」，g4 会红——这是**故意**的 |
| Python 侧读新表 | `play/planet_xq`（kit）：`KIT.load(dir, only=("新表", …))` + `q.table("新表")`；`meta.json` 里的配置量走 `q.meta[...]` | 表没在 `only=` 里会**响亮报错**（不会静默给空表） |

### §6.2 验证命令（每批都跑这三条）

```bash
# ① 行为中性（装数据的那次提交**必须**逐字节不变；变了就是改到模拟/读面语义了，要停下来想清楚）
target/release/planet_x.exe --seed 42 --round 240 --digest 20   # 取 sha256 == 53096F5F…94DC
#   ⚠ 这个基线**只在**「已同步的 main 是 1ccbb2c」时有效——main 还在动（见下）。
# ② 数据级（116→ 每加判据都会涨；`-j 7` 并行）
uv run --project play/planet_xq python play/tests/run.py all -j 7
# ③ Rust 侧（搬一条就该少一条）
cargo nextest run -P full
```

> ⚠ **digest 基线的历史**（施工图早期写的 `BB2EEB2B…2000` 已作废）：
> `BB2EEB2B…2000`（`feature/web-control-spec` 合并**前**）→ `C928C3F1…06A9`（合并后）→
> `748B4AA6…9603`（`feature/field-names` 批 A：11 个实体结构体 83 个字段加 serde 中文名 +
> 字段顺序；见 `.agents/notes/field-naming.md` §7.5）→ `F550E199…DBB37`（合 `main@2b8a871` 后）
> → **`53096F5F…94DC`**（合 `main@b12aee6` 后实测；再合到 `main@c11f054` 仍是它——那批 P2 清尾
> 没动 240 回合的轨迹；见下）。
>
> **第 6 批自己是行为中性的**（`748B4AA6…9603` 在批次前后逐字节相同：整批只动 `play/tests/*`
> 与 `src/tests/*`，后者是 `#[cfg(test)]` ⇒ 连 release 二进制都不重编）。`F550E199…DBB37` 与
> `53096F5F…94DC` **都是 `main` 的引擎改动带来的**（那批 P0/P1：出口腿保留量、逐城预算门、
> 市场世界库存口径、定价…），不是第 6 批。
> ⚠ 那个批次的作者自己在本子里写了「P0-3/P1-2/P1-4 会改默认行为，**需重标 digest**」——
> **main 还在动**（合并窗口里从 `2b8a871` 一路走到 `8abbba2`），所以合流时**以当时实测为准**，
> 别拿这里这个数当永久口径。

### §6.3 合成场景的 API（`play/tests/_harness.py`，已就绪）

```python
h = Harness("release")
w = h.gen(Path("….json"), seed=42)                  # 造世界（JSON 档 ⇒ Python 能改）
st = h.state_dump(w)                                 # 读状态（ships/cities/factions/control…）
proj = h.scenario("名字", 42, 3, patch={"ships": {"长城": {"hull": 3.0}}})   # 造→捏→推进→投影
#   patch 的形状 = 读面同名同形；打错的字段/点不到的名字会进 h.warnings（**不静默**）
#   整表替换也行（例：改一个建筑 ⇒ 把新的 buildings 列表整个塞回去）
proj = h.scenario_apply("名字", 42, 3, [diff, …])    # 造→**逐份 --apply 拨控制叶**→推进→投影
#   diff 的形状 = `--apply` 的补丁（{"control":[…], "scope":…}，g4 在逐条对账）——Python 里
#   不出现第二份 `state.control` 形状；⚠ **写值即接管**（不写 mode ⇒ 归 Player，见 §5.6）
```

### §6.4 四条**必须**遵守的规矩（踩过，都是血）

1. **同一个数只有一个位置**：能从别的列 join 出来的，不要再发一列（`spending.rs` 的注释里有先例：
   「批了多少」留在控制面，「真花掉的」进读面，相减即得）。
2. **「这一步没跑」给中性缺省，不是 0**：`is_hub=false`、`labor=1.0`、`governance_scale=1`…
   写成 0 会被读成「能力归零」。中性值表在 `src/model/neutral.rs`，有守卫盯着。
3. **⚠ 同回合相位错位**（[test-decoupled-suite.md](test-decoupled-suite.md) §12）：同一回合里
   「按城算的量」与「按势力算的量」在**城易主 / 势力城集合变化 / 城被复垦**时**必然对不上**。
   写判据时要么排除这几类、**要么把排除本身也写成判据**（「被排除的每一处都要有解释」）——
   否则排除就是藏违规的后门。
4. **搬一条删一条**：Rust 原件删掉，并在该模块头部留「搬去哪儿 + 为什么剩下这些」的对账表
   （`src/tests/{sim,autocontrol}/*.rs` 顶上已经有好几份样板可抄）。

### §6.5 现在的状态（**2026-10 第 1–6 批后**）

* 分支：`feature/test-migrate-rest`（worktree `C:/resource/planet_x-decoupled`）。第 6 批提交后**又合了
  三次 `main`**（`cc71e35` → `main@2b8a871`、`347301e` → `b12aee6`、`b618073` → `1ccbb2c`）。
  ⚠ `main` 是**多会话共用**的、**动得非常快**——这次窗口里它从 `704d008` 一路走到 `1ccbb2c`
  （P0-1/P0-2/P0-3/P1-1/P1-2/P1-4 陆续落地，我干活期间平均几分钟一个提交）⇒ **动手前先
  `git merge main` 同步**，做完三条验证再合回 `main`；**别假设上次看到的 main 还是那个 main**。
* 计数：Python **171**（g1 43 / g2 79 / g3 26 / g4 23）；Rust **205** + 31 探针 `#[ignore]`。
  （g4 18→23 与 Rust 的绝大部分增量是**同步 `main` 带进来的**；第 6 批自己的增量是 g2 62→79。）
* 当前 digest 基线：**`53096F5F5878220EAD555B6F03B438D11712494DE167C6CFEEAA8DB7158394DC`**
  「第 6 批 + `main@1ccbb2c`」实测（§6.2 有这段的历史）；第 6 批自己那一段仍是 `748B4AA6…9603`
  （整批只动 `play/tests/*` 与 `src/tests/*` ⇒ **行为中性**，实测确认）。
* 四条门在合并后的树上全绿（实测：`g1` 43/43、`g2` 79/79、`g3` 26/26、`g4` 23/23、
  `nextest -P full` 205/205、`nextest -p planet_x_web` 25/25、`_g4_negative.py` 25 个注入错全咬住）；
  `target/test-fixtures/` 有自动清理（`run.py` 默认 `sweep_stale`，`--no-sweep` 可关）。
* **两次「红」的记录**（都不是第 6 批的，但值得记——下次判断「谁弄红的」直接用这两条）：
  1. 同步到 `main@2b8a871` 时 g1「预算守卫没有空转」红（`0 处真的花过钱`）——是 main 那批
     P0-1 带来的，**P0-2（`01719eb 逐城预算门与总账分离`）已经修掉**。
  2. g2 新搬的那条预算 A/B **红了两轮**——都是**我自己的判据过宽**，不是引擎坏了：
     P1-4 改市场定价 ⇒ 库存成了第二个瓶颈（`main@1ccbb2c`：批满 1e6 的同一座城回合 1 顶到
     `rate`、回合 2 → 7.27、回合 3 → 0）；P1-5 又给 Player 预算加了维护 reserve 的 `con_scale`
     ⇒ **连第一回合都是 0**。最后按 Rust 原件同一把尺子**先垫国库**才修好（§5.6 那个 ⚠）。
     教训：**从单测搬过来的等式，先问一句「活回合里还有没有第二个瓶颈」**。
* 判据写在 `play/tests/g*.py` 的 `run()` 里（数据取自 `extract()` 的摘要 ⇒ **改断言不重读投影**）。
  ⚠ `_code_stamp` 把**除 `run` 外的全部顶层函数**算进摘要指纹 ⇒ **加一个新判据函数会让该组摘要重算一次**
  （一次性十几秒，不是缓存坏了）。
* **第 6 批已合 `main`**（`fadca4e` = 快进；`.agents/notes.md` 的索引行没跟着改——那次合流时另一个
  会话正压着同一个文件，改它就得再动一次别人的工作区，不值得）。**再往后 = 第 7 批**：按 §4 那份
  **已写死**的「故意不搬」清单收尾（不要再逐条重新论证）。§5.6 里还剩两条**等字段命名批 B/C 收口**的
  （`blueprint` 的角色/姿态）。
