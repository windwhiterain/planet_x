# 测试全搬的施工图：**判据缺什么数据，就往序列化里装什么**

> 状态：**第 1–6 批已落地**（2026-10；分支 `feature/test-migrate-rest`，worktree `C:/resource/planet_x-decoupled`，
> 第 1–6 批**都已合进 `main`**（第 6 批 `fadca4e`，快进））。
> §4 的「故意不搬」清单已写死（第 7 批）——**不用再逐条论证**。
> 计数：**Python 355**（g1 77 / g2 202 / g3 47 / g4 29）；**Rust 153**（+31 探针 ignored）。
> **sim 74 → 13**（第 7 批搬走/删掉 61 条；其中 1 条是只打印的探针）。
> **整族搬空并删文件**：`combat.rs`、`shots.rs`、`fleet.rs`。
> 起点口径（本主题开工时）：Python 75 / Rust 222 单测 + 8 集成。（2026-10 用户裁决：*「总之目标是全搬，有需要的数据没序列化就把他装进序列化里」*）
> ｜ 索引：[notes.md](../notes.md) ｜ 上层：[test-decoupled-suite.md](test-decoupled-suite.md)

## §0 现在的账

| | 数据级（Python） | Rust 侧 |
| --- | --- | --- |
| 起始（本主题开工时） | 75 | 222 单测 + 8 集成 |
| 施工图写下时 | 120（g1 33 / g2 46 / g3 26 / g4 14） | 206（+31 探针） |
| 第 1–5 批后 | 146（g1 43 / g2 62 / g3 26 / g4 15） | 200（+31 探针 `#[ignore]`） |
| **现在（第 7 批：搬到 `blueprints`/`fleet`/`site_supply`）** | **215**（g1 58 / g2 106 / g3 28 / g4 23） | **189**（+31 探针 `#[ignore]`） |

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

### §5.7 第 7 批（把 `sim` 搬空）——**在做**

**为什么做**：用户要把 Rust 门压下来（「rust 侧 test 不跑 sim」）。先量了账：**光过滤只省 ~2 s**
（sim 77 条里 70 条会跑，真跑 ~2.3 s），**要省到编译那 2–3 s 必须把 `src/tests/sim/*.rs` 真删掉**。
2026-10 已把一轮门从 23 s 压到 **7–8 s**（`[profile.test] opt-level = 1` + `tests/` 四探针合一，
见 `test-wall-clock.md` §0.2/§0.3）⇒ 现在按族搬便宜得多。

**分类口径**（74 条 → 71 条）：
* **A 已重复 ⇒ 直接删**（本轮做完，见下）
* **B 搬得动**（用第 6 批的 `h.scenario_apply` 摆场景 / 读面本来就够）≈ 45 条
* **C 要新 `--call` 或新读面列** ≈ 12–15 条
* **D §4 明说不搬**（内部契约 / 手工世界 / 错误路径）≈ 6–8 条 ⇒ **最后要用户裁决**：
  留一个**写死理由**的白名单，还是把它们降级成 `#[ignore]` 探针

**A 类（已落地）**：删了 3 条，判据都被 g1 现有判据**严格覆盖**：

| 删掉的 Rust 原件 | 现在住哪儿 | 为什么不是丢判据 |
| --- | --- | --- |
| `sim/tests/fleet.rs::advance_populates_round_events` | g1「全新开局的回合 0 没有事件」+「推进过就有事件」 | 原件只断言「回合 0 空 → 跑几回合非空」，那两半读面上都在（`events` 表按 `round` 分组）⇒ **先补 g1 判据、再删原件** |
| `sim/tests/inputs.rs::pre_is_the_input_face_not_an_observation_copy` | g1「输入面里没有观测字段（它属于 post）」 | **同一份 banned 名单**逐字；「`pre` 必须有 `order`/`relation_noise`」那半由 g1「输入面没有空转」覆盖 |
| `sim/tests/inputs.rs::the_input_face_reproduces_byte_for_byte` | g1「同 seed 重跑逐字节一致」 | 那条比的是**整份投影每个文件**的 sha256（`round_inputs` 在里面）⇒ 严格更强 |

**B 类·第 1 族 `capital`（已落地，4 条 → 整文件删除）**

读面本来就够（`decisions[kind=capital]` 带 `verdict ∈ {forced, review, relocate}` + `detail`
里的 `reviewed`/`candidate`/`current_cost`/`candidate_cost`/`relocated_from`，加上
`factions.capital_body` 与 `cities.{人口, 已焚毁, 天体名}`）：

| Rust 原件 | 现在住 | 怎么判 |
| --- | --- | --- |
| 亡城强迁 ⇒ 人口最高的活城 | g3 `capital_checks` | `verdict=forced` ⇒ 不评估、无判据数字、`capital_body == target`、`target ∈ 人口最高那一组天体` |
| 周期评估 ⇒ 迁到人口中心 | g3 `capital_checks` | `verdict=relocate` ⇒ `reviewed`、`candidate == target`、`候选成本 + 门槛 < 现成本` |
| 判定是稀疏的 | g3 `capital_checks` | 非 `forced` 的判定只在 `capital_review_every` 的整数倍回合；`review` 行无 `target`、两头判据数字都在 |
| Player 钉的首都不被覆盖 | g2 `capital_scenario_checks` | **合成场景 A/B**：`--apply` 写 `首都` 叶，`Player` / `Auto` 各跑 49 回合（≥3 个评估轮）；`Player` ⇒ 零 `review`/`relocate` 行，`Auto` ⇒ 必须留下评估行（否则空转） |

⚠ 两个踩过的坑（都写进了代码注释）：
1. **`target` 的 `null` 到 pandas 是 `NaN` 不是 `None`**——我第一版用手工探针读原始 JSON（`null` →
   `None`）所以没踩到，写进 g3 立刻红了 1042 处。凡是从 `q.table()` 的**列**里取可空值，
   都要 `pd.isna` 归一化。
2. **并列第一**：s42 r237 月球与天王星都是 200 人，引擎挑了天王星 ⇒ 判据必须是
   「`target` ∈ 人口最高**那一组**天体」，不能只认 `max()` 的第一个。
3. 「Player 不被覆盖」单看一边**证明不了任何事**：默认 `admin_range = 6.0` 而水星到地球才
   0.61 AU ⇒ 评估本来就不想迁。必须补 `Auto` 那半边（同位置、同窗口，它**必须**被评估过）。

**B 类·第 2 族 `inputs`（已落地，3 条 → 整文件删除）**

`round_inputs` 早就在读面上（`order`/`relation_noise`/`rolls`），补齐 `ships`（**保持世界的舰序**）
+ `events[ship_destroyed]` + `factions` + `meta.diplomacy.noise` 就够：

| Rust 原件 | 怎么判（g1 `input_face_shape`，逐回合） |
| --- | --- |
| 解算顺序是不重不漏的名单 | 无重复、覆盖回合末还活着的舰、**多出来的每一个都是本回合 `ship_destroyed` 的**、顺序真的被打乱过（≠ 舰表顺序） |
| 关系噪声覆盖每一对、落在 `±noise` | 对数 = `n(n-1)/2`、无自环、`|v| ≤ meta.diplomacy.noise`（`noise = 0` ⇒ 这一节必须为空） |
| `rolls` 的形状与内容 | 逐条：`value ∈ [0,1)`、`faction`/`subject` 不空、闸门 xor 加权抽签（导航是第三种：幅度骰）、池非空且**权重和 = `pool_total`**、`picked` 在池里 |

实测 12 回合（`INPUT_ROUNDS` 没改）就盖住**全部 17 个用途**（Rust 原件要的 10 个都在），
1443 条抽签逐条判过。

**B 类·第 3 族 `story`（半落地）**

`meta.story` 把后果**声明式**发出来了（`{"kind":"relations"|"grant_resources"|"grant_ship"}`）⇒
g2 的新判据**不写死**「prologue 在第 1 回合给谁降多少」，而是逐条对着 `factions.关系` 看方向
（3 seed 共 **21 处**）。

* `story_effects_apply` ⇒ **整条搬走**（g2「剧情的机械后果真的落到读面上」）。
* `story_grant_ship_spawns_a_fleet_member` ⇒ **留下**：它要断言出厂位置**恰好是天体当前位置 +
  (0.05, 0.05)**，而读面上的 `bodies` 表是**静态**的（只有轨道根数，没有逐回合位置）⇒
  在 Python 里算那个位置就是把轨道公式抄第二遍（§4 明说不搬）。

**C 类第一刀：新派生表 `body_positions`（已落地）**

**动机**：`bodies` 是**静态**母表（只有轨道根数 + 写表那一刻的位置），而天体在动、`ships.x/y` 是
绝对坐标 ⇒「这艘舰此刻**相对某个天体**在哪儿」以前**没有读法**。一条表解锁一族。

* `src/projection.rs`：`DERIVED` 加 `body_positions`（`join_on = "天体名表"`、`round = true`）+
  writer + `write_round` 发射 + `projection_schema` 的条目（**每列一句话**，g4 会逐个对账）。
  列：`round / 天体名 / x / y`（AU，`r2`；与 `ships.x/y` **同一把绝对坐标尺子**）。
* **纯追加**：不参与任何计算 ⇒ **digest 逐字不变**（实测）。
* 既有守卫自动盯上它：`src/tests/projection/mod.rs::derived_tables_are_written_and_declared`
  （`DERIVED.len() == schema.derived.len()` + 每张声明的表真的写出来）。

**靠它搬走的两条**（sim 63 → 61，Rust 198 → 196）：

| Rust 原件 | 现在住 | 关键 |
| --- | --- | --- |
| `fleet::dock_follows_body_and_idle_holds_position` | g2 **合成场景 · 停泊与待命**（4 条判据） | 钉成 `Player` 后：`Dock` 逐回合持久 + 到目标天体的距离**逐回合缩短**（30.79 → 19.59）；`Idle` 的位置**逐字不动**。⚠ 防空转在 `Idle` 那边：窗口里天体真的在公转（否则「位置不动」是废话） |
| `story::story_grant_ship_spawns_a_fleet_member` | g2「剧情 `grant_ship` 的出厂位置与指令」（2 条判据） | 出厂位置 = **天体这一刻的位置 + (0.05, 0.05)**、指令 `Idle`；判据由 `meta.story` 的 `grant_ship` 驱动（3 seed 共 15 次）。⚠ 读面 `x/y` 过 `r2` ⇒ 偏移只能判到 **±0.01**，判不到 1e-9 |

⚠ **一条踩过的坑（值得记）**：`fleet::dock_…` 一开始我想写成**长局不变量**「`order_effective` 是
Dock ⇒ 它在动」——**错的**。实测长局里 `Dock` 的 797 个「两回合同天体」样本**全部原地没动**，
因为 AI 会在回合末刚派完 Dock、下一回合开头就改派 ⇒ 那些叶子**从没执行过**。必须像原件那样
**钉成 `Player`** 才是这条用例本来测的东西。

**第 7 批逐族流水（sim 74 → 54）**

| 族 | 结果 | 落在哪 / 靠什么 |
| --- | --- | --- |
| A 类 3 条重复 | 删 | g1 早有严格覆盖 |
| `capital` 4 | 整文件删 | g3 `capital_checks` + g2 合成场景（A/B） |
| `inputs` 3 | 整文件删 | g1 `input_face_shape`（逐回合） |
| `story` 1 | 半（另 1 条后来靠 `body_positions` 也搬了） | g2「剧情后果落到读面上」 |
| `mod.rs` 1 | 删（该文件现在**只剩夹具**） | g1 `world_shape` |
| `governance` 1 | 删 | g1 `neutral_defaults` |
| `blueprints` 2 | 删 | g2 悬空指针停产 + 买不起等钱（A/B） |
| `fleet` 2 | 删 | g2 陈旧的跟随 + 停泊/待命（靠新派生表） |
| `site_supply` 2 | 删 | 新挂 5 个 `--call` + g2 `site_ledger_checks` |

**两处读面/接口扩了**：
1. **派生表 `body_positions`**（`round / 天体名 / x / y`）——`bodies` 是静态母表、`ships.x/y` 是
   绝对坐标，「这艘舰相对某天体在哪儿」以前没读法。纯追加 ⇒ digest 不变。
2. **`--call` 家族**：`site_reserve` / `exportable_at` / `site_deficit` / `lane_rounds` /
   `site_ledger`。注意 `call_function(&config, &state, …)` **拿到了 state** ⇒ 有状态的纯函数也能挂。

**⚠ 三条踩过的假绿/空转（都已写成注释）**：
* `q.table()` 的**列**里 JSON `null` 是 **NaN** 不是 `None`（g3 立刻红了 1042 处）。
* 同人口要**并列任取**（s42 r237 月球/天王星都 200 人）。
* `site_ledger` 只按 `state.depots` 收站点 ⇒ **回合 0 是空表**；垫国库时用了**预算表**的键
  （`硅/碳/铁`）而选装要 **氦-3/金** ⇒ 「垫厚」那一臂其实还是穷的。**两处都是判据会绿但没在测**。

**第 7 批后半（同一轮里继续，sim 54 → 45）**

| 族 | 结果 | 靠什么 |
| --- | --- | --- |
| `blueprints` 5 | 删（该文件只剩 1 条） | g2 一个合成场景两臂（只差图的 `归属`）：选装/角色/舰队默认/倾向/`order_source` |
| `knowledge` 2 | 删 | g3 MOND（棘轮 **7652 个顶上势力·回合**、涨 ⇒ 带内必有舰 **1323 次**、前沿**夹逼对账** 477 行）+ g1 开局打点**从配置读** |
| `shots` 2 | 删 | g2 齐射一级对账（561 条齐射 / 566 发）+ 「至少一发真打出去」 |

**再挂两个 `--call`**：`mond_frontier`（纯函数，掌握到顶给 `null`）。

⚠ 这一轮又抓到三处「判据会绿但没在测」：
1. **`势力` 过滤**：g2 的「没图可言的舰」第一版没按势力收窄 ⇒ 把别国的舰混进来，角色集合里多出 `Freight`。
2. **前沿对账的容差**：`MOND 掌握度` 自己过 `r2`，而前沿按**全精度**掌握度算 ⇒ 逐值相等差 ~0.015。
   改成**夹逼**（拿引擎函数在 `m ± 0.005` 求值）后不再需要任何人肉容差常数。
3. **`锈 ⇒ 带内没舰` 不成立**：读面给「带内舰数」，目标是「在场强度」（逐舰深度不同）
   ——实测 123 次回落里 **9 次带内有舰**。只能搬「涨 ⇒ 带内有舰」那半。

**第 7 批第三段（sim 45 → 41）**

| 族 | 结果 | 靠什么 |
| --- | --- | --- |
| `mond::route_depth_measures_mond_immersion` | 删 | 新 `--call route_depth`：原件用**裸坐标**（`[r±k, 0]`），半径从 `meta.mond.radius` 读 ⇒ **逐字可复现** |
| `ideology::ideology_similarity_ranges_and_is_monotonic` | 删 | 新 `--call ideology_similarity`（键名与读面 `factions.思潮` 一致）；判据比原件**更强**（加对称 + 单调） |
| `combat::damaged_components_repair_in_friendly_territory` | 删 | g2 合成场景：**捏一艘舰的组件/坐标**（本土 **+1.8/回合** vs 外海 **+0.72/回合**） |
| `fleet::newly_built_ships_have_no_order_of_their_own` | 删 | g2：3 seed 里 **594 艘**新舰的 `order_leaf_mode` 只会是 `Inherit`、`order_effective_mode` 只会是 `Auto` |

**一条重要的能力发现**：`edit()` 要「带身份键的行表」，而**舰 / 城 / 势力都有身份键**
⇒ 它们的字段（坐标、组件、组件耐久、资源、建筑 …）**都能捏**。这是本段两处 A/B 的基础。
**只有 `depots` 不行**（复合键的 map `"中国|水星"`）——它是 `site_supply` 那两条 A/B 的硬墙。

**试过但搬不动（都记在各自模块头）**：
* `spending::upkeep_shortfall…`：原件是 `step_upkeep` 的**单元测**——整回合里产出先到账，
  把库存砍到半价也当场补上（实测欠费恒为 0）⇒ §4 内部契约类。
* `haul::a_haul_route_alternates_legs_because_of_the_cargo`：读面**给不出路线的 from/to**，
  而承包投递的「卸货端」与「货主」不是一回事（单路线舰里仍有 196 处反例）。
* `ideology` 的三条 `step_ideology` 单元测：要**往世界里注入事件**（读面没有事件的写入口）。

**第 7 批第四段（sim 41 → 36）：全是「合成场景」**

| 原件 | 判据 | 实测 |
| --- | --- | --- |
| `haul::a_commanded_haul_route_delivers_depot_cargo_into_the_capital_pool` | g2 玩家钉的常驻运输线（6 条） | 指令 **61 回合一字不变**；r40 装 1.7 件 → r41 **进池**卸货 |
| `ideology::entertainment_holds_a_distant_city` | g2 重金娱乐拉住远城（4 条） | 重金臂 `0.35→0.52` 逐回合不降；**对照臂 `0.35→0.31` 真在下滑**（防空转） |
| `fleet::colonize_keeps_player_ownership` | g2 殖民（6 条） | r33 建城、一次性指令花掉变 `Idle`、**叶片全程 `Player`**；早退臂同 |
| `ideology::low_loyalty_city_defects_to_most_opposing_ideology_instead_of_razing` | g2 改旗易帜（4 条） | r1 倒向相似度**最低**那家（`0.0` vs 其余 `0.5`，相似度用 `--call` 现算） |
| `combat::fire_degrades_components_under_damage` | g2 组件损耗（3 条） | **22,644** 个「没挨打」的舰·回合**零掉血**；238 次真伤里 7 次观察到组件下降 |

**这一段的通用配方**（值得复用）：**捏 + 钉**——
`h.scenario(patch=…)` 捏**有身份键的实体**（势力思潮/国库、城市忠诚度、舰的坐标与组件），
`--apply` 钉**玩家叶**（`福利预算`/`城市福利预算`/`Haul`/`Colonize`），然后只看读面。
**起点回合与目标天体从长局里扫出来**（殖民那条第 29 回合才有空定居点），不写死。

⚠ **这一轮抓到的三处「绿了但没在测」**（都写进注释了）：
1. **思潮从对照局读**：改旗易帜那条第一版把 `思潮` 从**没打补丁**的投影读 ⇒ 相似度算的是默认值，
   判据只是**碰巧**还是那一家（修正后是 `0.0` vs `0.5`，才算真的在测）；
2. **`colony_founded` 的 `target_id` 是城名**，天体在 `data.body`——按天体找一条都找不到；
3. **`haul_steps` 的键集与「活着」两半有回合末伪影**：表里多出的舰是**回合末被改派**的，
   而表里 22 行「已沉」是**同回合晚些才沉**的 ⇒ 这两半读面证不了（那条留在 Rust）。

**第 7 批第五段（sim 36 → 29）：补读面，一次解锁一簇**

| 新读面 | 解锁 |
| --- | --- |
| `factions.贸易禁运`（`{禁运方: 档位}`）+ `--call trade_block_cause` | `trade::trade_block_list_names_the_blocker_and_the_tier`（3 seed **17,712** 条；同源复核逐条目相等） |
| `--call mond_drift` / `mond_arrival_chance` | `mond` 两条**纯函数测**（`roll=0` 必然蒙对、**任意有限深度都还有胜算**、门槛 = `arrival_eps ÷ drift_per_au`） |
| `factions.mond_presence` / `factions.mond_target` + 两个同名 `--call` | `knowledge` **三条**（带内 ⇒ 强度 0；强度 = 舰数 × `(1+深度×权重)`；深驻泊第 48 回合学满） |
| （不需要新面） | `haul::a_haul_route_alternates_legs_because_of_the_cargo` —— 声明的 `Haul{from,to}` **本来就在** `ships.order_effective` 上，3 seed **11,621** 步零违规 |

**两条方法论**（都值得复用）：
1. **先问「这个量读面上有没有」再动手**：haul 那条我第一版去「推」路线（拿观测到的装卸天体反推），
   单路线舰里还有 196 处反例——而**声明的路线就在读面上**。推出来的东西不可信。
2. **逐回合定律能照抄，但要先配对"哪一行配哪一步"**（⚠ 这条我一开始搞错了，见下面「订正」）：
   `m' = m + rate×(target−m)` 用**目的回合那一行**（轴的 `r → r+1` 用第 `r+1` 行的目标），
   而不是源回合那一行。配错一格 ⇒ 看起来像"相位差"（实测 0.0267），其实全是自己的 bug。

**第 7 批第六段（sim 29 → 25）：又是「捏 + 钉」+ 一个新调用**

| 原件 | 判据 | 实测 |
| --- | --- | --- |
| `trade::freight_gap_is_the_same_ledger…` | 新 `--call freight_ledger`（`capacity_ledger` 放开成 `pub`） | 2 seed、**12 个势力·回合**、**28 处缺口**：逐条自洽 + `Σ缺口÷Σneed` **就是读面那一列**（差 ≤ r2 舍入 4.7e-3） |
| `governance::a_mond_master_keeps_a_deep_city_loyal…` | g2 合成场景（只拨 `MOND 掌握度`） | 凡人 **0.50→0.37**、掌握者 **0.50→0.62**（差 0.25 > 0.2、城没丢） |
| `combat::combat_respects_shields_and_speed_evasion` | g2 合成场景（造一仗） | `damage 12.6 / absorbed 8.82 / hull_pen 8.19`；守方护盾 12→4.73、船体 24→18.21 且没死 |
| `combat::fleet_air_defense_covers_nearby_missile_targets` | g2 合成场景（两臂只差 PD 友舰坐标） | 近处拦截 **4.0**/伤害 **4.1**；远处拦截 **0.0**/伤害 **8.1** |

⚠ 三条「试过但不行」的经验（都写进模块头）：
1. **`mastery_does_not_pay_the_governance_bill` 够不到**：它要「覆盖率 0 ⇒ 欠费暴跌支路」，而
   **完整回合里产出先到账**，覆盖率恒 > 0（国库清零后忠诚仍稳在 1.0）⇒ 判据会退化成
   「两臂都是 1.0」的假绿。留 §4。
2. **`control_rusts_back_when_the_fleet_leaves` 也够不到**：换了新的**强度**列再试一次，
   123 次回落里仍有 **9 次强度 > 0**——两列都是**回合末**的值，而更新用的是走那一刻的强度。
3. **`a_hired_delivery_splits…` 差一个读面来源**：`contract_delivered` 有 `amount`/`cut`/两端，
   但**分账两头进哪个池子**没有读面记录；扩事件字段会动 digest ⇒ 不能扩。

**第 7 批第七段（sim 25 → 23）：把「造一仗」的配方用到底**

| 原件 | 判据 | 实测 |
| --- | --- | --- |
| `shots::a_fully_intercepted_salvo_still_leaves_an_event` | g2 合成场景（守方装**两层**点防） | `pd=12.0`、`pd_absorbed=8.1`、**`damage=0` 而事件照样在**；空手臂 `damage=8.1`（防空转） |
| `fleet::follow_ship_auto_attacks_hostile_but_not_the_followed_friend` | g2 合成场景（钉 `Follow` 叶 + 摆三方） | 打敌人 `长城→华盛顿`、**指向友舰的攻击 0 条**、`Follow{赤霄}` 叶 4 回合没降级 |

⇒ `shots.rs` 与 `fleet.rs` 各自搬空并删文件（`mod shots;` / `mod fleet;` 也摘了）。

**第 7 批残渣（23 条，全部落 §4，逐条写明理由）**

| 族 | 条数 | 为什么只能留 Rust |
| --- | --- | --- |
| `ideology` | 4 | 要**往世界里注入事件**（`--call` 是**纯函数**契约，加「可变调用」是设计改动）；`military_signal…` 直调 `military_deltas(事件表)` |
| `site_supply` | 3 | 要**可写 `depots`**（复合键 map `"中国\|水星"`；`edit()` 只认带身份键的**行表**） |
| `haul` | 3 | `step_production`/`haul_step` 的**隔离**调用；货值守恒还差**穿带丢货**那个 sink（实测 399 回合里 7 回合对不上） |
| `domestic_market` / `market` | 2 + 2 | 另一个会话正在飞的**内部账/定价单元测** |
| `governance` | 2 | `step_governance` 单元测：**覆盖率 0 的欠费支路**在完整回合里够不到（产出先到账） |
| `spending` | 2 | `step_upkeep` / `build_lines` 单元测（同上：欠费恒为 0） |
| ~~`knowledge`~~ | ~~1~~ | **已搬**（第十一段：MOND 逐回合定律） |
| `trade` | 1 | `haul_steps` 的**键集**与**存活**两半有回合末伪影（表里多出的舰是回合末被改派的） |
| `blueprints` | 1 | 要「掏空库存之后**仍有新舰下水**」——没资源就不下水 ⇒ 判据必然空转 |
| `war_scar` | 1 | **手工世界**（往 `notables` 塞历史后逐年龄问内部函数） |
| `mond` | 1 | 只打印的 `#[ignore]` 探针 |

**第 7 批第八段（sim 23 → 20，收口）**

| 原件 | 判据 | 实测 |
| --- | --- | --- |
| `site_supply::the_capital_body_spends_the_faction_pool` | g2 合成场景（城改「还差一半没建」+ 只拨池子） | 首都池满 ⇒ **r1 就长**（60→69.12）；池空 ⇒ r1 不长 |
| `site_supply::only_the_local_depot_can_fund_an_offsite_city` | 同上（非首都那一臂） | 非首都池满 5000 ⇒ **r1 不变**；自然跑 40 回合 ⇒ 20.89→**41.77** |
| `ideology::military_signal_uses_the_milestones_and_is_branch_agnostic` | 新 `--call military_deltas {events}` | 六个用例逐条：互杀各 0 / 单方面 ±1 / 欠费只扣失主 / 拆城 ±1 而复垦者不计分 / 易主与叛乱同分 / 新建城不计分 |

**收口时剩下的 20 条**（前表 23 条里再去掉上面三条；逐条理由见上表）。

⚠ 再记两条**判据本身出错**的经验：
1. `site_supply::an_export_haul_never_loads_the_site_reserve` 试了两版读面判据**都被打回**：
   「装完残余 ≥ 保留量」错在 r11 金星（见底 0 < 保留量 6——引擎的规则是「只能装走**超出**保留量的
   那部分」）；「要么留够、要么见底」错在 r25 水星（存量 6.075 < 保留量 6.080——**城自己也在从
   货栈花钱**）。真正的判据要**装货前那一刻**的存量与保留量 ⇒ 读面没有。
2. **C 盘被测试投影撑满**（`ERR_INDEX: 磁盘空间不足 os error 112`）：`target/test-fixtures` 攒到
   **18 G**（g3 的 7×1000 回合投影是大头）。清掉腾出 31 G；这些是**可再生产物**，重跑会重建。

**第 7 批第九段（收口后继续，sim 20 → 19）**

| 原件 | 判据 | 实测 |
| --- | --- | --- |
| `ideology::ideology_military_win_drives_toward_militarism` | g2 **造一场真仗**（最偏和平端那家的舰装 `railgun`、敌舰船体压到 1、两家关系 `-35`） | 打仗臂 `和平↔军国` **−0.60 → −0.54**（Δ=0.06，我方击杀 1 次）；对照臂 **−0.60 → −0.57**（Δ=0.03，0 击杀）——正是 `0.05×(0.5−(−0.6))` 与 `0.05×(0−(−0.6))` |

新加 `meta.ideology`（四轴的换算系数与 `drift_rate`），g3 加两条长局不变量（+3 条判据）：
**四轴恒在 `[-1,1]`**、**每回合位移 ≤ `drift_rate × 2`**（63,000 个势力·回合 × 4 轴全过）。

✅ **思潮经济轴的逐回合定律搬成了**（第十一段）：「目标稳定 ⇒ 逐字相等、目标变过 ⇒ 落进夹逼区间」
——**63,000 个势力·回合 0 违规**。当初判它"不行"是我的索引配错了一格（见「订正」）。
* 同族的 **`market` / `domestic_market` 四条仍是 §4**：价格序列与「逐城 spent」都不在读面上，
  且两条都要**手工世界**（清空全部货栈 / 造两座一模一样的城）。

## ⚠ 订正（第十一段）：一个"索引配错一格"害我写了三条错误结论

**症状**：我拿"第 `r` 回合末的轴/掌握度"去配"第 `r` 回合的过程量"，得出"读面复算的目标与
step 当场用的那个对不上"，于是把 `knowledge` 与 `ideology` 的那些逐回合定律判成"读面证不了"，
还写进了文档与两条提交信息。

**真相**：轴的 `r → r+1` 这一步，用的是**第 `r+1` 行**的过程量（那一行才是跑出 `轴(r+1)` 的
那一步看到的东西）。代码也支持：`step_ideology`（`sim/mod.rs:186`）之后**没有任何 step 再写
那个 sink**（`mod.rs:200` 才折进 post），而 `view.production` 就是 `sink.faction_production`
（`sim/metrics.rs:69`）⇒ **过程量那几列根本没有相位差**。

**订正后的数**（同一个投影，只改了配对）：

| 定律 | 配错（当时） | 配对（现在） |
| --- | --- | --- |
| 思潮经济轴：`人民↔精英(r+1) = 朝当回合目标松弛` | 9.4% 违规 | **63,000 个势力·回合 0 违规**（7 seed × 1000） |
| MOND：`掌握度(r+1) = 棘轮 / 学满线性 / 朝目标松弛` | 0.0267 最大偏差 | 目标稳定 **61,578 个逐字相等**；目标中途变过 **1,422 个落进夹逼区间**，0 违规 |

**唯一真存在的相位效应**：回合**中途**才定、且**不被 sink 记录**的量——MOND 那 1,422 个
"船在回合中途进出带内"就是它（回合末的目标与走那一刻的差一档）。判据的写法是
**目标稳定 ⇒ 逐字相等，目标变动 ⇒ 夹逼**。

**教训**：写逐回合律之前先问一句——**"我这一行记的是哪一步看到的东西？"** 配错一格会伪装成
"引擎有时序问题"，而它看起来完全可信。

**第 7 批第十段（sim 19 → 17：不用改契约的两条）**

| 原件 | 判据 | 实测 |
| --- | --- | --- |
| `war_scar::war_scar_floor_shape_decays_over_its_window` | 新 `--call war_scar_floor {a,b}` + 四个年龄的档 | `age 0 = −30.0`（配置初值）→ 单调抬高 → 窗口内 `<0` → 出窗口 `null`；**顺序无关**；**没打过仗的一对没有疤** ⇒ `war_scar.rs` 删除 |
| `ideology::ideology_similarity_shifts_diplomatic_affinity_directionally` | g2 合成场景（两臂只差思潮） | 同极相似度 1 ⇒ 关系爬到 **+16.6**；对极相似度 0 ⇒ 掉到 **−53.9**（间距 70.5） |

**三条「试过但打回」**（这一轮的教训，值得复读）：
1. **（已订正）经济净值 → `人民↔精英`**：当时记的三版失败**全是我的索引错**（用源回合的过程量配
   目的回合的轴）。改对之后 **63,000 个势力·回合逐字相等**，第十一段已搬。⚠ 教训：
   **"窗口均值/末期值"这两种偷懒的汇总本来就不该用**（净值会在窗口里翻号），要么用逐回合定律，
   要么别下结论。
2. **`spending::upkeep_shortfall…` 的构造本身是判据的一部分**：它要「**恰好半价**」，
   读面复现不了——我试过「把舰改成重舰 ⇒ 维护费结构性超过产出」，结果要么 `unpaid = 0`
   （产出先到账）、要么 `unpaid = upkeep`（`rust = 1.0`，舰队当场锈光）**没有中间态**。
3. **`market` / `domestic_market` 四条确认 §4**：价格序列与「逐城 spent」都不在读面上。

**第 7 批第十一段（sim 17 → 14）**：见上面的 **⚠ 订正** 一节 + 下表

| 原件 | 判据 | 实测 |
| --- | --- | --- |
| `knowledge::control_rusts_back_when_the_fleet_leaves` | g3 **MOND 逐回合定律** | 目标稳定 **61,578 个势力·回合逐字相等**；目标中途变过 1,422 个落进夹逼区间 ⇒ `knowledge.rs` 整族删除 |
| `ideology::ideology_economy_bad_…` | g3 **思潮经济轴逐回合定律** | **63,000 个势力·回合 0 违规** ⇒ `ideology.rs` 整族删除 |
| `spending::build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling` | g2 合成场景（三条臂：库存 × 预算） | 两头足 ⇒ `increment = rate = 11.92`；钱 0 ⇒ **0**；库存 0 ⇒ **≈0** |

**剩下 14 条的下一步**：`combat` 4 / `governance` 3 / `haul` 5 / `ideology` 7 / `knowledge` 6 /
`mond` 4 / `shots` 3 / `trade` 3 / `spending` 2 / `domestic_market` 2 / `market` 2 / `war_scar` 1 /
`site_supply` 3 / `blueprints` 6 / `fleet` 3。已知分两类：
* **`depots` 不可写**：`edit()` 要「带身份键的行表」，而 `depots` 是**复合键的 map**（`"中国|水星"`）
  ⇒ `site_supply` 那 2 条 A/B 造不出来（要么扩引擎的 `--nouns` 声明「map 型属性的键部件」，要么留 §4）。
* **要挂纯函数 `--call`**：`resolve_loadout`/`choose_loadout`（`blueprints` 1）、`route_depth`（`mond` 1）、
  `ideology_similarity`（`ideology` 2）——第 5 批那套现成的。
* **还能用同一配方做的**：`haul::a_hired_delivery_splits_the_cargo_between_carrier_and_shipper`
  （`contract_delivered` 事件带 `amount`/`cut`，抽成比落在**配置区间** `[share, share_max]`；
  ⚠ **池子去向那半**事件里没有，要搬就得把分账两头也进事件/读面）；
  `governance::player_welfare_budget…`（要能读「按库存价值比例支付」，现在产出/贸易同时在动库存）；
  `combat::fleet_air_defense_covers_nearby_missile_targets`（要构造编队）。
* **要新读面列**：`factions.贸易禁运`（trade 1）、`ships`/`faction_process` 的「在场强度」
  （knowledge 3，现在是**带内舰数**，而机制用的是 `1 + 深度 × depth_weight`）、
  `haul_steps` 的 from/to（haul 1）。再加一个 `--call mond_drift` / `nav_roll` 能解 mond 2。
* **§4 明说不搬的残渣**：`domestic_market` 2 + `market` 2（另一个会话正在飞的内部账/定价单元测）、
  `war_scar` 1（手工世界）、`shots` 1（要构造 duel）、`site_supply` 3（要可写货栈 / `haul_load`）、
  `combat` 2（要构造编队/屏护）、`spending` 1、`blueprints` 1、`ideology` 4、`haul` 2。

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

## 「直接测分布，而不是采样」做到哪一步了（2026-10 第 7 批末）

用户裁决：概率分布的测试**不该抽样**，该测**分布本身**。已落地的第一步：

* `RoundInputs` 新增 **`role_distribution`**（该类型的纪律原话：「只增不改语义：新发现一类输入
  就往对应小节里加」），由 `assign_roles` 填——**只有它认早退档**，所以读的人不必再抄一遍分支规则。
  记 `quota` / `held` / `gap_join` / `gap_leave` / `rotation` / `fixed` / `actual` / `exogenous`。

**它顺手证明了抽样判据有多弱**：实测 `r1 中国 observe 配额 1.51` 而**在册 2**（硬承诺 + 在任
盖过配额）——那条 400 回合均值 ±0.5 的判据一直在**吸收一个 0.49 的系统性偏差**（1.51 → 2.00
刚好卡进容差）。而单回合头数只是**一次样本**（实测 `Σp = 0.55` 也能掷出 2 艘）⇒ 只有分布是不变量。

**⚠ 逐舰机会值还没记全，卡在「口径」而不是公式**：`rolls` 只记**落中那一支**的 `p`
（观测没命中的舰**仍然掷过**观测的骰子）。把 `Σp` 与 `缺口 + 轮换×在册` 对账**对不上**：

```
r2 中国 freight:  Σp_leave = 0.0（该舰闸门 threshold = 0.0），而同一艘舰的 flow = 0.05
```

`others` / `tickets` 只算**掷得动的舰**，而两支的**头数**算的是**全部在册**——舱里有货的两艘
运输舰**计入头数、却不在分母里** ⇒ **只记 `p` 一定对不上**。

**下一轮的确切做法**：`RoleOdds` 里带上 `(p, tickets, others, flow)`，且**落空那一支的 `p` 也要
带出来**（观测没命中时那枚骰子的机会值现在被丢掉了）。那时 `Σp == flow`（未被 `min(1,·)` 截断时）
是逐字恒等式，两条 `autocontrol` 抽样循环（400 / 200 回合）就能删掉，门再省 ~4 s。
