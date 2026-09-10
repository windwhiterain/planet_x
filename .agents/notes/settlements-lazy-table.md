# 定居点也迁到「名字即唯一 key」+ 投影的 `settlements` 懒表

> 状态 `[x]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §20

> 延续 `name-as-unique-key.md` 的裁决：`Settlement` 有真实中文名却仍按下标引用（`City.settlement: usize`）。把最后的数字 id 也去掉，并把「每个天体的定居点」做成 projection 的懒表。

- **`City.settlement: usize` → `String`（定居点名）**：`Settlement` 与 `City` 1:1，但殖民地城的城名是 `{定居点名}-殖民城`（≠ 定居点名），所以必须显式按**名字**引用，不能从城名推导。
- **accessor 按名字**：`Body::settlement(&self, name)` 按名查找；`state.city_settlement(cname)`/`state.body_settlement(bname, sname)` 全用 `&str` 名。
- **占用判断改为名字集**：`find_vacant_settlement`/`has_blank_site`/colonize 的 `occupied: BTreeSet<String>`（定居点名）；空白位 = 名字不在 occupied 里的定居点；建城 `settlement = settlement.name`。
- **`world.rs city()` 去掉 `site: usize`**：直接从传入的 `&Settlement` 取 `settlement.name`（22 处调用同步去参）。
- **projection：新增 `settlements` 懒表**（`idx/settlements.jsonl`，`round:false` 全局主表）：每 body 的每个定居点一行（`settlement_id`=名、`body_id`、`index`、`name`、面积/生态容量/建设修正/资源矿藏）；主流新增 `settlement_ids` 数组；`cities` 表新增 `settlement`（=定居点名）列。Python kit `q.join('settlements')` 直接可用。
- **验证**：`cargo build --bins` 绿；`cargo test --lib` 42 passed（含 `settlements_and_cities_are_one_to_one`、`projection_writes_lean_main_and_indexed_tables`）；`cargo test --test longhorizon` 5 passed/4 ignored；smoke `--seed 7 --round 2 --index` → `idx/settlements.jsonl` 正常、`planet_xq` 可 join。
- **注意**：`.ron` state 的 `City.settlement`/`Body.settlements` 结构改变，旧档不兼容（符合「不考虑向前兼容」）。
