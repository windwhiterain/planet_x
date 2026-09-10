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
