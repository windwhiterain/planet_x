# 稀疏历史 / 事件账本（sparse history ledger）

> 目标：让「**一个城市易主了，就近是什么事件导致的？被夷平然后被殖民，还是叛乱？**」和
> 「**一艘舰被击毁，是被哪艘舰击毁？**」都能在 Python 里一行 join 查出来。
>
> 状态：**Stage A / B / C 均已落地**，已并入 `main`（`cb813f2` 为 A+B；C 见 §4c）。
> 后续/未做见 §5，由账本暴露出的新问题见 §6。

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

## 4c. Stage C 已落地（长存账本 + 可读性 + 三处「事后回读」的根治）

### 4c.1 长存里程碑账本（解决根因 ②）

`State.ledger: Ledger { entries: Vec<LedgerEntry{round, event}> , dropped, dropped_through_round }`，
只收 `Salience::Milestone`（由 `GameEvent::salience()` 单点声明），**不随回合清空**。

- **写入点是唯一的发事件漏斗 `sim::ev`**（`state.ledger.push(round, e)` 后 `state.events.push(e)`），
  于是「事件发了、账本没记」在结构上不可能——与 `kill_ship`/`spawn_ship` 是同一套纪律。
- 容量裁剪在 `advance` 收尾按 `config.history.max_milestones` 做（0 = 默认**无损**）；
  截断时最旧的先丢，并把 `dropped` / `dropped_through_round` 记进账本自身——**截断可见**。
- `Ledger::history_of(kind, id)` 是 Rust 侧的「某实体全部里程碑」（投影侧对应 `q.ledger(entity=…)`）。
- `--ledger [N]` 输出 `{round, count, returned, complete, dropped, dropped_through_round, events:[{round, headline, event}]}`。
- `SCHEMA_VERSION` 2 → 3。

### 4c.2 一句话 headline（单点渲染，三处共用）

`GameEvent::headline() -> String`：穷尽 match（新增 variant 时编译器强迫你写那句话），
**自足**（只读事件自身字段，不接受 `&State`）、**单行**、且 **`participants()` 列出的每个 id 都
逐字出现在句子里**（守卫 `headline_names_every_participant` 钉住）。

- 自足是硬要求：账本里的事件是**归档历史**，实体可能早就没了或改了名，回查 state 只会得到
  「今天的答案」而不是「当时的答案」。
- 同一句话出现在三处：CLI `--ledger` / `--digest` 的 `top_events`、投影 `idx/events.jsonl` 的
  `headline` 列、Python `q.ledger()`。**完备 ≠ 可读**，这是 §15「窗口事件文案」的前置。
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
完全不看 state，于是这条规则可以被单元测试逐条钉死（`military_signal_uses_the_ledger_and_is_branch_agnostic`）。
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

**账本体积（实测，用于决定默认是否设上限）**：round 60 的 checkpoint 106 KB，其中账本
43.5 KB（**41%**），约 94 字符/条、7.7 条/回合 → 1000 回合约 **0.7 MB**（而实体部分几乎不随
回合增长）。因此默认**无损**（`max_milestones: 0`），超长归档局可设上限换取有界文件。

---

## 5. 未做 / 后续

### 单独立项（有测量依据）
- ~~用 `ShipDestroyed.by` 替换 `step_ideology` 的近似 `killer_of`~~ → **已做**（§4c.3）。
- ~~僵尸势力的「夺城—倒戈」振荡~~ → **同回合自相抵消那部分已修**（§4c.4）；
  剩下的多回合「拆平—复垦」循环见 §6。

### Stage C 里剩下的（都是**刻意没做**，各带理由）
- ~~`State.ledger` / `headline()` / `--ledger` / `top_events`~~ → **已做**（§4c.1/4c.2）。
- **salience 权重配置化**：`--digest` 的 `top_events` 目前用**Rust 里声明的** `GameEvent::weight()`
  （穷尽 match）。它是一个**展示排序键**、不是模拟数值，外置成 config 反而会让「新增 variant
  必须声明权重」这条编译期纪律失效。将来真要按剧本调叙事重点，再改成「穷尽 match 读 config」的形式。
- **`cause_id`（事件级因果链显式引用）**：仍**刻意没做**——实测会是 100% null 的死列，而链在
  Python 侧用 `razed.by_ship` / `destroyed.by` / `how+prev_owner` 的结构就能走通。
- **投影体积**：归一化后 `idx/events.jsonl` 比原来的内联事件大约多 50%（列更多，现在还多一列
  `headline`）。3000 回合量级可考虑 parquet（ideas.md §17 候选）。
- **`--digest` 的窗口粒度**：`top_events` 只列 [Milestone]，上限 24 条（`TOP_EVENTS`），
  `skipped` 如实给出。窗口太大时最重的会被「开战/停战」这类阈值抖动占满（见 §6 末尾），
  需要更细的窗口而不是更多条目。

---

## 6. 由账本暴露出来的新问题（值得单独修）

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
`大红斑科学站`（`q.ledger(entity=('city','大红斑科学站'))` 可直接读）：

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

### 6.2 战争的阈值抖动（新观察，未修）

`--digest 20` 的 `top_events` 里（权重最高的就是开战/停战）能看到**同一对势力在一个 20 回合窗口
里开战→停战→开战**好几次（实测 seed 7 @ r40–60：`欧盟↔行星X崇拜教` 三个来回）。`hostile` 是
`relation <= war_threshold` 的**硬阈值**，而关系每回合带随机漂移，于是阈值附近会来回穿越，
每次都记一对 `war_started`/`war_ended`（两回合内不冲突，但多回合反复）。

这不是「同回合抵消」（`war_pairs` 是回合前后各算一次，一对势力一回合最多跃迁一次），而是**缺回滞
（hysteresis）**：标准修法是开战阈值与停战阈值不同（`ceasefire_relation > war_threshold`，
config 里已有 `ceasefire_relation` 字段可以复用）。属机制/平衡设计。

---

## 7. 复现

```bash
cargo run --bin planet_x -- --seed 7 --round 60 --index play/after
cargo run --bin planet_x -- --seed 7 --round 60 --save play/c60.ron
cargo run --bin planet_x -- --start play/c60.ron --round 0 --ledger 20   # 账本活过 checkpoint

python play/_probe_invariant.py play/after/idx/events.jsonl   # 同回合翻转不变量 + headline 完整性
cd play/planet_xq && uv sync
uv run python -c "from planet_xq import load; q=load('../after'); print(q.ledger(limit=10)); print(q.storyboard(50))"

cargo test --lib                          # 68 passed（含 5 个 config 往返 + 8 个投影守卫）
cargo test --test longhorizon             # 6 passed / 8 ignored
cargo test --test longhorizon probe_multipolar -- --ignored --nocapture   # 多极健康报告
```

未跟踪的临时探针（可删）：`play/_probe_sparse{,2,3}.py`（pandas 稀疏字段实测）、
`play/_smoke_history.py`、`play/_probe_stageb.py`（舰存亡对账 + 凶手近似的差异率）、
`play/_golden_compare.py`、`play/_mp_leaders.py`（单极锁死量化）、`play/_probe_invariant.py`
（同回合翻转不变量）、`play/_ledger.py`（`--ledger` 渲染）、`play/_ron_locate.py`（RON 定位）、
`play/hist_probe/`、`play/{baseline,after*,mp_*}`。
