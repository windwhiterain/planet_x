# 稀疏历史 / 事件历史（sparse history / 三层历史）

> 状态 `[x]`（Stage A + B + C + D + E 全部落地） ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §23

> 起因：轨迹答不出「**一个城市易主了，就近是什么事件导致的？被夷平然后被殖民，还是叛乱？**」
> 与「**一艘舰被击毁，是被哪艘舰击毁？**」。完整设计 + 实测见
> **[`sparse-history-design.md`](sparse-history-design.md)**。

**诊断（三个独立根因，按根本性排序）**：
1. **事实根本没被记录**（与 pandas 无关）：`Resurgence` 不带 `city`；难民夺城 `displace_city_for_refugee`
   **零事件**；`ShipDestroyed` 无凶手；欠缴锈蚀复用同一个「被击毁」；剧情赠舰零事件。
   seed 7 r24 活体样本：`大红斑科学站` 被夷平 → 同回合在木星重建（事件不带 city）→ r31 倒戈，
   整条链只能靠 `resurgence.body == city.body_id` 反查。
2. **历史活不过 checkpoint**（与 pandas 无关）：`state.events` 每回合清空，只有 `chronicle` 累计。
3. **enum 被直接序列化成表**：13 个 variant 摊成 23 列并集 → 平均 null **74.8%**、`faction` 角色
   **7 种拼写**、`from`/`to` **一列两义**（`city_defected` 是势力、`capital_relocated` 是天体）。
   → **反证**：把 ③ 修到完美也修不出 ①，没写下来的事实任何表形态都变不出来。

**已落地（Stage A，`[x]`）**：
- `[x]` **归一化投影 API**（`src/model/event.rs`）：`GameEvent::{history_row,kind,salience,participants}`。
  `history_row` 是**穷尽 match** → 新增 variant 时编译器强迫你声明参与方/显著性/载荷，
  **历史不可能被「忘记记录」**（这是本设计的牙齿）。Rust 侧 variant 字段名保持可读，
  归一化只发生在投影层。
- `[x]` **因果字段补齐**：`ShipDestroyed{cause: DeathCause, by: Option<Killer>}`（补刀那一发的
  舰/势力/弹种；`fire()` 里第一发打到 hull≤0 的就是凶手）、`CityRazed{by_ship,damage,pop_before}`、
  `ColonyFounded{how,prev_owner}`、`Resurgence{city}`、`CityDefected/Revolt{loyalty}`、
  新增 `CityOverrun`（难民夺活城）、`ShipSpawned{via}`（剧情赠舰）。
- `[x]` **`events` 变 lazy 表**（`idx/events.jsonl`，`event_id="<round>:<seq>"`）：固定列 +
  统一参与方槽位（`actor_*`/`target_*`/`extra`）+ per-type 载荷收进单个 `data` 对象列
  （**一列只承载一种类型**；不再有同名多义/同角色多名）。主流 `events` → `event_ids`。
  实测平均 null **74.8% → 4.7%**；`variant 专属列 = 0`。
- `[x]` **顺手修 `city_ids` 丢 razed 城**：投影 `city_ids` 过滤 `!razed` 而 `idx/cities.jsonl` 写全部，
  导致 `q.join('cities')` 静默丢掉**被夷平的城**——偏偏那是历史最关心的实体。改为逐行一致。
- `[x]` **Python kit**：`q.events()/actors()/history(kind,id)/cause(kind,id)/fates()/audit()`。
  `actors()` 从三类槽位**通用展开**（无 variant 知识）→ 任意实体同一个查询形状。实测 60,775 事件下
  单实体历史 0.84 ms。**先按类型取**（`q.events(type=…)` 字段稠密）；长表「缺失 = 没有那一行」，
  所以稀疏字段统计是 `groupby().size()` 一行的事（宽表 64% NaN 的坑全部规避）。
- `[x]` **完备性守卫** `every_city_state_change_is_explained_by_an_event`：投影 120 回合，逐回合对比
  密集快照，任何 `(faction_id, razed)` 变化都必须有命名该城的事件解释（实测 **145 次全有解释**），
  并断言 `checked >= 5`（**守卫必须非空**）。另 `event_milestones_is_deterministic`。
  `q.audit()` 把同一不变量暴露给 agent。
- `[x]` `SCHEMA_VERSION` 1→2（`GameEvent` 是 `State` 的一部分）；`main.rs::event_counts` 删掉
  手抄的 variant→label `match`，改走单一权威 `kind()`。
- 验证：`cargo test --lib` **55 passed**（+4 投影守卫）、`cargo test --test longhorizon` **6 passed/8 ignored**
  （`same_seed_reproduces_identically`、`world_is_multipolar` 全绿 → 平衡未动）、seed 7 @ 60 端到端
  `q.cause('ship','星环')` → `killer=天工/中国, weapon=kinetic, assists=[镇岳,长城]`；`q.audit()` = 0。

**未做（Stage B，`[x]` —— 已落地，见下）**：

**已落地（Stage B：漏斗化 + 对账，**可证明行为中性**）**：
- `[x]` **状态变更漏斗（single writer）**：`sim.rs` 里**每一处**归属/存亡写入现在都在漏斗内，无例外——
  `kill_ship`（hull 归零 + 记 `ShipDestroyed`，同舰只记一次）、`sweep_dead_ships`（清扫 + **兜底补事件** + 清指令）、
  `spawn_ship`（装配/取名/面板/付组件费 + 记 `ShipSpawned`）、`raze_city`（清人口/建筑/进度 + `CityRazed`/`Revolt`）、
  `reseed_city`（razed→活城 + `ColonyFounded{Refounded, prev_owner}`）、`found_city`（新建 + `ColonyFounded{NewSite}`）、
  `overrun_city`（夺活城 + `CityOverrun`）、`defect_city`（换主 + 迁控制叶子 + `CityDefected`）、`wire_city_control`。
  「忘记记事件」从此在**结构上**不可能：改状态与记事件在同一处。
  - 兜底：`sweep_dead_ships` 带 `debug_assert_eq!(invented, 0)`——正常 0 艘需兜底，不为 0 = 某条路径漏了
    `kill_ship`，测试当场炸；release 仍用最保守的 `Scrapped` 补一条（历史完整但不谎称战损）。
- `[x]` **对账扩到「舰的存亡」——当场抓出第二类漏洞**：`every_ship_state_change_is_explained_by_an_event`
  一上线就发现 `step_resurgence` 的种子舰**完全不发造舰事件**（实测 63 次出生里 **46 次无解释**），
  与「城易主查不到原因」完全同源，只是藏在舰那一侧；修法即 `spawn_ship` 漏斗。
  实测 120 回合：**242 次舰死亡 / 249 次舰出生 / 145 次城变化，全部有事件解释**；两条守卫都断言「检查数 ≥ 5」。
- `[x]` `q.changes(kind, id)`：纯 dense-diff 视图（与事件历史互证；舰还会显式给出「消失的那一回合」）。
- `[x]` **行为中性的证明方法（可复用）**：改 `sim.rs` 后不靠「跑一遍看着对」，而是 **golden-file 对比**——
  `python play/_golden_compare.py <baseline> <after>`：`idx/{cities,ships,factions,bodies,settlements}.jsonl` +
  `meta.json` 必须**逐字节一致**，`main.jsonl` 去掉 `event_ids` 后必须一致，`idx/events.jsonl` 允许不同。
  **Stage B 全程通过**：漏斗化只多了 46 条 `ship_spawned` 记录，模拟逐字节未变。
- `[x]` **~~（负结果，勿重复尝试）~~ `step_ideology` 改用权威 `by`** —— **已做，并且当初的「负结果」结论是错的**。
  当初只改「凶手」一侧就回退；Stage C 复查发现同一段代码里还有**第二处同类缺陷**（失城方回读，见下），
  两处一起修之后长局不但没锁死、反而更健康（seed 1 轮换数 **1 → 25**）。教训：**「60 回合逐字节一致」
  不足以证明长局中性**（窗口内中性 + 长局守卫失败 = 行为改动），**但「长局守卫失败」也不等于「这是纯
  平衡调整」**——先问「同一段逻辑里是不是还有别的缺陷没修」。半个修正的症状和平衡回归一模一样。

**已落地（Stage C，`[x]`，见 `sparse-history-design.md` §4c）**：
- `[x]` **`State.milestones` 长存里程碑层**（`src/model/event.rs` 的 `Milestones`/`MilestoneEntry`，
  `SCHEMA_VERSION` 2→3）：只收 `Salience::Milestone`（由 `salience()` 单点声明），**不随回合清空**
  → 解决根因 ②。写入点是唯一的发事件漏斗 `sim::ev` → 「事件发了、里程碑没记」结构上不可能。
  `--milestones [N]` 输出（带 `complete/dropped/dropped_through_round`，截断可见）；
  容量由 `config/game.ron` 的 `history.max_milestones` 控制（**默认 0 = 无损**）。
  **实测体积**：round 60 的 checkpoint 106 KB，里程碑占 41%（94 字符/条 × 7.7 条/回合）
  → 1000 回合约 0.7 MB；超长归档局可设上限。
- `[x]` **一句话 headline**：`GameEvent::headline()`（穷尽 match）**自足**（只读事件自身字段，
  不回查 state——归档历史里的实体可能早就没了）、**单行**，且 `participants()` 的每个 id
  都**逐字出现**在句子里（守卫 `headline_names_every_participant`）。同一句话出现在
  CLI `--milestones`/`--digest top_events`、投影 `idx/events.jsonl` 的 `headline` 列、Python `q.milestones()`。
- `[x]` **投影事件行改由 `EventRow` 序列化生成**（此前手写 `json!`，加列会悄悄漏——`headline` 就这么差点漏掉）。
- `[x]` **`--digest` 加窗口 `top_events`**：窗口内里程碑按 `GameEvent::weight()`（穷尽 match 的
  **展示排序键**，刻意不外置 config——外置会让「新增 variant 必须声明权重」这条编译期纪律失效）
  取最重 24 条，**展示按时间序**，并如实给出 `total/shown/skipped`。
- `[x]` Python kit：`q.milestones(since/until/limit/entity)`、`q.storyboard(window)`；
  `q.events()` 多一列 `headline`。
- `[x]` **三处「事后回读」的根治**（`step_ideology`）：① 凶手用权威 `by`（互杀不再吞掉战功）；
  ② **新增 `CityRazed.owner`**（失城方只有夷平那一刻才知道——同回合复垦会把 `faction_id` 改成新主）；
  ③ `CityDefected`（主路）与 `Revolt`（兜底）同分（旧代码给兜底 −1、主路 0 分，分数取决于
  「有没有可倒戈的势力」这个无关偶然）。信号抽成**纯函数** `military_deltas(&[GameEvent])`
  （只吃事件、不看 state）→ 可被单测逐条钉死。`colony_founded` **刻意不计**（殖民归 `nature_colony` 轴）。
- `[x]` **同回合归属翻转不变量**：`displace_city_for_refugee(state, exclude)` 排除本回合已易主过的城
  → `city_overrun`（seed 7 @ 60 回合）**41 → 20**；守卫 `no_city_changes_owner_twice_in_one_round`。
  （`city_razed`→`colony_founded` 同回合不算违规：中间经过了「死亡」态，是两件真实的事。）
- `[x]` **顺带修掉一个预存重大缺陷：RON 读不回 checkpoint**（`--save`/`--start` 全断）。
  根因：**单元 enum 嵌在内部标签 enum（`#[serde(tag)]`）里**时，serde 默认把单元变体写成**裸标识符**，
  RON 的 `deserialize_any` 无法还原 → 整份 `State` 反序列化失败。`DeathCause`/`SpawnVia`/`FoundingHow`
  是 Stage A 新增的，所以**是 Stage A 引入的**；而 `--save`/`--start` 是文档里的核心流程却**零测试**，
  `load_initial` 还把解析错误静默咽掉、报成一句指向文件末尾的「missing field `round` in `State`」。
  修法：`stringly_unit_enum!` 宏 + `#[serde(into="String", try_from="String")]`（**JSON 形状一字节不变**）。
  守卫：`every_game_event_variant_round_trips_through_ron`（**逐个变体**，配穷尽 match 的
  `variant_checklist` 提醒补样本）、`checkpoint_survives_save_and_resume_identically`（存→读→续跑逐字节一致）。
- 验证：`cargo test --lib` **68 passed**、`cargo test --test longhorizon` **6 passed/8 ignored**、无警告。
  `probe_multipolar`（1000 回合）：seed 1/42/12345 → rotations **25 / 80 / 83**、alive 9、zombies 0。

**未做（仍是 `[ ]`，各带理由）**：
- `[ ]` **salience 权重配置化**：`top_events` 的排序键 `GameEvent::weight()` 刻意留在 Rust
  （它是**展示**排序、不是模拟数值；外置 config 会让「新增 variant 必须声明权重」失效）。
  真要按剧本调叙事重点，再改成「穷尽 match 读 config」。
- `[ ]` **`cause_id` 显式因果链刻意没做**：实测会是 100% null 的死列，而链在 Python 侧用
  `razed.by_ship`/`destroyed.by`/`how`+`prev_owner` 的结构就能走通。
- `[ ]` 投影体积：`idx/events.jsonl` 比原内联事件大约多 50%（列更多，现在还多 `headline`）。
  3000 回合量级可考虑 parquet。

**里程碑暴露出的新问题（`[ ]` 值得单独修）**：
- `[~]` **僵尸势力的「夺城—倒戈」振荡**：**同回合自相抵消那部分已修**（41 → 20 条 `city_overrun`，
  见上）。**剩下的是多回合循环**：`reseed_city` 每次都挑该势力**自己最低名的空白城**，于是
  「欧盟拆平 → 联合国复垦 → 欧盟再拆平」在同一处反复（seed 7 的 `大红斑科学站`：
  `q.milestones(entity=('city','大红斑科学站'))` 可直接读整条链）。这不是净零动作（每次复垦都真的重建
  人口/建筑），而是**反僵尸机制**与**舰炮拆城**互相咬住。改法候选：① 复垦锚点排除「最近 N 回合内
  被拆平过」的城；② `resurgence` 加冷却；③ 重建优先选**别人**的废墟（现在只挑自己的 diaspora claim）。
  **注意**会影响 `zombie_factions_are_bounded`/`world_is_multipolar`，需长局验证。
- `[ ]` **战争的阈值抖动（新观察）**：`--digest 20` 里能看到同一对势力在一个 20 回合窗口内
  开战→停战→开战好几个来回（seed 7 @ r40–60：`欧盟↔行星X崇拜教` 三个来回）。`hostile` 是
  `relation <= war_threshold` 的**硬阈值**，关系每回合带随机漂移 → 阈值附近来回穿越。
  不是同回合抵消（一对势力一回合最多跃迁一次），而是**缺回滞（hysteresis）**：
  标准修法是开战/停战用不同阈值（config 里已有 `ceasefire_relation` 可复用）。

---

**已落地（Stage D，`[x]`）—— 分层判据从「重要性」换成「后续计算的访问需求」**：

> 这一版**推翻了 Stage C 的前提**。Stage C 为了让 checkpoint 自述历史，把「听起来重要」的
> 14 个 variant 预填进 `State::milestones`；实测那**吃掉存档 67%**（r200：249,557 / 372,248
> 字节，1860 条 ≈ 9.3 条/回合），而且与「后续计算要不要回看它」毫无关系。

判据（写进 `Salience` 三个 variant 的文档，文档即规格）：
- **`Milestone`**：后续计算需要访问**无限的过去历史** → 长存进 `State::milestones`，随 checkpoint 存活。
- **`Notable`**：后续计算需要访问**一定的事件窗口** → 只需一个有界窗口在 state 里（`State::notables`）。
- **`Detail`**：后续计算**只需要前一帧**，或**根本没有读者**——只为 agent 事后分析而记录。

- `[x]` **读者盘点**（写进 `GameEvent::salience` 的文档，作为定级依据的唯一来源）：非测试的
  `state.events` / `state.notables` 读者**全部只读当前回合**——`step_ideology` 的
  `military_deltas`、`kill_ship` 的同回合去重、`step_diplomacy` 的「本回合谁和谁交火」、
  `step_story` 的载荷提取 + `StoryTrigger::*`、`tactics` 的 `Withdraw` 判定、`agent.rs` /
  `projection.rs` 的渲染。**没有任何逻辑读无限过去。**
- `[x]` **里程碑层清空**（14 → **0** 个 variant）。仅有的无限过去需求（`first_war` / `first_raze` /
  `first_colony` 的「史上第一次」）**已经有载体**：`state.chronicle` 的 `id` 去重——历史层不必为
  它们长存。字段保留（`Milestones`）作为将来真出现无限过去读者时的家，**刻意不预填**。
  更一般的教训：**真正的无限过去需求通常该被折成一个 state 标量（累加器），而不是回放原始历史**
  ——`step_ideology` 就是这么做的（每回合只读当前帧，累加进 `ideology` 标量）。
- `[x]` **新增窗口层** `State::notables: Notables`（与 `Milestones` 共用 `HistoryEntry{round,event}`）
  + `config.history.notable_window`（默认 **24** 回合 = 2 年）。裁剪区间 `[round-window+1, round]`
  ——**本回合仍在窗口内**（`window = 1` = 只看本回合，不是空表）。**过期不记 `dropped`**：过期是
  设计而非丢失（与里程碑层「截断可见」刻意相反，两者都在文档里写明了为什么）。
- `[x]` **`WarStarted` / `WarEnded` → `Notable`**，依据是「记恨」这个窗口读者（见下）。各层 `push()`
  自己过滤，`ev()` 仍是**唯一漏斗**（现在一次写三层）。
- `[x]` **记恨（战争疤痕）**：`sim::war_scar_floor()` 回头看窗口内**最近一次** `WarStarted{a,b}`
  （无序匹配），返回一道从 `war_scar_relation`（-30）**线性衰减到 0** 的关系地板。新鲜时
  -30 < `war_threshold`(-20) → **刚开战的对手不可能当回合言和**；地板在第
  `war_scar_rounds*(1-20/30)` = **9** 回合抬过阈值 → 和平重新可能。「记恨，但会淡」。
  实测（seed 7）：开战/停战 r60 **65/63 → 32/29**、r200 **93/91 → 59/57**；最短战争从「闪烁」
  变成**恰好 9 回合**（55 场里 30 场 = 9）。
- `[x]` **修一个真 bug：地板被别的写入者绕过。** 最初地板只套在 `step_diplomacy` 的漂移里，而关系
  有**多个**写入者——`step_balance_of_power` 的「合纵」（弱者相互靠拢，跑在 `step_diplomacy`
  **之后**）把两个正彼此交战的弱者拉近，于是实测最短战争只有 **6** 回合（3 场），直接推翻了地板
  的承诺。修法不是放宽断言，而是把地板放进**关系写入的唯一漏斗**（`set_relation_sym` +
  `adjust_relation`）——与 `ev()` / `kill_ship()` 那套 single-writer 纪律同源。修正后 55 场战争
  `min = 9`，零违例。**教训：一条「地板 / 不变量」如果有多个写入者，它就不是不变量。**
- `[x]` **投影新增 `weight` 列**（`GameEvent::weight`，纯显示排序键）。`salience` 回答「谁要回看它」，
  `weight` 回答「人读起来重不重要」——**两者刻意分开**，因为混用会让「挑值得读的事件」在里程碑层
  清空后**静默变空**。`--digest top_events` 与 `q.storyboard()` 因此改按 `weight` 挑（`storyboard`
  新增 `min_weight=8` 门槛），不再按 `salience` 过滤；`--schema` 的 `column_docs` 显式写明这条区别。
  **`weight` 是 0–9 的序数阶梯，不是 0–100 分数**——门槛一度被写成 60（照「分数」的错觉），
  于是 `q.storyboard()` **静默返回空表**（测试全绿）。已加守卫
  `weight_ladder_stays_a_documented_zero_to_nine_scale` 把量纲钉住。
- `[x]` CLI `--notables [N]` 新增；`--milestones [N]` 保留但文档写明「通常 `count: 0`，这就是判据的
  正确结果」。`SCHEMA_VERSION` 3→4（**只升版本号**：v3 档里的里程碑是按旧判据预填的残留，不是任何
  计算回看的历史；agent 要那些历史本来就走投影）。
- 验证：`cargo check --workspace --all-targets` 无警告；`cargo test --lib` **72 passed**（新增
  `history_layers_are_assigned_by_reader_need_not_importance`（**守卫里程碑层为空**——谁凭「听起来
  重要」把它填回去就会红）、`milestones_trim_reports_truncation_visibly`、
  `notables_keep_exactly_the_window`、`each_layer_only_takes_its_own_salience`、
  `war_scar_floor_makes_a_real_floor_on_war_duration`（**用 60 回合真实长局验证最短战争时长**，
  而不是只测函数）；`longhorizon` 6 passed / 8 ignored；`probe_multipolar` 三 seed 均 9 势力存活 /
  0 僵尸 / 无霸权失控（rotations 47 / 71 / 90）。**r200 存档 372,248 → 108,889 字节（-71%）**。

- `[ ]` **（留给以后）** 窗口层的下一个候选读者：`city_razed` / `city_defected` / `city_overrun`
  ——「刚丢掉的城，旧主想夺回 / 民心不稳」是一个很自然的窗口机制。**先有读者，再提升定级**，
  别反过来（那正是 Stage C 犯的错）。
- `[x]` **`resurgence` 的 churn 循环**（曾经的候选窗口读者）——**已做**，见下方 **Stage E**。
  结果值得记一笔：**修它完全没用到窗口层**（只需要本回合的 `flipped_this_round`），所以
  `CityRazed` **仍然留在 `Detail`**。这是判据（「先有读者再提升定级」）第二次拦下一个想当然的
  提升——第一次是战争（那次是真的有窗口读者）。
- `[ ]` **（留给以后）** `ship_destroyed` 里大量 `cause: upkeep_shortfall`（欠维护报废，**不是战死**）
  ——「势力重建时白送一艘自己养不起的种子舰」的经济侧问题，与几何抵消无关。
- `[ ]` **（留给以后）** 战争**进入**仍无迟滞：地板只保证「最短 9 回合」，`hostile` 还是硬阈值。
  若要做「宣战需要更长的敌对积累」，那是 `Notable` 窗口的又一个读者（读最近 W 回合的
  `attack`/`siege` 密度）。config 里已有 `ceasefire_relation` 可复用。
- `[ ]` **（留给以后）** `history.max_milestones` 现在没有意义（层是空的）。等真有无限过去读者
  进来时再定默认值；**不要提前设上限**——那是把问题藏起来而不是解决。
**已落地（Stage E，`[x]`）—— 拆平/复垦的极限环：把「同回合抵消」不变量补齐到 anchor 1/2**

> 起因：Stage D 在清空里程碑层时读到 r196/198/200 **一字不改地重复**的四连
> （拆平 → 同回合复垦 → 白送种子舰 → `resurgence`）。量化后它是当时最大的单一事件来源。

- `[x]` **量化**（seed 7 @200 回合，修正前）：137 次 `city_razed` 里 **93 次（68%）是「同回合被
  同一势力复垦」**——归属 A→A，净变化只剩「人口/建筑被重置」，却记 `city_razed` +
  `colony_founded` + `ship_spawned` + `resurgence` **四条事件**、还白送一艘种子舰；只有 6 次是
  别人复垦（正当的）。单城循环 `水星熔炉基地` **22 次**（被拆平 **32 次**）、`长三角` 18、
  `珠三角` 17；「复垦→再被拆平」的间隔集中在 **1–2 回合**（27 + 53 次）——这是**极限环**，不是战争。
- `[x]` **根因不是缺机制，而是既有不变量的应用不完整。** `displace_city_for_refugee` 的注释早就
  写明这类缺陷：「两个步进在同一回合里正好互相抵消（净效果为零，却照样记两条里程碑事件、还白造
  一艘种子舰）」——但那次只为 **anchor 4（难民夺城）** 加了 `flipped_this_round` 过滤，
  **anchor 1/2（自己的废墟 / 任何空白避风港）漏掉了**。
- `[x]` **第一次试错（值得记，因为它把设计打坏了）**：按「跳过这个势力、让拆平的后果站住一回合」
  实现（`continue`）→ `longhorizon` **挂 2 条**，`probe_multipolar` 出现 **zombies 6/6/5**
  （原本 0/0/0）、存活势力 9→7。原因：`zombie_count` 是**整局采样到的峰值**，而 `continue` 让被
  拆平的势力**在本回合末真的没有立足点**，采样点正好抓到它。
  **结论：反僵尸保证的是「回合末总有立足点」，不能被这条过滤破坏。**
- `[x]` **正确修法（与 anchor 4 完全同构）**：排除那块刚丢的废墟，**改在别的立足点重建**
  （自己的另一块废墟 → 任何空白避风港 → 未占据定居点新建）。于是既没有净效果为零的同回合抵消，
  也没有任何一个回合末留下无立足点的势力。
- `[x]` **顺带抓出一个预存的洞**：`flipped_this_round` 的**种子集合与它保护的不变量定义不一致**
  ——守卫 `no_city_changes_owner_twice_in_one_round` 把「活城易主」定义为
  `city_defected / city_overrun / **colony_founded**` 三者之和，而种子只收了
  `{defected, overrun, razed}`，**漏了 `ColonyFounded`**。于是**本回合刚被殖民舰复垦的城**在
  anchor 4 眼里不算「已易主」，可以被同回合的难民夺走：实测回合 76/79 的 `大红斑科学站`
  ——「深空运输联盟 复垦 俄罗斯 留下的废墟」之后同回合又被「联合国的难民夺取」。它**潜伏**在那儿，
  是这次改动了 anchor 1/2 的选择顺序才被踩到。补进种子集合即修好。
  **教训：一条不变量的「判定集合」和「保护集合」必须是同一个定义——否则它只在没人踩的时候成立。**
- 实测（seed 7 @200 回合）：总事件 **2273 → 1848（-19%）**；`city_razed` **137 → 60（-56%）**；
  同回合自我复垦 **93 → 0**；被拆平 ≥5 次的城 **10 → 5**；最严重的城 **32 → 6**。
  `city_overrun` 46 → **90**：抵消被换成了**迁往别的立足点**（anchor 4 同构的代价，符合既有设计）。
- 长局（`probe_multipolar`，1000 回合）：三 seed 全部 **alive 9 / zombies 0**；
  rotations 47/71/90 → **52/101/118**（轮替更快）；terminal_top 0.377/0.656/0.665 →
  **0.424/0.375/0.501**（霸权更弱）；gini 0.455/0.505/0.505 → **0.455/0.434/0.404**（更均衡）。
  `probe_zombies`：「never reached 3 zombies」。
- 新增守卫 `a_city_razed_this_round_is_not_refounded_by_its_own_loser_this_round`：**120 回合真实
  长局**里断言该不变量，并要求 `razings >= 20`（守卫不许空转）。lib **74 passed**；
  longhorizon 6 passed / 8 ignored。
- `[ ]` **（留给以后）** 剩下的 60 次拆平里仍有少量 1–2 回合间隔，但最大单城已降到 6 次，属真实
  围城/拉锯的量级，**不再是极限环**。若还要压，问题在「围城方为什么原地不走」
  （`nearest_hostile_city_in_siege_range` 只避开 `razed` 城），不在复垦侧。
