# 稀疏历史 / 事件历史（sparse history / 三层历史）

> 目标：让「**一个城市易主了，就近是什么事件导致的？被夷平然后被殖民，还是叛乱？**」和
> 「**一艘舰被击毁，是被哪艘舰击毁？**」都能在 Python 里一行 join 查出来。
>
> 状态：**Stage A / B / C 均已落地**，已并入 `main`（`cb813f2` 为 A+B；C 见 §4c）。
> 后续/未做见 §5，由里程碑暴露出的新问题见 §6。

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
另有 `event_milestones_is_deterministic`（同 seed → 逐字节一致）。

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

### 4b.4 一次**被否决**的改动——后来发现否决的理由是错的（重要的负结果）

`step_ideology` 里原本用「同回合最后一条 `Attack` 的势力」近似凶手（因为已死目标会被开火循环
跳过，那条 `Attack` 其实必然就是补刀）。既然 `ShipDestroyed` 已有权威 `by`，替换掉近似看起来
是纯改进：

- 60 回合窗口：**逐字节一致**（含 events.jsonl），0/20 起不一致 → 看起来零风险。
- 1000 回合长局：**翻转 `world_is_multipolar` 的「霸权轮换」判定** —— seed 1 后半程被
  **俄罗斯锁死**（轮换数 1 < 需要的 2；峰值占比 0.820，逼近 0.85 上限）。

当时判定「这是平衡层面的改动」→ **回退**。

**后来在 Stage C 复查，这个判定是错的**：那次只改了「凶手」一侧，而同一段代码里还有**第二处
同类缺陷**（失城方回读、见 §4c.3）没修；两处一起修之后，长局不但没有锁死，反而更健康
（seed 1 轮换数 **1 → 25**，见 §4c.5）。也就是说，当时那次"平衡回归"其实是**半个修正**造成的
失真，不是一个正确的修正带来的代价。

> 教训（两条，都值得记住）：
> 1. **「60 回合逐字节一致」不足以证明长局中性**——窗口内中性 + 长局守卫失败 = 行为改动。
> 2. **但「长局守卫失败」也不等于「这个改动是纯平衡调整」**：先问「同一段逻辑里是不是还有
>    别的缺陷没修」。半个修正的症状，看起来和平衡回归一模一样。

---

## 4c. Stage C 已落地（长存里程碑 + 可读性 + 三处「事后回读」的根治）

### 4c.1 长存里程碑（解决根因 ②）

> ⚠ **本节的前提已被 [§4d](#4d-stage-d-已落地分层判据从重要性换成后续计算的访问需求--取代-4c-的前提) 取代。**
> 长存层按「听起来重要」预填 14 个 variant 是**错的**：按 Stage D 的判据（读者需求）它应当是
> **空的**，而预填的代价实测是吃掉存档 **67%**。下面这段保留下来作为「当时的判断」的记录，
> 但**不要照它行事**——判据与现状见 §4d。

`State.milestones: Milestones { entries: Vec<MilestoneEntry{round, event}> , dropped, dropped_through_round }`，
只收 `Salience::Milestone`（由 `GameEvent::salience()` 单点声明），**不随回合清空**。

- **写入点是唯一的发事件漏斗 `sim::ev`**（`state.milestones.push(round, e)` 后 `state.events.push(e)`），
  于是「事件发了、里程碑没记」在结构上不可能——与 `kill_ship`/`spawn_ship` 是同一套纪律。
- 容量裁剪在 `advance` 收尾按 `config.history.max_milestones` 做（0 = 默认**无损**）；
  截断时最旧的先丢，并把 `dropped` / `dropped_through_round` 记进里程碑自身——**截断可见**。
- `Milestones::history_of(kind, id)` 是 Rust 侧的「某实体全部里程碑」（投影侧对应 `q.milestones(entity=…)`）。
- `--milestones [N]` 输出 `{round, count, returned, complete, dropped, dropped_through_round, events:[{round, headline, event}]}`。
- `SCHEMA_VERSION` 2 → 3。

### 4c.2 一句话 headline（单点渲染，三处共用）

`GameEvent::headline() -> String`：穷尽 match（新增 variant 时编译器强迫你写那句话），
**自足**（只读事件自身字段，不接受 `&State`）、**单行**、且 **`participants()` 列出的每个 id 都
逐字出现在句子里**（守卫 `headline_names_every_participant` 钉住）。

- 自足是硬要求：里程碑里的事件是**归档历史**，实体可能早就没了或改了名，回查 state 只会得到
  「今天的答案」而不是「当时的答案」。
- 同一句话出现在三处：CLI `--milestones` / `--digest` 的 `top_events`、投影 `idx/events.jsonl` 的
  `headline` 列、Python `q.milestones()`。**完备 ≠ 可读**，这是 `coarse-trajectory-views.md` 里「窗口一句话事件文案」的前置。
- 投影的事件行改为**由 `EventRow` 的序列化结果生成**（此前手写 `json!`，加了列就会悄悄漏掉——
  `headline` 就是这么差点漏的）。

### 4c.3 三处「事后回读」的根治（`step_ideology`）

旧 `step_ideology` 在**回合末**回读 state 去重建「回合中发生了什么」，两处都读错了：

| 事实 | 旧读法 | 为什么错 | 现在 |
|---|---|---|---|
| 谁击沉了这艘舰 | 同回合最后一条 `Attack` 的势力，且要 `state.ship(attacker)` 才知阵营 | **互杀**时凶手舰已不在 `state.ships` → 战功**丢失** | `ShipDestroyed.by`（补刀那一刻记下） |
| 谁丢了这座城 | `state.city(city).faction_id` | 夷平**不改归属**，同回合稍后的复垦会把它改成新主 → 扣分记到了**抢城者**头上 | **新增 `CityRazed.owner`**（夷平那一刻的失主） |

顺带修掉一处**分支不一致**：`CityDefected`（离心倒戈主路）与 `Revolt`（找不到倒戈目标时的兜底）
是**同一个触发**的两条分支，旧代码给兜底 −1、主路 0 分——等于「分数取决于世界上有没有可倒戈的
势力」这个与本次得失无关的偶然。现在两种活城易主（倒戈 / 难民夺城）与夷平一律同等计分。

信号计算被抽成**纯函数** `military_deltas(&[GameEvent]) -> BTreeMap<FactionId, f64>`——只吃事件、
完全不看 state，于是这条规则可以被单元测试逐条钉死（`military_signal_uses_the_milestones_and_is_branch_agnostic`）。
**`colony_founded` 刻意不计**：新建/复垦是殖民行为，归 `nature_colony` 轴，记进军事轴会让殖民者
集体漂向军国。

### 4c.4 同回合归属翻转不变量（僵尸振荡的结构性约束）

`step_governance`（离心倒戈）先跑、`step_resurgence`（难民夺城）后跑。后者若把前者刚放手的城
夺回来，两个步进就在**同一回合里正好互相抵消**：净效果为零，却照样记两条里程碑、白造一艘种子舰。

修法：`displace_city_for_refugee(state, exclude)` —— `exclude` = 本回合已经易主过的城。这个集合
**在 `step_resurgence` 循环里还会继续增长**（每落实一个立足点就把那座城加进去），因此同一个回合里
没有哪座城会被重建/夺取两次。若强占候选全被排除则**本回合不重建**（宁可该势力多当一回合僵尸，
也不做净零动作）。

效果（seed 7 @ 60 回合）：`city_overrun` **41 → 20**。守卫
`no_city_changes_owner_twice_in_one_round` 钉住「一座城一回合内不能易主两次」
（`city_defected` / `city_overrun` / `colony_founded` 三者之和每 `(回合, 城)` 最多 1 条）。

> 注意 `city_razed` → `colony_founded`（被夷平后同回合复垦）**不算违规**：城经过了「死亡」这个
> 中间态，是两件不同的事，两条事件都是真的。剩下这类多回合「拆平—复垦」循环见 §6。

### 4c.5 顺带修掉一个**预存重大缺陷**：RON 读不回 checkpoint（`--save`/`--start` 全断）

排查「`--save` 写出的档 `--start` 读不回来」时定位到：**单元 enum 嵌在内部标签 enum 里**时，
serde 默认把单元变体写成**裸标识符**（`cause:combat`），而 RON 的 `deserialize_any` 无法把裸标识符
还原成内容 → 整份 `State` 反序列化失败。

最小复现：

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum E { A { ship: String, cause: DeathCause } }   // DeathCause 是纯单元 enum
// (type:"a",ship:"x",cause:combat)      ← 读不回来
// (type:"a",ship:"x",cause:"combat")    ← 可以
```

- **是 Stage A 引入的**：`DeathCause` / `SpawnVia` / `FoundingHow` 都是 Stage A 新增的单元 enum
  （在此之前 `GameEvent` 里没有单元 enum，所以 RON 往返是好的）。
- 危害被 `load_initial` 放大了：它把「checkpoint 解析失败」**静默咽掉**、退回当裸 `State` 解析，
  最终报成一句指向**文件末尾**的「missing field `round` in `State`」——离真正的原因十万八千里。
- **为什么一直没人发现**：`--save`/`--start` 是文档里的核心流程，却**没有任何测试**覆盖它。
- 修法：`stringly_unit_enum!` 宏给这三个 enum 生成字符串化的 `From`/`TryFrom<String>`，配合
  `#[serde(into = "String", try_from = "String")]`。**JSON 形状一字节不变**（`serde_json` 本来也把
  单元变体写成字符串），只有 RON 从裸标识符变成带引号的字符串。认不出的标签**明确报错**，不退回默认变体。
- 守卫：`every_game_event_variant_round_trips_through_ron`（**逐个变体**过 RON+JSON 往返，
  配一个穷尽 match 的 `variant_checklist` 提醒新增变体要补样本）、
  `checkpoint_survives_save_and_resume_identically`（存→读→续跑，必须与一路跑到底**逐字节一致**）。

### 4c.6 验证

`cargo test --lib` **68 passed**（+9）、`cargo test --test longhorizon` **6 passed / 8 ignored**、
无警告。多极健康（`probe_multipolar`，1000 回合）：

| seed | avg_top | terminal_top | rotations | alive | zombies | gini |
|---|---|---|---|---|---|---|
| 1 | 0.637 | 0.377 | **25** | 9 | 0 | 0.455 |
| 42 | 0.592 | 0.656 | 80 | 9 | 0 | 0.505 |
| 12345 | 0.535 | 0.665 | 83 | 9 | 0 | 0.505 |

对比 §4b.4 那个「只改凶手一侧」的半修正（seed 1 轮换数 **1**、峰值 0.820、被判锁死）：
**两处一起修之后反而更健康**。

**里程碑体积（实测，用于决定默认是否设上限）**：round 60 的 checkpoint 106 KB，其中里程碑
43.5 KB（**41%**），约 94 字符/条、7.7 条/回合 → 1000 回合约 **0.7 MB**（而实体部分几乎不随
回合增长）。因此默认**无损**（`max_milestones: 0`），超长归档局可设上限换取有界文件。

---

## 4d. Stage D 已落地（分层判据从「重要性」换成「后续计算的访问需求」）—— **取代 §4c 的前提**

§4c 把「听起来重要」的 14 个 variant 预填进长存层。实测那个决定**吃掉存档 67%**（r200：
249,557 / 372,248 字节，1860 条 ≈ 9.3 条/回合），而它与「后续计算要不要回看它」毫无关系。

### 4d.1 判据（写进 `Salience`，**文档即规格**）

| 层 | 判据（**唯一的判据**） | 载体 |
|---|---|---|
| `Milestone` | 后续计算需要访问**无限的过去历史** | `State::milestones`，随 checkpoint 存活，无窗口 |
| `Notable` | 后续计算需要访问**一定的事件窗口** | `State::notables` + `history.notable_window`（默认 24 回合） |
| `Detail` | 后续计算**只需前一帧**，或**根本没有读者**——只为 agent 分析而记录 | `State::events`（一回合）+ 磁盘投影（永久，供 agent） |

判据的**方向**很重要：它问的不是「这条事件重不重要」，而是「**谁**在**多久之后**要回看它」。
所以每一条定级都必须能指到**具体读者**；指不到读者的，就是 `Detail`。`Milestones` 与 `Notables`
共用 `HistoryEntry{round, event}`——两层的差别只在**保留期**，不在记录形状。

### 4d.2 读者盘点 → 里程碑层清空

盘点非测试的 `state.events` / `state.notables` 读者，**全部只读当前回合**：`step_ideology` 的
`military_deltas`、`kill_ship` 的同回合去重、`step_diplomacy` 的「本回合谁和谁交火」、
`step_story` 的载荷提取 + `StoryTrigger::*`、`tactics` 的 `Withdraw` 判定、`agent.rs` /
`projection.rs` 的渲染。

**没有任何逻辑读无限过去**，所以里程碑层**清空**（14 → 0），字段保留但不预填。仅有的无限过去
需求（`first_war` / `first_raze` / `first_colony` 的「史上第一次」）已经有载体：`state.chronicle`
的 `id` 去重——历史层不必为它们长存。

> **一般化的教训**：真正的无限过去需求**通常该被折成一个 state 标量（累加器）**，而不是回放原始
> 历史。`step_ideology` 就是这么做的——每回合只读当前帧的 `military_deltas`，累加进 `ideology`
> 标量。历史层的价值只在「折不成标量」的场合。这也解释了为什么里程碑层按判据应当近乎空，以及
> 为什么「先按重要性预填」必然会填错。

### 4d.3 窗口层与它的第一个读者：记恨（战争疤痕）

`WarStarted` / `WarEnded` → `Notable`，依据是明确的范例：**开战之后相当一段时间两国会互相记恨**
——所以外交计算要回看「最近 W 回合我们打过仗吗」，而 W 之外的那场战争不再影响任何计算，因此
**不必**长存。这就是「窗口」这一层的存在理由，而不是「战争比较重要」。

实现：`sim::war_scar_floor()` 在窗口内找**最近一次** `WarStarted{a,b}`（无序匹配），返回一道从
`war_scar_relation`（-30）**线性衰减到 0** 的关系地板。新鲜时 -30 < `war_threshold`(-20) →
刚开战的对手**不可能当回合言和**；地板在第 `war_scar_rounds*(1-20/30)` = **9** 回合抬过阈值 →
和平重新可能（「记恨，但会淡」）。

实测（seed 7）：开战/停战 r60 **65/63 → 32/29**，r200 **93/91 → 59/57**；最短战争从「闪烁」
变成**恰好 9 回合**（55 场里 30 场 = 9，中位数 9）。

> **它同时治好了 §6.2 的阈值抖动**——但不是靠迟滞，而是靠「一场战争至少持续 9 回合」。

### 4d.4 修 bug：一条地板如果有多个写入者，它就不是地板

地板最初只套在 `step_diplomacy` 的漂移里。但关系有**多个**写入者，其中
`step_balance_of_power` 的「合纵」（弱者相互靠拢）跑在 `step_diplomacy` **之后**，把两个正彼此
交战的弱者拉近——于是实测最短战争只有 **6** 回合（3 场），直接推翻了地板的承诺。

- **错误的修法**：放宽断言（把期望的最小值改成 6）——那是把 bug 写成规格。
- **正确的修法**：把地板放进**关系写入的唯一漏斗**（`set_relation_sym` + `adjust_relation`），
  与 `ev()` / `kill_ship()` 那套 single-writer 纪律同源。修正后 55 场战争 `min = 9`，零违例。
- 守卫 `war_scar_floor_makes_a_real_floor_on_war_duration` **用 60 回合真实长局**验证最短战争
  时长，而不是只测那个函数——只测函数的话，这个 bug 完全测不出来。

### 4d.5 `salience` 是分层，`weight` 是重要性——投影必须两列都有

清空后暴露一个新坑：`--digest` 的 `top_events` 与 `q.storyboard()` 原本按 `salience == Milestone`
挑「值得读的事件」。里程碑层清空后，它们会**静默返回空**。这说明**「分层」和「重要性」是两件事**，
不能互相替代：

- `salience` 回答「**谁要回看它**」→ 分层 / 存储。
- `weight`（`GameEvent::weight`，穷尽 match、**纯显示用**）回答「**人读起来重不重要**」→ 排序。

于是投影新增 `weight` 列，`top_events` 与 `q.storyboard()`（新增 `min_weight=8`）改按它挑；
`--schema` 的 `column_docs` 里显式写明这条区别，避免下一个 agent 再混用。

> **又一个同类坑**：`weight` 的**量纲是 0–9 的序数阶梯**（9=开战/结盟/迁都、8=城市易主或毁灭、
> 7=重建/剧情、5=舰存亡、2=撤退/降级、0=逐发流水），不是 0–100 的分数。门槛一度被写成 60
> ——`q.storyboard()` 于是**静默返回空表**，而所有测试都是绿的。守卫
> `weight_ladder_stays_a_documented_zero_to_nine_scale` 现在把量纲与阶梯顺序钉住。
> 教训与 §4d.5 同源：**「静默变空」是过滤器类 bug 的默认失败模式**，所以门槛必须有守卫。

### 4d.6 验证

`cargo check --workspace --all-targets` 无警告；`cargo test --lib` **72 passed**（新增 5 条，其中
`history_layers_are_assigned_by_reader_need_not_importance` 是**守卫里程碑层为空**——谁凭
「听起来重要」把它填回去就会红）；`longhorizon` 6 passed / 8 ignored；`probe_multipolar` 三 seed
均 9 势力存活 / 0 僵尸 / 无霸权失控（rotations 47 / 71 / 90）。
**r200 存档 372,248 → 108,889 字节（-71%）**。`SCHEMA_VERSION` 3 → 4（只升版本号：v3 档里的
里程碑是按旧判据预填的残留，不是任何计算回看的历史）。

---
## 4e. Stage E 已落地（拆平/复垦极限环：把「同回合抵消」不变量补齐到 anchor 1/2）

Stage D 清空里程碑层时读到 r196/198/200 **一字不改重复**的四连，量化后是当时最大的单一事件来源。

### 4e.1 缺陷不是缺机制，而是**既有不变量的应用不完整**

`displace_city_for_refugee` 的注释早就写明这类缺陷（「两个步进在同一回合里正好互相抵消：净效果
为零，却照样记两条里程碑事件、还白造一艘种子舰」），但那次只为 **anchor 4（难民夺城）** 加了
`flipped_this_round` 过滤，**anchor 1/2（自己的废墟 / 任何空白避风港）漏掉了**。实测（seed 7
@200 回合，修正前）：137 次 `city_razed` 里 **93 次（68%）是同回合被同一势力复垦**（归属 A→A），
`水星熔炉基地` 一座城循环 22 次、被拆平 32 次，「复垦→再被拆平」间隔集中在 1–2 回合。

### 4e.2 第一次试错把设计打坏了——值得单独记

按「跳过这个势力、让拆平的后果站住一回合」（`continue`）实现，结果 `longhorizon` 挂 2 条，
`probe_multipolar` 出现 **zombies 6/6/5**（原本 0/0/0）、存活势力 9→7。

原因是一个**测量口径**问题：`zombie_count` 是**整局采样到的峰值**，而 `continue` 让被拆平的势力
**在本回合末真的没有立足点**，采样点正好抓到它。

> **结论：反僵尸保证的是「回合末总有立足点」，不能被任何过滤破坏。** 一个「让后果站住」的直觉
> 修法，撞上这条不变量时就是错的——而它只会在长局守卫里显形（单元测试全绿）。

### 4e.3 正确修法：与 anchor 4 同构的「换立足点」

排除那块刚丢的废墟，**改在别的立足点重建**（自己的另一块废墟 → 任何空白避风港 → 未占据定居点
新建）。于是既没有净效果为零的同回合抵消，也没有任何一个回合末留下无立足点的势力。

实测：总事件 **2273 → 1848（-19%）**、`city_razed` **137 → 60**、同回合自我复垦 **93 → 0**、
被拆平 ≥5 次的城 **10 → 5**、最严重的城 **32 → 6**。代价：`city_overrun` 46 → 90（抵消换成了
迁往别的立足点，与 anchor 4 同构）。长局三 seed 全部 **alive 9 / zombies 0**，rotations
47/71/90 → **52/101/118**，terminal_top 与 gini 都更均衡。

### 4e.4 顺带抓出一个潜伏的洞：判定集合 ≠ 保护集合

`flipped_this_round` 的**种子集合**与它保护的**不变量定义**不一致：

| | 定义 |
|---|---|
| 守卫 `no_city_changes_owner_twice_in_one_round` 说「活城易主」= | `city_defected` / `city_overrun` / **`colony_founded`** |
| 种子集合实际收的 | `{city_defected, city_overrun, city_razed}` ← **漏了 `colony_founded`** |

于是**本回合刚被殖民舰复垦的城**在 anchor 4 眼里不算「已易主」，可以被同回合的难民夺走：实测
回合 76/79 的 `大红斑科学站`——「深空运输联盟 复垦 俄罗斯 留下的废墟」之后同回合又被「联合国的
难民夺取」。它**潜伏**在那儿，是这次改动了 anchor 1/2 的选择顺序才被踩到。

> **教训：一条不变量的「判定集合」和「保护集合」必须是同一个定义——否则它只在没人踩的时候成立。**

### 4e.5 判据的第二次「拦下」：这次不需要窗口层

churn 曾经是「窗口层下一个读者」的头号候选（「我是不是刚在这座城栽过」）。真去修的时候发现：
**只需要本回合的 `flipped_this_round`，不需要任何窗口**。所以 `CityRazed` **仍然留在 `Detail`**。

这是「先有读者再提升定级」这条纪律第二次拦下一个想当然的提升（第一次是战争——那次是真的有窗口
读者：记恨）。**结论：不要因为「这个缺陷看起来需要历史」就去提升定级；先看修法到底读了什么。**

---
## 5. 未做 / 后续

### 单独立项（有测量依据）
- ~~用 `ShipDestroyed.by` 替换 `step_ideology` 的近似 `killer_of`~~ → **已做**（§4c.3）。
- ~~僵尸势力的「夺城—倒戈」振荡~~ → **同回合自相抵消那部分已修**（§4c.4）；
  剩下的多回合「拆平—复垦」循环见 §6。

### Stage C 里剩下的（都是**刻意没做**，各带理由）
- ~~`State.milestones` / `headline()` / `--milestones` / `top_events`~~ → **已做**（§4c.1/4c.2）。
- **salience 权重配置化**：`--digest` 的 `top_events` 目前用**Rust 里声明的** `GameEvent::weight()`
  （穷尽 match）。它是一个**展示排序键**、不是模拟数值，外置成 config 反而会让「新增 variant
  必须声明权重」这条编译期纪律失效。将来真要按剧本调叙事重点，再改成「穷尽 match 读 config」的形式。
- **`cause_id`（事件级因果链显式引用）**：仍**刻意没做**——实测会是 100% null 的死列，而链在
  Python 侧用 `razed.by_ship` / `destroyed.by` / `how+prev_owner` 的结构就能走通。
- **投影体积**：归一化后 `idx/events.jsonl` 比原来的内联事件大约多 50%（列更多，现在还多一列
  `headline`）。3000 回合量级可考虑 parquet（`lazy-index-pandas.md` 候选）。
- **`--digest` 的窗口粒度**：`top_events` 只列 [Milestone]，上限 24 条（`TOP_EVENTS`），
  `skipped` 如实给出。窗口太大时最重的会被「开战/停战」这类阈值抖动占满（见 §6 末尾），
  需要更细的窗口而不是更多条目。

---

## 6. 由里程碑暴露出来的新问题（值得单独修）

### 6.1 僵尸势力的「夺城—倒戈」振荡——**同回合抵消部分已修**

seed 7 @ 60 回合（改动前）`city_overrun` 有 **41** 条、`resurgence` 46 条；`冥王星前哨` 每 4 回合
循环一次：

```
r33 city_defected 星系矿业 → 欧盟        (step_governance：忠诚跌破阈值)
r33 city_overrun  欧盟 → 星系矿业        (step_resurgence：星系矿业此时既无舰又无活城 → 难民夺城)
r33 resurgence    星系矿业
… r37 / r41 / r45 / r49 完全重复
```

同回合自相抵消已由 §4c.4 修掉（`41 → 20` 条 `city_overrun`），并有守卫钉住。

**剩下的是多回合循环（未修）**：`reseed_city` 每次都挑该势力**自己最低名的空白城**，于是
「欧盟拆平 → 联合国复垦 → 欧盟再拆平」可以在同一处反复很多轮。实测 seed 7 的
`大红斑科学站`（`q.milestones(entity=('city','大红斑科学站'))` 可直接读）：

```
 r4 city_defected  无国界科学组织→星系矿业   r11 city_razed 中国拆平(联合国) + 中国复垦
 r7 city_overrun   联合国夺回 + 复垦          r20 city_overrun 星系矿业夺回 + 复垦
 r27 city_defected 星系矿业→中国              r33/r35/r37/r39/r45/r47… 欧盟拆平 ↔ 联合国复垦
```

这不是「净零动作」（每次复垦都要重建人口/建筑，是真实损耗），而是**反僵尸机制**（保证联合国
永远能重新立足）与**舰炮拆城**互相咬住。它属于机制/平衡设计，不是状态机错误。
改法候选：① 复垦锚点排除「最近 N 回合内被拆平过」的城；② `resurgence` 加冷却；③ 让重建优先
选**别人**的废墟（它现在只挑自己的 diaspora claim）。**注意**：都会影响
`zombie_factions_are_bounded` / `world_is_multipolar`，需长局验证。

### 6.2 战争的阈值抖动（**已由 §4d.3 的「记恨地板」修掉**）

`--digest 20` 的 `top_events` 里（权重最高的就是开战/停战）能看到**同一对势力在一个 20 回合窗口
里开战→停战→开战**好几次（实测 seed 7 @ r40–60：`欧盟↔行星X崇拜教` 三个来回）。`hostile` 是
`relation <= war_threshold` 的**硬阈值**，而关系每回合带随机漂移，于是阈值附近会来回穿越，
每次都记一对 `war_started`/`war_ended`（两回合内不冲突，但多回合反复）。

这不是「同回合抵消」（`war_pairs` 是回合前后各算一次，一对势力一回合最多跃迁一次），而是**缺回滞
（hysteresis）**：标准修法是开战阈值与停战阈值不同（`ceasefire_relation > war_threshold`，
config 里已有 `ceasefire_relation` 字段可以复用）。属机制/平衡设计。

> **后续（§4d.3）**：最后没有走迟滞，而是走「记恨地板」——一场战争至少持续 9 回合，
> 于是阈值附近的来回穿越不再每次产出一对事件。实测 r60 开战/停战从 65/63 降到 32/29，
> r200 从 93/91 降到 59/57。若将来还要更硬的迟滞（进入战争需要更长的敌对积累），
> 那会是 `Notable` 窗口的又一个读者（读最近 W 回合的 `attack`/`siege` 密度）。

---

## 7. 复现

```bash
cargo run --bin planet_x -- --seed 7 --round 60 --index play/after
cargo run --bin planet_x -- --seed 7 --round 60 --save play/c60.ron
cargo run --bin planet_x -- --start play/c60.ron --round 0 --notables 40   # 窗口层活过 checkpoint
cargo run --bin planet_x -- --start play/c60.ron --round 0 --milestones   # count: 0 —— 按判据为空（§4d.2）

python play/_probe_invariant.py play/after/idx/events.jsonl   # 同回合翻转不变量 + headline 完整性
cd play/planet_xq && uv sync
uv run python -c "from planet_xq import load; q=load('../after'); print(q.history('city','大红斑科学站')); print(q.notables()); print(q.storyboard(50))"

cargo test --lib                          # 72 passed（含 6 个 config 往返/分层守卫 + 8 个投影守卫）
cargo test --test longhorizon             # 6 passed / 8 ignored
cargo test --test longhorizon probe_multipolar -- --ignored --nocapture   # 多极健康报告
```

未跟踪的临时探针（可删）：`play/_probe_sparse{,2,3}.py`（pandas 稀疏字段实测）、
`play/_smoke_history.py`、`play/_probe_stageb.py`（舰存亡对账 + 凶手近似的差异率）、
`play/_golden_compare.py`、`play/_mp_leaders.py`（单极锁死量化）、`play/_probe_invariant.py`
（同回合翻转不变量）、`play/_ledger.py`（`--notables` 渲染）、`play/_ron_locate.py`（RON 定位）、
`play/hist_probe/`、`play/{baseline,after*,mp_*}`。
