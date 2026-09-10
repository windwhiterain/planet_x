# 稀疏历史 / 事件账本（sparse history ledger）

> 目标：让「**一个城市易主了，就近是什么事件导致的？被夷平然后被殖民，还是叛乱？**」和
> 「**一艘舰被击毁，是被哪艘舰击毁？**」都能在 Python 里一行 join 查出来。
>
> 状态：**Stage A 已落地**（branch `feature/sparse-history`）。Stage B/C 见文末。

---

## 1. 问题诊断：三个**独立**的根因

「轨迹对 state 变化的表达力低」不是一件事，是三件。按根本性排序：

| 优先级 | 根因 | 与 pandas/enum 的关系 | 证据（改动前） |
|---|---|---|---|
| **① 事实根本没被记录** | `Resurgence` 不带 `city`；难民夺城 `displace_city_for_refugee` **零事件**；`ShipDestroyed` 无凶手；欠缴锈蚀复用同一个「被击毁」；剧情赠舰零事件 | **无关** | seed 7 r24：`大红斑科学站` 被欧盟夷平 → 同回合「无国界科学组织」在**木星**重建（事件不带 city） → r31 倒戈欧盟。整条链只能靠 `resurgence.body == city.body_id` 反查 |
| **② 历史活不过 checkpoint** | `state.events.clear()`（每回合清空）；只有 `chronicle` 累计 | **无关** | 分段续局（agent-play.md 推荐的 `--start ckpt`）丢失机械史 |
| **③ enum 被直接序列化成表** | 13 个 variant 的字段被 serde 摊成一张 23 列并集 | **就是它** | 平均 null 率 **74.8%**；`faction` 这一个角色有 **7 种拼写**；`from`/`to` 一列两义（`city_defected` 是势力、`capital_relocated` 是天体） |

**关键反证**：把 ③ 修到完美也修不出 ①——**没写下来的事实，任何表形态都变不出来**。
所以真因是「**写入点散落 + 没有统一投影**」，enum 只是把症状放大了。

### 1.1 写入点审计（改动前）

`c.faction_id = …` / `c.razed = true` 散落在 5+ 处直接赋值，历史是「顺手记的副产品」，所以必漏：

| 变更点 | 事件 | 缺什么 |
|---|---|---|
| `step_governance` 倒戈 | `CityDefected{from,to}` ✓ | 忠诚度 |
| `step_governance` 叛乱夷平 | `Revolt{city,faction}` ✓ | 忠诚度 |
| `bombard_city` 夷平 | `CityRazed{fallen_to}` | **哪艘舰**只在同回合 `Siege` 里 |
| `colonize` 复垦 | `ColonyFounded{owner}` | 空白城的**旧主**（diaspora claim）丢了 |
| `step_resurgence` 复垦 | `Resurgence{faction,body,ship}` | **没有 city** |
| `step_resurgence` 难民夺城 | **无** | 一座活城静默易主 |
| `fire` 击毁 | `ShipDestroyed{ship,owner,class}` | **凶手**（只能靠 `Attack` 反推，集火不可判） |
| `step_upkeep` 锈蚀报废 | 复用同一个 `ShipDestroyed` | 战死 vs 报废不可分 |
| `grant_ship` 剧情赠舰 | **无** | 舰凭空出现 |
| 投影 `city_ids` | 过滤 `!c.razed`，而 `idx/cities.jsonl` 写全部 | `q.join('cities')` **静默丢掉被夷平的城**——偏偏那是历史最关心的实体 |

### 1.2 为什么不能只靠 dense-diff

dense 表相邻回合做 diff 能自动发现「变了」，但**因果盲**：说不出被谁夷平/被谁击毁，
而且**同回合 raze→recolonize 的 diff 是空的**（`step_military` 内舰序是 RNG 洗牌的，同回合发生是真可能的）。

> ∴ 正确解法不是二选一，而是 **sim 发事件（因果）+ dense diff 当对账（完备）**，互相补盲区。

---

## 2. 设计

### 2.1 稀疏的定义（唯一真正的编辑判据）

> **稀疏历史 = 一组显式声明的「连续性谓词」+ 每次跨越它的因果记录。**

不是「每回合采样连续量」。`hull 900→850` 不是事件；`hull→0`、`razed=false→true`、
`faction_id A→B`、忠诚跨过 `loyalty_revolt` 才是。**谓词集合就是全部设计面**——没声明的谓词 = 静默不记录。

### 2.2 Rust 侧保持可读，归一化只发生在投影层

`GameEvent` 的 variant 字段名继续用各自领域的可读写法（`attacker`/`fallen_to`/`owner`…），
**归一化不在这里做**。新增一层穷尽 `match` 的投影 API（`src/model/event.rs`）：

```rust
impl GameEvent {
    pub fn history_row(&self) -> EventRow;   // → 固定列：actor/target/extra/magnitude/data
    pub fn kind(&self) -> &'static str;      // 与 serde 判别式逐字一致（有守卫测试）
    pub fn salience(&self) -> Salience;      // Milestone / Notable / Detail
    pub fn participants(&self) -> Vec<Participant>;  // (role, kind, id) 长表索引
}
```

**这是本设计的牙齿**：`history_row` 是穷尽 `match`，**新增一个 variant 时编译器强迫你声明它的
参与方/显著性/载荷——历史不可能被「忘记记录」**。

`EntityKind{City,Ship,Faction,Body,Settlement}` × `EventRole{Actor,Target,Victim,Beneficiary,Third}`
构成 `Participant{role,kind,id}`。id 永远是**字符串名**（与全局身份约定一致）。

### 2.3 表形态

`idx/events.jsonl`（lazy，`event_id = "<round>:<seq>"`，主流带 `event_ids`）：

```
round seq event_id type salience actor_kind actor_id target_kind target_id extra[] magnitude data{}
```

- **无 variant 专属列**：`data` 一个对象列承载 per-type 载荷 → 一列只承载一种类型
  （不会出现「同时是标量和列表」的列，那种列 `isna()/sum()/dropna()` 全部不可靠）。
- `extra` 是 `[{role,kind,id}]` 长表：多参与方（联盟成员、凶手+旧主）都在这里。
- **统计建议：先按类型取**（`q.events(type=…)`）——那时字段是稠密的。

### 2.4 因果的新字段

```
ShipDestroyed { …, cause: DeathCause{Combat|UpkeepShortfall|Scrapped}, by: Option<Killer{ship,faction,weapon}> }
CityRazed     { city, fallen_to, by_ship, damage, pop_before }
ColonyFounded { …, how: FoundingHow{NewSite|Refounded}, prev_owner: Option<FactionId> }
Resurgence    { faction, body, ship, city }          // city 非 Option：每次重建必然落在一座城上
CityDefected  { city, from, to, loyalty }
Revolt        { city, faction, loyalty }
CityOverrun   { city, from, to }                     // 新增：难民夺活城（此前零事件）
ShipSpawned   { …, city: Option<CityId>, via: SpawnVia{Shipyard|Story} }
```

凶手来源：`fire()` 里第一发把 `hull` 打到 ≤0 的就是**补刀**（已在 0 的目标在循环开头被跳过），
记下该发的 `(attacker_id, faction, weapon_kind_name)`。不需要在 `resolve_shot` 里加管道。

### 2.5 Python 面（`play/planet_xq`）

```python
q.events(round=, type=, types=, since=, until=, salience=, entity=)   # 长表；单 type 自动摊平 data
q.actors()            # 长表 (round, seq, event_id, kind, id, role)，**通用展开**，无 variant 知识
q.history(kind, id)   # ★ 任意实体的历史
q.cause(kind, id)     # ★ ship → 凶手/弹种/助攻；city → 最近一次归属/存亡事件
q.fates(kind=, since=, until=)   # 窗口内结局清单
q.audit()             # 完备性自查（应为空）
```

**`actors()` 是通用展开**：从 `actor_*`/`target_*`/`extra` 三类槽位展开，**不需要任何 variant 的
字段知识**——所以「任意实体的历史」是同一个查询形状，这正是归一化的目的。

---

## 3. 实测（pandas 3.0.5，真实 715 事件投影）

| 设计 | 列 | 平均 null | 备注 |
|---|---|---|---|
| A 把 tagged enum 直接序列化成表（旧） | 23 | **74.8%** | faction 角色 7 种拼写 |
| B 归一化成固定 schema | 12 | **4.7%** | Rust enum 一行不改 |
| C 先按 type 取再查 | 12 | 相关字段 **0%** | NaN 是「混帧」的自我伤害 |

### 3.1 长表为什么舒服

「缺失」= **没有那一行**，不是 NaN。所以稀疏字段的四类统计都是单行：
`counts` = `groupby().size()`；`首末次` = `agg(["min","max"])`；`窗口率` = `groupby([id, round//w]).size()`；
`asof 存量` = `merge_asof(by=entity)`（实测 83 条稀疏易主事件 → 976 行密集「r 时归属」1.3 ms；
且 asof 出的 NaN **有意义**＝首次事件之前，不能无脑 fillna）。

### 3.2 宽表的坑（实测数字）

`size()` vs `count()` 分母不同（113 vs 18）；`value_counts`/`crosstab` 默认吞 NaN（709 行）；
`groupby` 的 NaN 键静默丢行；pivot 宽表 64% NaN；`Sparse[...]` dtype **只省 39% 内存、不改 API**；
`explode` 空数组造幽灵 NaN 行（`q.actors()` 里已显式 `dropna`）。

### 3.3 规模

60,775 事件：建索引 9 ms、单实体历史 0.84 ms、400 实体全直方图 7 ms。长表是线性的。

### 3.4 验收（seed 7 @ 60 回合）

| | 旧 | 新 |
|---|---|---|
| 平均 null | 74.8% | **4.7%** |
| variant 专属列 | 23 | **0** |
| 战死 vs 欠费报废 | 不可分 | `combat` 20 / `upkeep_shortfall` 55 |
| 难民夺城 | 零事件 | `city_overrun` **41** 条 |
| `q.audit()` | — | **0** 条未解释变化 |

`冥王星前哨` r50 的链（当初看不见）：
`siege(喀山) → city_razed(by_ship=喀山, fallen_to=俄罗斯) → colony_founded(how=refounded, prev_owner=欧盟, owner=无国界科学组织) → resurgence`。

---

## 4. Stage A 已落地

| 文件 | 改动 |
|---|---|
| `src/model/event.rs` | `DeathCause`/`Killer`/`SpawnVia`/`FoundingHow`；`CityOverrun`；新字段；`EntityKind`/`EventRole`/`Participant`/`Salience`/`EventRow` + `history_row/kind/salience/participants` |
| `src/sim.rs` | 全部变更点补齐因果：补刀凶手（`fire`）、`by_ship`/`pop_before`（`bombard_city`）、`prev_owner`/`how`（`colonize` + `step_resurgence`）、`Landing` 枚举区分复垦/占地/夺城、`CityOverrun`、剧情赠舰 `ShipSpawned` |
| `src/model/ship_combat.rs` | `weapon_kind_name()` |
| `src/projection.rs` | `events` 变 lazy（`idx/events.jsonl`）；主流 `events` → `event_ids`；`city_ids` **不再过滤 razed**（与 cities 表逐行一致）；schema 重写；**完备性守卫测试** |
| `src/main.rs` | `event_counts` 删掉手抄的 variant→label `match`，改走单一权威 `kind()` |
| `src/model/state.rs` | `SCHEMA_VERSION` 1→2（`GameEvent` 结构变了），migrate 文档说明 |
| `play/planet_xq` | `events/actors/history/cause/fates/audit` + docstring/README |

**关键守卫** `every_city_state_change_is_explained_by_an_event`：投影 120 回合 → 逐回合对比密集
快照，任何 `(faction_id, razed)` 变化都必须有**命名该城**的事件解释。实测 **145 次变化全部有解释**，
并断言 `checked >= 5`（**守卫必须非空**：一个什么都没检查的绿灯等于没有守卫）。
另有 `event_ledger_is_deterministic`（同 seed → 逐字节一致）。

> 顺手修掉一个测试基建 bug：`Scratch` 目录名 = 「进程 id + tag」，而 cargo 测试**同进程多线程并行**，
> 两个测试共用 tag `a`/`b` 会撞目录。

---

## 4b. Stage B 已落地（漏斗化 + 对账；**可证明行为中性**）

> 目标：让「忘记记事件」在**结构上不可能**，并用密集表把这条不变量钉死。

### 4b.1 状态变更漏斗（single writer）

`sim.rs` 里现在**每一处**归属/存亡写入都在漏斗内（已核查，无例外）：

| 漏斗 | 负责 | 取代的散落写入 |
|---|---|---|
| `kill_ship(ship, cause, by)` | hull 归零 + 记 `ShipDestroyed`（同舰只记一次） | `fire` 的 emit 循环、`step_upkeep` 的 scrap 循环 |
| `sweep_dead_ships(watched)` | 清扫 `hull ≤ 0` + **兜底补事件** + 清指令 | `step_military` 末尾的裸 `retain(hull > 0)` |
| `spawn_ship(ShipSpawn)` | 装配/取名/面板/付组件费 + 记 `ShipSpawned` | 船坞出厂、剧情赠舰、重建种子舰（**三处** push） |
| `raze_city(cid, RazeCause)` | 清人口/建筑/进度 + 记 `CityRazed`/`Revolt` | `bombard_city`、`step_governance` 的叛乱分支 |
| `reseed_city(cid, to, class)` | razed→活城 + 换主 + 记 `ColonyFounded{Refounded, prev_owner}` | `colonize` 复垦、`step_resurgence` 的复垦档 |
| `found_city(name, body, settlement, to, class)` | 新建城 + 记 `ColonyFounded{NewSite}` | `colonize` 新site、`step_resurgence` 的收容所档 |
| `overrun_city(cid, to, class)` | 夺活城 + 记 `CityOverrun{from, to}` | `step_resurgence` 的难民夺城档 |
| `defect_city(..., loyalty)` | 换主 + 迁控制叶子 + 关系打击 + 记 `CityDefected` | 原来事件由调用方另发（会漏） |
| `wire_city_control(cid, to)` | 给新建筑补继承型控制叶子 | 复垦/新建/夺城三处重复代码 |

**兜底设计**：`sweep_dead_ships` 用 `debug_assert_eq!(invented, 0)` —— 正常 0 艘需要兜底；
不为 0 就说明某条路径漏了 `kill_ship`，测试当场炸；release 下仍用最保守的 `Scrapped` 补一条
（历史完整，但不谎称是战损）。`watched` 参数使断言只管**本回合内**的死亡（回合开始前就死的舰
被顺带清走，不是本回合的漏记）。

### 4b.2 守卫扩到「舰的存亡」——当场抓出第二类漏洞

`every_ship_state_change_is_explained_by_an_event`：密集表里「出现/消失」的每艘舰都必须有事件。
**一上线就抓出 `step_resurgence` 的种子舰完全不发造舰事件**（实测 63 次出生里 **46 次无解释**）
——与「城易主查不到原因」完全同源，只是藏在舰那一侧。修法即上面的 `spawn_ship` 漏斗。

实测（120 回合，seed 42）：**242 次舰死亡 / 249 次舰出生 / 145 次城变化，全部有事件解释**；
两条守卫都断言「检查数 ≥ 5」（**守卫必须非空**）。

### 4b.3 行为中性的证明方法（可复用）

改完 `sim.rs` 后**不做**「跑一遍看着对」，而是 **golden-file 对比**：

```bash
git stash          # 或先在干净提交上生成基线
planet_x --seed 7 --round 60 --index play/baseline
# …改代码…
planet_x --seed 7 --round 60 --index play/after
python play/_golden_compare.py play/baseline play/after
```

判定：`idx/{cities,ships,factions,bodies,settlements}.jsonl` + `meta.json` 必须**逐字节一致**；
`main.jsonl` 去掉 `event_ids` 后必须逐字节一致；`idx/events.jsonl` **允许**不同（它记录的是
「本来就发生但没被记下来」的事，或同回合内事件次序变化）。

**结果：Stage B 全程通过**——漏斗化只增加了 46 条 `ship_spawned` 记录，模拟本身逐字节未变。

### 4b.4 一次**被否决**的改动（重要的负结果）

`step_ideology` 里原本用「同回合最后一条 `Attack` 的势力」近似凶手（因为已死目标会被开火循环
跳过，那条 `Attack` 其实必然就是补刀）。既然 `ShipDestroyed` 已有权威 `by`，替换掉近似看起来
是纯改进：

- 60 回合窗口：**逐字节一致**（含 events.jsonl），0/20 起不一致 → 看起来零风险。
- 1000 回合长局：**翻转 `world_is_multipolar` 的「霸权轮换」判定** —— seed 1 后半程被
  **俄罗斯锁死**（轮换数 1 < 需要的 2；峰值占比 0.820，逼近 0.85 上限）。

差别只在「凶手舰自己在同回合被反杀」这种罕见情形（旧逻辑 `state.ship(attacker)` 拿不到就漏记功劳）。
一次罕见的分歧足以在 1000 回合的混沌里改变结局 → **这是平衡层面的改动，不是历史完备性的改动**，
故**回退**，并记入 §5 待单独做平衡验证。

> 教训：**「60 回合逐字节一致」不足以证明长局中性**。窗口内中性 + 长局守卫失败 = 行为改动，
> 必须按行为改动对待。

---

## 5. 未做（Stage C / 后续）

### 单独立项（有测量依据）
- **用 `ShipDestroyed.by` 替换 `step_ideology` 的近似 `killer_of`**：见 §4b.4 —— 窗口内中性，
  但 1000 回合会翻转 `world_is_multipolar`。需要**单独一次平衡验证**（连同 `probe_multipolar`
  的横向对比），不该混在历史改动里。
- **僵尸势力的「夺城—倒戈」振荡**（见 §6）：`city_overrun` 41 条 / 60 回合，同一座城 4 回合一轮
  易主。改法候选三条，均需长局验证。

### Stage C：长存账本 + 叙事性价比
- `State.ledger`：只收 `Salience::Milestone` 的**稀疏里程碑层**，随 checkpoint 存活（解决根因 ②）。
  天然有界 ≈ O(实体数 × 常数)，海量 `attack`/`siege` 流水永不进 State。
- **一句话 headline**：`GameEvent::headline()` 单点渲染（`第47回合：中国夷平火星-殖民城，美国失一城`），
  `--digest` / `chronicle` / Python 三处共用。**完备 ≠ 可读**——这正是 ideas.md §15「窗口事件文案」的前置。
- salience 权重配置化（`config/game.ron`），`--digest` 加 `top_events`。
- `cause_id`（事件级因果链显式引用）——**目前刻意没做**：实测里 `cause_id` 会是 100% null 的死列，
  而链在 Python 侧用一张小规则表 + 类型结构就能走通（`razed.by_ship`、`destroyed.by`、`how/prev_owner`）。
- 投影体积：归一化后 `idx/events.jsonl` 比原来的内联事件大约多 50%（列更多）。
  3000 回合量级可考虑 parquet（ideas.md §17 候选）。

---

## 6. 由账本暴露出来的新问题（值得单独修）

**僵尸势力的「夺城—倒戈」振荡。** seed 7 @ 60 回合里 `city_overrun` 有 **41** 条、`resurgence` 46 条。
`冥王星前哨` 每 4 回合循环一次：

```
r33 city_defected 星系矿业 → 欧盟        (step_governance：忠诚跌破阈值)
r33 city_overrun  欧盟 → 星系矿业        (step_resurgence：星系矿业此时既无舰又无活城 → 难民夺城)
r33 resurgence    星系矿业
… r37 / r41 / r45 / r49 完全重复
```

机制自相抵消：`step_governance` 先跑（城倒戈走了）→ 该势力立刻变成「无舰无活城」→
`step_resurgence` 当回合就跑，用 `displace_city_for_refugee` 把**同一座城**夺回来。
净效果为零，却把这座城永久钉在 4 回合一轮的易主循环里，并使事件量翻倍。
**改法候选**：① `resurgence` 加冷却（近 N 回合内重建过则跳过）；② 夺城目标排除「刚被本势力丢掉的城市」；
③ 让难民避难所优先选**未被倒戈过**的城。**注意**：任何改法都可能影响
`zombie_factions_are_bounded` / `world_is_multipolar` 守卫，需长局验证。

---

## 7. 复现

```bash
cargo run --bin planet_x -- --seed 7 --round 60 --index play/hist_probe
python play/_smoke_history.py            # 端到端：历史链 / 凶手 / 死因 / audit
cargo test --lib                         # 56 passed（含 5 个投影守卫）
cargo test --test longhorizon            # 6 passed / 8 ignored

# Stage B 的行为中性证明（golden-file 对比）
python play/_golden_compare.py play/baseline play/after
```

未跟踪的临时探针（可删）：`play/_probe_sparse{,2,3}.py`（pandas 稀疏字段实测）、
`play/_smoke_history.py`、`play/_probe_stageb.py`（舰存亡对账 + 凶手近似的差异率）、
`play/_golden_compare.py`、`play/_mp_leaders.py`（单极锁死量化）、`play/hist_probe/`、
`play/{baseline,after*,mp_*}`。
