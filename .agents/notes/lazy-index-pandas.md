# Lazy 间接索引 + agent 可读 schema + Python/pandas（uv）分析层

> 状态 `[x]`（feature/lazy-index 原型） ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §17

> 方向：未来 object 字段会带巨量数据，不能全量内联；要把重型字段**间接索引**（按 id 拆到表），
> agent 必须按 id join。分析层不 embed、用 Python/pandas（LLM 熟），Python 项目管理用 **uv**。

**已落地一个最小闭环（`src/projection.rs` + `play/planet_xq`）**：
- `[x]` **`planet_x --seed S --round N --index DIR`**（`src/projection.rs`）：把 N+1 回合投影成——
  - `DIR/main.jsonl`：**lean 主流**，每回合一行 `{round,time_month,events,chronicle,metrics,
    ship_ids[],city_ids[],body_ids[]}`（重型实体不内联，只带 id）。
  - `DIR/idx/{ships,cities}.jsonl`：**per-round 表** `(round, key_id, ...)` 完整对象。
  - `DIR/idx/bodies.jsonl`：**全局主表**（天体 name/轨道/定居点，几乎不变），一次性。
  - `DIR/schema.json`：**agent 可读的投影契约**——声明 `eager`（内联字段）vs `lazy`（索引字段），
    每个 lazy 字段的 `table/key/id_col/round`、`columns` 类型、`description`、`read_order` 阅读顺序。
- `[x]` **lazy 表由 `LAZY` 常量声明式驱动**：加一个重型字段，Emit + schema 自动跟上，零特判。
- `[x]` **Python kit `play/planet_xq`（uv 管理）**：`pyproject.toml` + `planet_xq` 包。读
  `schema.json` 认清 eager/lazy，`load()` 出 `main.jsonl` + 各索引表 DataFrame；暴露
  `q.facts`/`q.ships(round)`/`q.cities(round)`/`q.bodies()`/`q.ids(field,round)`/
  `q.join(field,round)`（explode 主流 id-数组 + 按 id merge；per-round 表按 `(round,key)` join，
  全局表按 `key` join）。`demo.py` 演示 agent 读 schema → 按 id join。
- `[x]` **uv 布局**：`play/planet_xq/` 是独立 uv 项目；`.gitignore` 放行 `play/planet_xq`、
  忽略其 `.venv`/`__pycache__`；`uv.lock` 入库。`uv sync` + `uv run planet-xq <dir>` / `uv run python demo.py <dir>`。
- `[x]` **config 进投影（规则×事实同会话查询）**：`--index` 投影额外写一份 `meta.json`
  （复用 `--meta` 的同一渲染器 `agent::meta_value`），`schema.json` 加 `meta:"meta.json"`
  并更新 `read_order`。`planet_xq` 自动读它：`q.meta`（原始 dict）+ `q.spec(section)` +
  `q.ships_spec()`/`q.buildings_spec()`/`q.components_spec()`/`q.structures_spec()`/
  `q.resource_value()`（各规则表平铺成 DataFrame，index=规格名；`build_cost`/`cost` 这类
  嵌套 dict 保留为 object 单元格，可用 `.meta` 取原始）。于是 agent 在**同一个 pandas 会话**
  里能 `q.ships_spec().merge(q.join('ships',round=r), ...)` 算「舰队维护」「造得起几艘」，
  不用再把 `--meta` 当游离 JSON 手读。`demo.py` 增加「规则×事实」示例（round 6 舰队维护/月）。
- `[x]` **确定性 + 守卫**：`projection_writes_lean_main_and_indexed_tables`（main 每行不内联
  ships/cities/bodies、只带 id；schema 声明 eager/lazy；各索引表写出且带 key 列）、
  `projection_is_deterministic`（同 seed → main.jsonl 逐字节一致）。
- 验证：`cargo build` 无 warning；`cargo test --lib` 41 passed（+2 投影守卫）；longhorizon
  5 passed / 4 ignored。端到端（`--seed 7 --round 12`）：main.jsonl 13 行 lean（8 列），
  `ships(round=6)`/`join('ships',round=6)` 出 8 艘舰、`cities(round=6)` 22 行、`bodies()` 18 行。

**候选（留待后续）**：
- `[ ]` **parquet**：巨量时换列式存储（pandas 原生读 `read_parquet`），Rust 侧加 arrow/parquet
  依赖；当前先 JSON Lines 起步。
- `[ ]` **更多 lazy 字段**：真正的重型字段如 `ships[].components/component_hp`、`cities[].buildings`
  （可再拆一层 building 子表）、`events`/`chronicle` 到巨量时也标 lazy。
- `[x]` **统计函数库**（Python 侧，`planet_xq` 内）——先落了**取数 + 窗口平均**：`metric_series(path)`
  （逐月序列，index=round）、`window_avg(path,size,agg)`（按 size 回合/窗口聚合，默认 mean）、
  `yearly_avg(path)`（12 月=1 年）、`decadal_avg(path)`（120 月=10 年）。`path` 为点分路径，
  可指世界量（`metrics.population`）或势力量（`metrics.factions.中国.production_value`）。
  仍开：`rolling_mean/max`、`histogram`、`hegemon_timeline`、`leader_rotation`、`war_durations`、
  `gini(stockpile)`——把「方便做统计」做成库而非让 agent 每次手写 pandas。
- `[ ]` **eager 单对象模式**（保留短跑 `--round`/`--traj` 全量快照）与索引模式并存，同一份
  schema 投影，两路都保持。

---

- **区域性霸权**：治理 + 本土防御 + MOND + 合纵连横让世界有了地理与外交结构、也**不再统一**
  （单一势力城占峰值 < 0.85，制衡联盟会发生），但**仍允许某势力在长局里长期占 ~55% 城镇
  份额**（制裁对自给自足的富矿大国收效有限、联盟缺协同牙齿）。多极还没真正达成，这是
  下一步（第 2 节）的主攻方向——重点放在「让抱团真的咬下去」与「超载/过度扩张更咬人」。
- **seed 7 @ 240 回合实盘观测（本次 jq 查询，`play/traj_seed7.jsonl`）**：这是**轮换存在但末态
  仍单极**的最直观证据。开场 22 城 9 势力大致均衡 → 第 5 回合爆发首战/教团之战、第 18 回合首城被
  夷平、第 60 回合剧情弧收束于「行星X 现身」；随后出现真实霸权轮换：**中国(0.35) → 星系矿业
  (0.59) → 中国/俄罗斯(0.50/0.36) → 中国(0.65)**（`world_is_multipolar` 的「最强≥2 个轮换」
  能满足，峰值 0.65<0.85）。但**240 回合末又坍缩成 1 霸权 + 8 个 1 城旁观者 + 资源高度集中**：
  中国 9/17 城(53%)、17/24 舰(71%)、564 船体，库存 硅109/铁66/水冰60/碳46；其余 8 势力各剩 1 城、
  资源几乎全空（美国/欧盟/俄罗斯/星系矿业全 0），而**联合国坐拥 644.9 铁却只有 1 城 2 舰**、
  **深空运输联盟囤着稀缺的 钍5.86/铂1.24 也不造舰**——「区域霸权 + 永久旁观者 + 财富不转化为
  力量」在 seed 7 被完整复现。若要收紧「任意一方城占比长期均值 < 0.5」，先修「重建缺口」并让
  「超载/过度扩张」按「城数×每城人口」更快触发，同时避免制裁只压弱国、放过大亨。
  另：轮换本身在 seed 7 成立，说明**目前丢的不是「轮换」而是「末态均衡」**，宜作为 `balance-of-power.md` 的一条
  独立可量化守卫（断言 240 回合末最强势力城占比 ≤ 0.5）。
- **长局性能**：`tests/longhorizon.rs` 的 `diagnose_long_horizon`（3 种子 × 3000 回合）较慢
  （~1 分钟），已 `#[ignore]` 化；默认 `cargo test` 只跑快守卫（~13s），别把慢测得放回默认。
- **确定性**：新增机制全部为确定性（无 RNG 或仅用种子 RNG）；`same_seed_reproduces_identically`
  守卫可复现性。任何新机制不要引入未播种的随机性。
