# 实体身份「名字即唯一 key」（schema 唯一权威、无 shadow）

> 状态 `[x]`（branch `feature/name-as-unique-key`） ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §19

> 用户裁决：**schema 必须唯一权威、不能有 shadow 结构，因此统一用名字做唯一 key**；**只换有真实名字的实体**，没名字的（Building/Settlement）保留父实体内的下标注记。已在独立 worktree 完成并验证。

- **类型别名全改 `String`**：`FactionId`/`BodyId`/`CityId`/`ShipId` 都 = 实体唯一名。`Faction`/`Body`/`City`/`Ship` **删掉 `id` 字段**，`name` 即唯一身份；`Building`（`id`=u32 下标注记）未动；`Settlement` 随后也迁到名字 key（见 `settlements-lazy-table.md`）。
- **外键/关系/作用域全按名字**：`Ship.faction_id`/`City.faction_id/body_id` = 名字；`relations`/`ship_orders`/`ControlScope::{factions,bodies,cities}` 的 key 全为名字；`GameEvent` 里 ship/city/faction/body 引用全用名字；`ShipBehavior::TargetShip{ship: String}`（target=舰名，枚举改 `Clone` 去 `Copy`）。
- **accessor 按名字**：`state.faction/city/body/ship(&str)` 及 `*_mut`；`body_position/city_settlement/body_settlement` 同步 `&str`。
- **每势力名字库**：`config/game.ron` 新增 `name_pool`（各势力一坨、用词不重叠→天然全局唯一）；`ship_display_name(pool, seq)` 用「互素步长打乱轮转 + 世代数后缀」确定性取名，`State.ship_name_seq` per-faction 单调计数器保证舰名唯一且击毁后不复用。
- **config/story/剧情引用全改名字**：`mond.masters`、`Relations(a,b)`、`GrantShip(faction,body)`、`GrantResources(faction)`、`WarBetween(a,b)`、`FactionAtWar(faction)` 等全部用势力名/天体名。
- **验证**：`cargo build` ×2 绿；`cargo test --lib` 39 passed；`cargo test --test longhorizon` 5 passed/4 ignored（含 `same_seed_reproduces_identically`、`world_is_multipolar`、`no_nonfinite_over_long_run`）；smoke `--seed 7 --round 0` 舰名如 `长城/赤霄/北斗`(中国)、`华盛顿/总统/落基`(美国)，`Ship` 无 `id` 字段、`faction_id`/`relations` key 为名字。**注意**：`config/game.ron` 与 `.ron` checkpoint 的实体身份从数字 id 换成名字，旧档不再兼容（符合「不考虑向前兼容」）。
- `[ ]`（后续可做）**数字 id 彻底移除后的收敛**：`web.rs` 的 `StateView`/`MetaView` 仍属独立投影，可再复用同源；`agent` 未预计算派生字段（舰 panel/城 armor 等）仍靠 `--meta`/语义视图。

## §2 「为什么 `BuildingId` 全局唯一了，键里还要带城名」——**2026-10 裁决：保持**

一次读代码时问出来的：`InvestKey = BuildKey = (CityId, BuildingId)`，而
`BuildingId` 其实**已经全局唯一**（`sim/construction.rs`：`next_building_id = 全场 max(id) + 1`），
读面也把 `cities.buildings[].id` 露出来了 ⇒ 城市那一半看着冗余，「只留 id」值得问一句。

**结论：保持成对键（即上面第 5 行那条规矩）。** 理由不是「id 指不到」，而是**可读性**：
`21` 对人、对 UI、对 agent 的笔记都没有意义，「上海的建筑 21」才有意义；写面的叶本来就按
`keys: ["city","building"]` 寻址（`control/leaves.rs`），控制树也按「城 → 建筑」分组 ⇒
去掉城市那一半只是把一次 join 从引擎挪到每一个读的人手里。

**这份冗余的价钱（照实记下，省得下一个人重新发现）**：

| 代价 | 具体 |
| --- | --- |
| JSON 档要专门的键适配器 | `--save w.json` 把元组键写成 `"水星熔炉基地\|21"`，读回来时 `serde_json` 报 `invalid type: string …, expected a tuple of size 2` ⇒ 新增 `json::key2`（写 `名\|序号`、读按 `\|` 拆）。**`State.depots = (FactionId, BodyId)` 是真二元键**（同一天体上可有几个势力的货栈），所以这个适配器无论如何都要有 |
| 投影的控制表多一个槽 | `derived.control` 的 `key` = 城名 + `sub` = 建筑序号（其余 kind 的 `sub` 为 null） |
| 每个权重行背一个城名字符串 | 记忆与档都白背一点（可忽略，不入账） |

**顺手挖出的隐患：[x] 已修（`State.next_building_id` 单调计数器）**

id 分配器原来是 `max(全场) + 1`、**每回合重算** ⇒ **同一个 id 跨回合会被复用**：最高 id 的建筑
一被拆，下个新建筑就拿回那个号。此刻唯一性没问题（活着的建筑不会撞号，拆除时权重条目也一并删掉），
但**把 id 当长期引用**（跨回合的 UI 选中态、agent 笔记里的「建筑 21」、两份存档对比）会指错人。

**A/B 实测（seed 1 / 400 回合，就是为了证明它有牙）**：

| 分配器 | 结果 |
| --- | --- |
| 旧 `max + 1` | **3 个 id 被复用**：91/92/93 在 r104 出现、消失、又在 r128 前后落到**另一座城**（莫斯科）的建筑上 |
| 新单调计数器 | **0 例**（108 个 id 出现回合全部连成一段） |

实现：`State.next_building_id: BuildingId`（`#[serde(default)]`，`SCHEMA_VERSION` 22 → 23）。
`migrate` 里**幂等校准**（`== 0` ⇒ 补成 `场上 max + 1`，手改过的 JSON 档也走这条兜底），
`step_construction` / `step_military` 两处分配点改成 `max(计数器, 场上 max+1)` 取号、收尾写回
（只增不减）；世界生成时接上开局发出去的号。**验证**：`--digest` 逐字节不变（`BB2EEB2B…`）
⇒ 不动模拟；g2 新增守卫「**建筑 id 一旦消失就不再回来**」（3 seed × 400 回合 344 个 id 全连成
一段，其中 287 个真的在窗口内消失过 ⇒ 不空转）。
