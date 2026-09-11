# 测试全搬的施工图：**判据缺什么数据，就往序列化里装什么**

> 状态：`[ ]` 进行中（2026-10 用户裁决：*「总之目标是全搬，有需要的数据没序列化就把他装进序列化里」*）
> ｜ 索引：[notes.md](../notes.md) ｜ 上层：[test-decoupled-suite.md](test-decoupled-suite.md)

## §0 现在的账

| | 数据级（Python） | Rust 侧 |
| --- | --- | --- |
| 起始（本主题开工时） | 75 | 222 单测 + 8 集成 |
| 现在 | **120**（g1 33 / g2 46 / g3 26 / g4 14） | **208**（+31 探针 ignored） |

搬走的族：投影审计 6、governance/spending 9、combat 8、blueprints 9、合成场景 2。
**剩下的 208 条按「缺什么」分四类**——前三类的解法都是**把数据装进序列化**（照 B1–B5 的规矩：
过程量与观测同处一行、纯追加、**行为中性 ⇒ digest 逐字不变**、「这一步没跑」给**中性缺省**）。

## §1 类 A：数据没序列化（装进去就能搬）

| 缺的量 | 谁要它 | 装哪儿（建议） |
| --- | --- | --- |
| **舰的货舱**：`cargo`（按资源）+ `cargo_capacity` = 舰级舱容 × `hull / hull_max` | `haul::cargo_capacity_is_class_capacity_times_hull_fraction`、freight 的多条腿用例 | `ships` 表加两列（**状态里已经有 `Ship.cargo`**，投影没发） |
| **产地货栈的逐资源存量**（现在只有 `cities[].depot_value` 一个合计） | `haul::off_capital_production_lands_in_the_depot_not_the_pool`（「金星货栈里有碳、池子里的碳没动」） | 新派生表 `depots`（`round, faction_id, body_id, resource, amount`）——与 `market_trades` 同形 |
| **集货腿（lane）**：出口腿/进口腿、`from/to`、资源、量、派了哪条舰、保留量、需求 | `autocontrol::freight` 全族（~17 条）里「按积压占比抽签派单」那半边 | 新派生表 `haul_lanes`（`round, faction_id, kind, from_body, to_body, resource, amount, ship_id, reserve`）；`haul_steps`（已有）记的是**在途**，缺的是**派单决策** |
| **成交的价格分解逐项**（若 `market_trades` 没发全 `dist_au`/`depth`/`freight_rate`/`mond_extra`） | `trade::trade_price_terms_are_self_consistent` | `market_trades` 补列（先查 `schema.json`：可能已经够了） |

## §2 类 B：**纯函数**（要一条「调用」路，不是数据路）

`haul_split_is_max_min_fair`、`hit_factor`、`home_defense_mult`、`ship_panel` 的公式、
`spawn_clears_stale_plan` 之类：它们的判据是「给这些数 ⇒ 得那个数」，**没有世界可看**。

两条路（建议 A，因为它同时是**给 agent 的读面**）：

* **A. `planet_x --call <fn> --args <json>`**（一次调用、一行 JSON 回执）：把引擎里**已经写好、
  已经在用**的纯函数直接暴露出来，Python 拿真配置 + 真参数调它。好处：不是「抄公式」而是
  **调同一份实现**；坏处：多一个读面要维护（文档 + 参数校验 + 报错回执）。
* B. 留在 Rust（现状）。⇒ 与「全搬」的目标冲突，除非用户改口。

## §3 类 C：**合成场景**（地基已在：`--save w.json` + Python 改档）

`scenario()` API 已经能用（`_harness.gen/edit/state_dump/scenario`），凡是「改一个字段 ⇒ 看后果」
的都能搬。剩下的都是**要同时拨控制叶**的（`--apply` 与 JSON 档里的 `control` 段都能改，后者更适合
Python）。清单：`a_player_pinned_design…`、`a_dangling_pointer_is_left_dangling`、
`only_unreferenced_selfmade_designs_are_reaped`、`spending` 的两条预算 A/B、`blueprint` 的角色/姿态。

## §4 类 D：**内部契约 / 错误路径**（这些**不该**搬）

`--apply` 的补丁面报错（`ERR_CONTROL_*`）、schema 迁移（`migrate` 的分支）、
`model::neutral` 的中性值表、序列化往返、`debug_assert` 的「漏了 kill_ship」这类**引擎内部不变量**。
它们判的不是世界，而是**引擎自己的接口**——留在 Rust 是对的（搬走只会让两边各说各话）。

## §5 执行顺序（按「装一次数据 ⇒ 能搬一族」排）

1. **[ ] `trade`**（先查 `market_trades` 够不够，大概率够）——3 条，最便宜。
2. **[ ] ships 加 `cargo` / `cargo_capacity`** ⇒ 搬 `haul::cargo_capacity…` + 给 freight 铺路。
3. **[ ] 新表 `depots`** ⇒ 搬 `haul::off_capital_production…`。
4. **[ ] 新表 `haul_lanes`** ⇒ 搬 `autocontrol::freight` 的派单半边（~8 条）。
5. **[ ] `--call <fn>`** ⇒ 搬类 B（~15 条纯函数）。
6. **[ ] 合成场景收尾** ⇒ 搬类 C 剩下的（~10 条）。
7. **[ ] 回头把 §4 的清单写死在本文件里**（哪些**故意不搬**，免得下一轮又逐条重新论证）。

每批的规矩（与 `test-decoupled-suite.md` 一致）：**装数据的那次提交必须证明行为中性**
（`--seed 42 --round 240 --digest 20` 的 sha256 不变），搬走的判据要带**防空转判据**，
并且**搬一条删一条**（Rust 原件不留在那儿等人重新困惑）。

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
target/release/planet_x.exe --seed 42 --round 240 --digest 20   # 取 sha256 == BB2EEB2B…2000
# ② 数据级（116→ 每加判据都会涨；`-j 7` 并行）
uv run --project play/planet_xq python play/tests/run.py all -j 7
# ③ Rust 侧（搬一条就该少一条）
cargo nextest run -P full
```

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

### §6.5 现在的状态（接手时的坐标）

* 分支：`feature/test-migrate-rest`（worktree `C:\resource\planet_x-decoupled`），已合到 main。
* 计数：Python **120**（g1 33 / g2 46 / g3 26 / g4 14）；Rust **206** + 31 探针。
* 两道门都是绿的；`target/test-fixtures/` 有自动清理（`run.py` 默认 `sweep_stale`，`--no-sweep` 可关）。
* 判据写在 `play/tests/g*.py` 的 `run()` 里（数据取自 `extract()` 的摘要 ⇒ **改断言不重读投影**）。
