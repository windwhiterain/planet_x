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
