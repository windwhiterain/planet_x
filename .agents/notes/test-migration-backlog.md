# 测试全搬的施工图：**判据缺什么数据，就往序列化里装什么**

> 状态：**第 1–5 批已落地**（2026-10；分支 `feature/test-migrate-rest`，worktree `C:/resource/planet_x-decoupled`，
> 每批都已合进 `main`）。**第 6 批待做**（合成场景——地基缺口与清单见 §5.6）。§4 的「故意不搬」清单已写死（第 7 批）。
> 计数：**Python 146**（g1 43 / g2 62 / g3 26 / g4 15）；**Rust 200**（+31 探针 ignored）。
> 起点口径（本主题开工时）：Python 75 / Rust 222 单测 + 8 集成。（2026-10 用户裁决：*「总之目标是全搬，有需要的数据没序列化就把他装进序列化里」*）
> ｜ 索引：[notes.md](../notes.md) ｜ 上层：[test-decoupled-suite.md](test-decoupled-suite.md)

## §0 现在的账

| | 数据级（Python） | Rust 侧 |
| --- | --- | --- |
| 起始（本主题开工时） | 75 | 222 单测 + 8 集成 |
| 施工图写下时 | 120（g1 33 / g2 46 / g3 26 / g4 14） | 206（+31 探针） |
| **现在（第 1–5 批后）** | **146**（g1 43 / g2 62 / g3 26 / g4 15） | **200**（+31 探针 `#[ignore]`） |

搬走的族（截至第 5 批）：投影审计 6、governance/spending 9、combat 8、blueprints 9、合成场景 2、trade 3、cargo 1、off_capital 1、派单 1、纯函数 5。

### §0.1 第 1–5 批落地记录（每批 §6.2 三条全绿，且都已合 `main`）

| 批 | 装了什么 | 搬走 | Python | 合 `main` |
| --- | --- | --- | --- | --- |
| 1 `trade` | 零新增（`market_trades` + 主流 `view.market_settled` 已够） | trade 3 条 → g2 `trade_checks`（6 条） | g2 46→52 | `9f7f197` |
| 2 舰的货舱 | `ships` 加 **`载货`** + **`cargo_capacity`**（派生列） | `haul::cargo_capacity…` 数据级一半 → g2 `cargo_checks`（3 条） | 52→55 | `a5b68ec` |
| 3 `depots` | 新派生表 `depots` | `haul::off_capital_production…` 读面一半 → g2 `depot_checks`（5 条） | 55→60 | `7846c04` |
| 4 派单抽签 | 零新增（用 `round_inputs.rolls`，**否决** `haul_lanes`——§5.4） | `freight::route_lottery…` → g2 `dispatch_checks`（2 条） | 60→62 | `83521f6` |
| 5 `--call <fn>` | 新 CLI 读面（`src/main.rs`） | 纯函数 5 条 → g1 `call_functions`（10 条） | g1 33→43 | `9e1a566` |

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

## §3 类 C：**合成场景**——**第 6 批待做**（地基缺口/清单见 §5.6）

`scenario()` API 已经能用（`_harness.gen/edit/state_dump/scenario`），凡是「改一个字段 ⇒ 看后果」
的都能搬。剩下的都是**要同时拨控制叶**的（`--apply` 与 JSON 档里的 `control` 段都能改，后者更适合
Python）。清单见 §5.6：`a_player_pinned_design…`、`a_dangling_pointer_is_left_dangling`、
`only_unreferenced_selfmade_designs_are_reaped`、`spending` 的两条预算 A/B、`blueprint` 的角色/姿态。

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
6. **[ ] 合成场景收尾** ⇒ 搬类 C 剩下的（地基缺口与逐条清单见 §5.6）。
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

### §5.6 第 6 批（合成场景）的地基缺口与清单

**缺口**：`_harness.scenario()` 只能捏**列表型表**；类 C 剩下的要**拨控制叶**（`state.control`
是字典型）。两条路二选一（都小）：

* **A（建议）**：给 `_harness` 加 `scenario_apply(name, seed, rounds, diffs)`：`gen` → 逐份
  `--apply diff.json --round 0 --quiet --save cur` → `run_into(out, …, extra=("--start", cur))`。
  `--apply` 的 diff 形状**已定义好、g4 在测**（`{"control":[{…}], "scope":…}`），不必在 Python 里
  碰 `control` 的内部形状；坏处是多几次 CLI 调用（零点几秒）。
* **B**：把 `edit()` 扩成「字典型表做深合并」。

**清单**（Rust 原件删掉、模块头留对账表）：

| 用例（`autocontrol/blueprints.rs` / `sim/spending.rs` / `autocontrol/shipbuilding.rs`） | 要拨的叶 | 数据级断言（读面） |
| --- | --- | --- |
| `a_player_pinned_design_and_its_yard_are_left_alone` | `blueprints[图].mode=Player` + 建造区指针改指它 | `blueprints` 表：mode=Player、选装不变；`cities.建筑[].blueprint` 仍指它 |
| `a_dangling_pointer_is_left_dangling` | 建造区指针改成不存在的图（`cities` 整表替换即可，**不需要**控制叶——最简单，先搬这条） | 指针原样保留（AI 不替你修） |
| `only_unreferenced_selfmade_designs_are_reaped` | 造 `mode:Inherit` 的自建图 + 玩家图 + 玩家钉的 AI 图 | 自建的没了、玩家图/钉住的还在；`decisions` 里有 `reaped` |
| `spending` 的预算 A/B（`build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling` 等） | `investment_budget`/`construction_budget` 拨低/拨高 | `city_process.build`：`increment < rate`（钱是瓶颈）/ `increment ≈ rate`（产能封顶） |
| `blueprint` 的角色/姿态（`shipbuilding.rs::player_pinned_blueprint_is_not_retooled` / `an_auto_blueprint_is_retooled_as_a_blueprint`） | 图上的 `role`/`kiting`（**等字段命名批 B/C 收口后**再搬） | `blueprints`/`ships` 的有效值 |

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
target/release/planet_x.exe --seed 42 --round 240 --digest 20   # 取 sha256 == 748B4AA6…9603
# ② 数据级（116→ 每加判据都会涨；`-j 7` 并行）
uv run --project play/planet_xq python play/tests/run.py all -j 7
# ③ Rust 侧（搬一条就该少一条）
cargo nextest run -P full
```

> ⚠ **digest 基线的历史**（施工图早期写的 `BB2EEB2B…2000` 已作废）：
> `BB2EEB2B…2000`（`feature/web-control-spec` 合并**前**）→ `C928C3F1…06A9`（合并后）→
> **`748B4AA6…9603`**（`feature/field-names` 批 A：11 个实体结构体 83 个字段加 serde 中文名 +
> 字段顺序；见 `.agents/notes/field-naming.md` §7.5）。**当前口径 = `748B4AA6…9603`**
> （12 行、只取 `^{` 行、LF 连接、UTF-8 无 BOM）。`feature/domestic-market` 合并进来后实测仍是它
> （配置里那一段是注释掉的）。

### §6.3 合成场景的 API（`play/tests/_harness.py`，已就绪）

```python
h = Harness("release")
w = h.gen(Path("….json"), seed=42)                  # 造世界（JSON 档 ⇒ Python 能改）
st = h.state_dump(w)                                 # 读状态（ships/cities/factions/control…）
proj = h.scenario("名字", 42, 3, patch={"ships": {"长城": {"hull": 3.0}}})   # 造→捏→推进→投影
#   patch 的形状 = 读面同名同形；打错的字段/点不到的名字会进 h.warnings（**不静默**）
#   整表替换也行（例：改一个建筑 ⇒ 把新的 buildings 列表整个塞回去）
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

### §6.5 现在的状态（**2026-10 第 1–5 批后**）

* 分支：`feature/test-migrate-rest`（worktree `C:/resource/planet_x-decoupled`），已同步到 `main`
  （第 5 批合并 `9e1a566`）。⚠ `main` 是**多会话共用**的、会随时前进（合并窗口里就落了
  `feature/field-names` 与 `feature/domestic-market`）⇒ **每批动手前先 `git merge main` 同步**，
  做完三条验证再合回 `main`（字段中文名那次就是这么撞出来的：第 1 批合早了，第 2 批先补合 `main`
  才接上 `载货`）。
* 计数：Python **146**（g1 43 / g2 62 / g3 26 / g4 15）；Rust **200** + 31 探针 `#[ignore]`。
* 当前 digest 基线：**`748B4AA66FE169E8F316169D61CB5239057D0F3515CF59A50C629E76BF489603`**（§6.2）。
* 两道门都是绿的；`target/test-fixtures/` 有自动清理（`run.py` 默认 `sweep_stale`，`--no-sweep` 可关）。
* 判据写在 `play/tests/g*.py` 的 `run()` 里（数据取自 `extract()` 的摘要 ⇒ **改断言不重读投影**）。
* **下一个会话从 §5.6 第 6 批开始**：先补 `_harness` 的控制叶地基（`--apply`-in-scenario 或字典型深合并），
  再按清单逐条搬（先搬 `a_dangling_pointer_is_left_dangling`，它不需要控制叶）；第 7 批（§4 清单）已写死。
