# 《行星X》点子库（Ideas Backlog）

> **给未来 agent 的工作约定（先读这段）**
>
> 这是一个**活**的入库文档，不是一次性规划。请遵照：
>
> 1. **边实现边想点子**：在实现本轮目标的过程中，脑中冒出的新机制/新平衡/新剧情，只要
>    值得做，就**当回合先尝试**；做成验证编译通过、长局 harness 不崩、且确有增益的改动。
> 2. **来不及实现的不要丢**：任何想到了但**这一回合来不及做完/验证**的点子，一律**写进
>    本文档**（追加到对应分类，写得够细，让下一个 agent 能直接照着实现）。宁可写下来，
>    也不要让它在对话里蒸发。
> 3. **实现过的要勾掉**：某条点子一旦落地并提交，把它的状态标成 `[x]`（done），并在
>    括号里注明提交 hash / 所在模块，避免重复劳动。
> 4. **优先级靠前、可独立验证的先做**：优先挑那些能立刻被 `tests/longhorizon.rs` 或单元
>    测试量化的点子（例如「减少区域性霸权」「降低永久观众」「扩大多极参与」），因为它们
>    可以直接用数据证明「游戏在上千回合后依然有意义、不崩坏」。
>
> 主线目标：**让 AI(你) 在《行星X》里玩得有意义，且游戏在上千回合后依然是多方参与的博弈，
> 而不是退化成「少数永久霸权 + 一堆永久旁观者」，经济既不无限膨胀也不塌缩。**

---

## 状态图例

- `[x]` 已实现并提交
- `[ ]` 未实现（候选点子）
- `[~]` 已部分实现 / 已实现但效果未达预期，待进一步调校

---

## 1. 治理与忠诚度（光速管制 / 管理难度 / 人口分散） — `[~]`

已落地（`governance` 配置 + `loyalty_budget` 可控叶子，见 `src/sim.rs`、`src/model.rs`）：
治理开销与忠诚度随「距离首都 × 人口超载」叠加；付不起则离心叛乱（`Revolt`），城市夷平为空白。
但**世界仍偏向「区域性霸权 + 少量 1 城势力」**，尚未真正多极。

- `[ ]` **忠诚度驱动的「改旗易帜」**：当前叛乱只是把城夷平为空；可改为让低忠诚城市**倒戈到
  邻近/更会搞福利的势力**（或被相邻的和平势力吸收），而不是白白变成空地。增加外部因素的
  拉拢（福利、意识形态），让弱势力不靠军事也能「接盘」失地。
- `[ ]` **忠诚度影响生产效率**：低忠诚城市生产效率打折（罢工/怠工），高忠诚城市可能有加成。
  让「安抚居民」有直接经济收益，而不只是防叛乱。
- `[ ]` **行省/总督**：治理开销不再「逐城」，而是按「距首都所在行省」粗粒度计费；玩家可为
  某行省任命总督（一个可控叶子）提升该省忠诚。降低指令粒度，让长局治理更省心。
- `[ ]` **叛乱者建国**：叛乱规模够大时，叛军独立成新势力（利用 `Faction` 的 id 池），而不是
  只把城夷平。这让「帝国崩解」更戏剧化，也让世界天然保持多极。

## 2. 多极与霸权制衡 — `[~]`

已落地一版**合纵连横 / 均势外交**（`balance` 配置 + `sim::step_balance_of_power`，见
`src/sim.rs` / `src/model.rs` / `config/game.ron`）：综合实力占比达 `hegemon_power` 的
势力被判定为「霸权」，其余较弱势力——弱者-弱者向 `coalition_affinity` 靠拢（合纵）、
弱者对霸权被「遏制」（向 `hegemon_affinity` 下压、不无脑宣战）、霸权开打任一弱者触发
集体安全（其余弱者关系骤降）、被封锁的霸权经济制裁（自动市场额度缩水到
`sanction_trade_mult` + 维持帝国的治理/娱乐成本放大到 `sanction_cost_mult`——被孤立的大国
养边地更贵、远端殖民地更易离心）。带来 `CoalitionFormed`/`CoalitionEnded` 事件，并把当前
格局（`hegemon`/`members`/`power_share`）暴露给 agent 的 `coalition` 字段与 `meta` 的
`balance`。harness 加了 `tests::world_is_multipolar`（不统一：单一势力城占峰值 < 0.85 +
长局出现过反制联盟 + 后半程最强势力 ≥2 个会轮换）。

- `[x]` **联盟/防御协定 + 均势干预 + 经济制裁**（已落地，`src/sim.rs::step_balance_of_power`）：
  弱者对霸权抱团（合纵）、遏制（冷战式疏远，不无脑宣战）、集体安全（霸权开打任一弱者
  → 其余群起而攻之）、经济制裁。较旧想法更拟真：不是「关系好就入盟」的一刀切，而是
  基于**共同威胁**的渐进抱团。
- `[x]` **经济制裁落到「治理/维持能力」**（`sanction_cost_mult`，`src/sim.rs::sanction_cost_mult`
  接入 `step_governance`）：被反制联盟锁定的霸权，其维持帝国（行政+娱乐）成本按该倍率
  放大——孤立的大国要花更多资源养边地、治安成本上升，远端/边缘殖民地更难养、更易离心
  叛乱，体量自然回落。1200 回合观察：后半程最强势力平均城占从 ~0.55 降到 ~0.51，且
  seed 42 由「锁死同一霸主」变为「4 个不同最强势力轮换」，同时 max_dead 保持 ≤2。
- `[x]` **制裁与联盟解耦（`sanctioned_hegemon`，`src/sim.rs`）**：制裁不再等联盟**完全成形**
  （≥`min_members`）才启动，而是——只要某势力**已称霸（实力占比达标）且至少有一个弱者
  倒向联盟**（关系 ≤ `coalition_estrange`）就实施封锁。这使「人缘好但已坐大」的紧凑区域
  帝国也能被压缩（否则它不招人恨就没人封锁它），比 war 更拟真、不引发夷平。实测（1200 回合，
  `sanction_cost_mult=1.45`）：后半程最强势力平均城占降到 **~0.45–0.53**（seed42/12345 <0.5），
  峰值 ~0.68–0.73，guard 种子 seed1/42 的 max_dead 降到 **1**。
  ⚠️ 注意：seed 12345 在该激进档会短暂出现 **3 个僵尸**（治理叛乱+再殖民连锁，恰逢世界
  短暂无空白定居点、`step_resurgence` 无法重建）——这是**重建缺口**（见「当前已知问题」），
  不是设计目标。收紧到 ≤2 就要把 `sanction_cost_mult` 降到 ~1.2，会牺牲平均城占的收益。
- `[ ]` **重建缺口（关键下一项）**：`step_resurgence` 在「无自身残骸足迹 且 全系统无空白
  定居点」时会放弃重建该势力（被完全吞并则合法出局）。但激进制裁下的连锁叛乱/再殖民会
  让世界**短暂全满**，导致多个势力同时无法重建（seed 12345 出现过 max_dead=3）。应保证
  **总能给被消灭的势力一个立足点**（例如：保留其最近一座城的空白足迹直到它重建，或当全
  世界无空白时强制在最低 id 定居点腾出一个难民收容所），把「上千回合不崩坏」这一硬指标
  对所有种子夯实。
- `[ ]` **霸权疲劳 / 厌战螺旋**：当某势力控制定居点占比超过阈值（如 55%）时，国内士气/
  忠诚/生产走低，产出一个「高处不胜寒」的惩罚，逼其体量回落到稳定区间。当前已用
  「制衡 + 治理超载 + 制裁治理代价」间接实现，但**瞬时峰值仍可达 ~0.73**（那是霸权被
  围剿前的顶点，会回落）——与「稳定极值较低」的差距可再压。
- `[ ]` **多极硬指标再收紧**：`world_is_multipolar` 现断言「不统一 (<0.85) + 联盟出现过 +
  最强势力≥2 个会轮换」。更硬可继续断言「任意一方城市占比长期均值 < 0.5」；当前后半程
  均值已到 ~0.45–0.53，可结合上面的「重建缺口」修复后收紧到 <0.5。
- `[~]` **研究发现：瞬时峰值的成因是「紧凑区域帝国」**（`probe_peak` 观测）：最强势力在峰值
  时拥有 ~16 城，其中 **13–14 城都在距其首都 18 AU 内**（距离分布 [0 .. ≈36 AU]，仅 2–3
  城较远）。所以「距离衰减/过度扩张」只夷平那 2–3 个远城，**碾不垮紧凑的核心区**——要
  真正压低峰值需要(a)联军主动军事制衡（有僵尸风险）或(b)让区域本身不被一家吃下（AI
  扩张/地图聚类）。这是「上万回合依然多极」的深层难点。
- `[x]` **抱团的牙齿 / 联盟舰队协同（防御性）**（`sim::coalition_war_focus` 接
  `nearest_enemy_ship`/`pick_target`）：结盟势力在霸权先动手（集体防御触发）时**优先集火
  该霸权**、而非各自就近乱打，使集体防御真的打得赢。每回合对各势力各算一次焦点
  （O(势力×实体) 摊薄到全军，无明显性能开销）；全部确定性。注意：遏制模型下弱势力通常
  不与霸权主动开战，故该协同只在**集体防御战争**里激活——属于「守得住」的保障，而非
  压低单极份额的主因（压低主因是经济/治理制裁）。
- `[~]` **正义/威慑性（主动）协同**：上述集火是被动的（霸权先动手才触发）。若要让弱者
  联盟**主动**把一家独大的霸权拉下马（进一步压低瞬时峰值 ~0.73），可在遏制基础上让
  联盟在霸权实力占比超阈值时也渐进地军事施压——但需权衡僵尸数（一轮验证显示过强会致
  大量势力被夷平为僵尸）。当前先维持「防御协同 + 经济/治理制裁」的稳妥组合。
- `[ ]` **超载/过度扩张更咬人**：治理的 `population_capacity`/`loyalty_distance` 目前仍
  让一个富矿势力在合理距离内积累大片领地而不叛乱。可调低容量/加大距离衰减，或让**超载**
  按「城数 × 每城人口」更快触发，使大帝国周边殖民地更易离心——这比靠战争更拟真地
  遏制「区域性霸权」。

## 3. 行星X 本体：回归天体（长周期天文事件） — `[ ]`

现在「行星X」只是个 round-60 的故事节拍。它是本作命名核心，值得做成**真实的长周期天体**。

- `[ ]` **新增第 19 号天体（行星X）**：极偏心率、超长公转周期（数千月）。平时远在柯伊伯带
  之外，周期性扫入内太阳系。它**不是任何一个势力的城**，而是一个「世界级事件源」。
- `[ ]]` **回归效应**：当行星X 进入内太阳系（近日点附近）时，触发一轮全球性震荡——
  * 各势力关系向「共同威胁」靠拢（暂时化敌为友），结束后又恢复敌对；
  * 沿途扰动脉冲影响轨道/导航（与 MOND 异常叠加）；
  * 剧情编年史上每回归一次就多一章「行星X 纪元」。
- `[ ]` **数据驱动**：`config/game.ron` 新增 `planet_x`（轨道、回归间隔、效应强度），让
  agent 可调，无需改代码。
- `[ ]` **harness**：长局里断言「行星X 至少回归 X 次」「每次回归都产生全球性事件」，保证
  「行星X」不是一次性噱头，而是贯穿上千回合的主线。
- `[ ]` **实盘诊断（seed 7 @ `--traj 60`）**：剧情弧恰好在第 60 回合以「行星X 现身」收尾，
  但该节拍**纯属散文**——轨迹里它只写入 `chronicle`，对关系/轨道/经济/市场**零机械影响**
  （`State` 无第 19 号天体、无回归效应）。这印证了「把行星X 做成真实长周期天体 + 回归效应」
  的必要性：目前这个**命名核心的高潮在数值上没有任何落点**，文案说「太阳系已不再是以往那个
  太阳系」，数值却纹丝不动。

## 4. MOND / 柯伊伯引力异常 — `[~]`

已实现：非 master 势力在异常区内导航偏移（`mond` 配置 + `sim::mond_drift`），cult（`masters`）
指哪打哪。这是「cult 被围攻如何自保」的答案。可再加深：

- `[ ]` **异常区内的战斗光环**：非 master 舰在异常区内不仅偏航，开火命中/伤害也打折（被
  异常引力干扰测距），让 cult 在圣所附近更难被消耗。
- `[ ]` **MOND 对经济/开采的影响**：异常区内的天体矿藏（深空稀有矿产）产出加成——那里
  本就是「淘金热」地，但没 MOND 的势力开采用不了这些红利，形成「深空高地只有 cult 和
  master 能吃下」的经济不对称。
- `[ ]` **MOND 科技扩散**：其他势力可通过研究/剧情事件「习得」MOND（把某势力加入
  `masters`），把「技术特权」变成一条可争夺的叙事线。

## 5. 时代 / 科技演进（上千回合不漏气） — `[ ]`

- `[ ]` **解锁式舰级**：开局只造得出 护卫/驱逐，随时间/资源积累解锁 巡洋/航空母舰/战列舰。
  让上千回合有「时代的步伐」，而非静态阵容。
- `[ ]` **结构/材料演进**：从混凝土 → 钢结构 → 更高级结构（护甲/造价曲线），给建筑一个
  升级弧线。
- `[ ]` **设计图/舰种分支**：玩家（agent）可以为舰船选择「专精」路线（如速度型、装甲型），
  影响战斗权衡。

## 6. 经济深度 — `[~]`

- `[ ]` **有限矿藏（耗竭）**：目前开采是「面积×速率」无限产出，是经济可无限膨胀的根源。
  让矿藏有 `remaining`（总可采量），采完即枯竭，逼势力不断向外殖民才维持产出——短中期
  治「runaway」，但要注意别搞成「热寂」（全都枯竭 → 僵死）。需与第 3 条（行星X 引入新
  资源）配合才能长期不僵。
- `[ ]` **贸易路线/运力**：市场目前是无成本「瞬间兑换」。可把运费做成距离函数（运到深空
  更贵），与 MOND/导航联动。让「深空物质」真的难得。
- `[ ]` **稀缺与景气**：用 `PseudoRandom` 长周期驱动资源价格/产量波动（矿脉发现、需求变化），
  制造上千回合的经济景气/萧条循环，避免一条水平线。

## 7. 剧情 / 编年史深化 — `[~]`

- `[ ]` **事件型机械后果扩展**：`StoryEffect` 目前有 关系/资源/赠舰。可加：
  - `RazeCity(city)`（叙事强拆）、`GrantStructure(faction, kind, area)`（赠建筑）、
    `FactionGoal(faction, goal)`（给势力设短期目标）。
- `[ ]` **多结局 / 势力湮灭**：某个势力被完全吞并（无城无舰）时，不只是一条 `Resurgence`
  重生，而是可选地写一条「该势力陨落」的编年史并把它从 `factions` 移除（简化：由
  `config` 加一个开关）。让「消灭一个文明」有重量；但要与「反僵尸重建」权衡（长局别把
  世界玩没人）。
- `[ ]` **阵营/派系内政**：`alignment`/`aggression` 之外，再加一维「民粹/建制」，影响势力
  对外开战倾向与盟友选择，让外交不只由一条意识形态轴决定。
- `[ ]` **主线节拍的机械落点**：剧情弧里「巨头对峙/军备竞赛」这类**标题性节拍**目前只进
  `chronicle`，不产生对应机制（seed 7 @ `--traj 60`：`arms_race` 只是文字，美中并未真的
  同步下水新战列舰）。建议给这类节拍加**机械后果**（`GrantShip` 一艘对应旗舰、`Relations`
  定向压向战争、`GrantResources` 注入造舰材料），让「讲故事的高潮」在 `State` 里有具体
  形状，而不是只改一行标题。
- `[ ]` **「共同敌人」联盟化检验**：`cult_war` 文案称「世界骤然多了一个共同的敌人」，但
  seed 7 里教团末态仅与中国交战、1 城，**并未被围攻成联盟**。可给剧情条目加一个
  `common_enemy` 触发：某势力被剧情标记为「共同威胁」时，其余势力关系向「遏制」聚拢（复用
  `sim::step_balance_of_power` 的 coalition/遏制逻辑），让「共同敌人」在数值上也真的被抱团
  针对，而不是只活在标题里。

## 8. AI 游玩体验（agent 控制面） — `[~]`

- `[ ]` **`summary`/`delta` 级联**：把治理/忠诚/MOND/家园防御的语义也放进 agent 的
  `meta` 与每回合 `events`（当前已在 `meta` 暴露 `governance`/`mond`；`events` 已有
  `Revolt`/`Resurgence`）。
- `[x]` **合纵连横格局可见化**：当前每回合 `state` 新增 `coalition` 字段（`hegemon` /
  `members` / `power_share`），`meta` 新增 `balance` 配置——AI 可直读「谁在出头、
  谁在联合制衡、谁被制裁」。
- `[x]` **可读的「忠诚/治理」可视化**：每座城的 agent 输出已带 `loyalty`（0..1）与
  `gov_distance`（到其统治势力首都的距离 AU）——AI 一眼看出哪些城在失稳边缘，好据此
  投娱乐预算 / 决定是否放弃远端殖民地。
- `[ ]` **`control` 面向目标的模板**：给 agent 的 `control` 面再加一个「按目标批量生成
  diff」的辅助命令（例如 `order-fleet-to-hold <body>` / `invest-loyalty <city> <budget>`），
  降低 agent 手写嵌套 JSON 的成本。

## 9. 军事 / 舰船定制与战斗拟真（资源→战斗力） — `[~]`

已落地一套**非数字堆叠**的拟真战斗 + 舰船定制（`src/model.rs` 的 `ComponentSpec`/
`Weapon`/`ShipPanel`/`ship_weapons`/`ship_panel`，`src/sim.rs` 的 `choose_loadout`/
`fire`/`bombard_city`/`hit_factor`，`config/game.ron` 的 `components` 表，舰级新增 `slots`，
`Ship` 新增 `hull_max`/`shield`/`shield_max`/`components`）。核心设计：**定制改变打法而非
把属性加个数字**——武器有原生伤害类型（kinetic/plasma/missile）与对盾/对甲倍率、各自的
交战距离与追踪力；防御有**护盾池**（优先吸收+再生）、额外装甲、**点防御**（拦导弹）；规避是
确定性命中折减（`hit_factor(tracking, target_speed)`）；轰炸用「武器×对甲」总齐射。造舰出厂
时 `choose_loadout` 按**资源优势**确定性装配（买不起的不装、分数=库存价值函数、无 RNG），
组件成本出厂扣、upkeep 随组件上涨——形成一个真实的「多资源→不一样打法」的军事环。
agent 每舰暴露 components 与 effective 面板，meta 暴露组件表。

- `[x]` **拟人战斗 AI：自保撤退 + 集中火力**（已实现，`sim.rs` 的 `prefer_target`/
  `FOCUS_FINISH`/`nearest_enemy_ship`/`pick_target`/AI 分支，`CombatConfig::retreat_hull`/
  `retreat_min_dist`，`GameEvent::Withdraw`）：
  * **自保撤退**：舰护甲低于 `retreat_hull`(0.20)、且离首都超过 `retreat_min_dist`(3.0 AU)、
    且有敌在射程内时，后撤回首都修整充能（发 `Withdraw` 事件、朝首都移动、不送死）——打残就
    撤、养好再回来，战争有损耗与恢复循环。**不要**在主场撤退（免「到家了还一直撤不还手」）。
  * **集中火力**：`prefer_target` 排序 = 结盟集火(focus) > **打残血敌舰**(护甲占比 < 0.5) >
    就近。先瞄准、再集火补刀（打死一艘少一个输出点），而不是各自就近乱打。
  * 测试：`combat_concentrates_fire_on_wounded_enemy`、`damaged_far_ai_ship_withdraws_to_heal`；
    34 单测 + 5 长局守卫全过；3 种子 × 3000 回合无 NaN/panic。**A/B 实测（seed 42）**：集火
    若「无脑优先打残」会让战斗过份致命→80 舰/1173 价值崩到 15 舰/143 价值；缓和为「半血以下
    才集火」后恢复到 1000 回合 50 舰、城市稳在 22；撤退若太易触发会让舰弃守前线→城市 22→17。
    目前阈值（retreat_hull 0.20 / min_dist 3.0 / FOCUS_FINISH 0.5）下：城市长期稳在 ~22。
- `[x]` **AI 军备随经济收敛（维护费保留）+ 反僵尸重建补强**（已实现，`sim.rs` 的
  `read_budget`/`EconomyConfig::upkeep_reserve_mult`，`step_resurgence` 四级立足点，
  `find_vacant_settlement`/`displace_city_for_refugee`）：
  * **造舰预算 = 库存×invest_fraction，但要先把 `upkeep×upkeep_reserve_mult`(4.0) 的维护费
    从库存里预留出来**，只把超出部分用于造舰——基线 AI 从此**不再无脑大建**（seed 42 以前
    一度 143 舰），海军收敛到经济能养得起的规模。
  * **四级立足点**保证被灭势力总能重返（无永久旁观者）：① 自己最低 id 空白城（diaspora
    claim）→ ② 全系统任意最低 id 空白城（难民避风港）→ ③ 一个从未被占的定居点上新建城 →
    ④ 世界满员时，难民潮**夺取最强殖民者的最小 id 边缘城**（难民危机，大帝国付出代价）。
  * **实测（3 种子 × 3000 回合）**：3000 回合时 3 个种子都稳定在 **~79-96 舰 / ~21 城 /
    价值 354-1282 / 全程 0 僵尸**——彻底摆脱了之前「上千回合后军备/经济降温到个位数」的
    崩坏。5 长局守卫全过；诊断（3×3000，debug）约 **1m38s**（军舰更多→O(n²) 找目标更耗，
    属实体数增长而非单舰变慢；默认 `cargo test` 快速守卫仍 ~7s）。
- `[~]` **基线 AI 军备/经济平衡（续）**：军备收敛已做（上面 `[x]`）。剩余把基线 AI 再拟人些：
  **战损后优先补舰**、**和平/战时不疏于再殖民**（把被夷平/边缘的空白城及时恢复、避免只有
  22 定居点被几家占死）、以及**市场用款更智能（缺稀有矿时优先自购关键矿物而非只按
  invest_fraction 均衡分配）**。建议用 `probe` 类观测抓「城市数/舰数/世界价值」时序后再调。
- `[x]` **选装类别配比 + 战局感知**（已实现，`sim.rs::choose_loadout` 的
  `faction_at_war`/必备武器/必备防御/战时武器加分）：军舰选装不再「挑分最高、买得起、可能
  全堆武器或全堆乌龟」，而是**至少一件武器 + slot≥2 时至少一件防御**，其余按分数填满；
  **交战中的势力给武器加分**（舍得堆火力、和平则偏向防御/支持）。确定性、无 RNG。
  ⚠️ 实盘观测（seed 42 @2000）显示**基线 AI 只造护卫舰**（104 艘、无其它舰型）——因
  `upkeep_reserve_mult` 预算偏保守 + 护卫舰最便宜，多槽舰型很少出现，配比约束当前收益有限
  （对 1 槽护卫舰只剩「战时倾向武器」）。真正的收益要等「海军多样/混编」解决后才显现。
- `[x]` **海军多样/混编**（已实现，`sim.rs::choose_next_class` 重写为「归一化资源适配 +
  去重加分 + 维护费惩罚」的加权随机）：不再「随机挑一个可负担舰级」而在资源紧张时塌缩成
  全护卫。现在**软负担**（归一化 fit = 可用资源价值/造价价值，0..1，稀缺矿降低它、造价大小
  不放大分数）+ **去重加分**（欠份额的舰型）/ **维护费惩罚** 驱动加权随机（seeded RNG，确定性），
  让海军**混编**（护卫/驱逐/巡洋/战列/航母都出现）。关键：**用归一化 fit 而非绝对值 fit**——
  绝对值会让富势力把昂贵舰得分放大、拉开贫富差距、单一势力长期霸榜（`world_is_multipolar`
  一度挂掉）；归一化后稀缺资源仍影响舰级选择、造价不放大，**既混编又不破坏多极轮换**。
  验证：3 种子 3000 回合舰队混编（如 seed 42 r800：29 驱逐/11 战列/4 巡洋/1 护卫）、
  ~43-66 舰/~20-22 城/0 僵尸；36 单测 + 5 长局守卫全过；诊断 ~77s（debug）。
- `[x]` **武器克制选目标（拟人「别浪费导弹打点防重镇」）**（已实现，`sim.rs` 的
  `weapon_fit` + `prefer_target` 加 fit 维度 + `nearest_enemy_ship`/`pick_target` 接收攻击舰
  武器）：AI 在**选目标**时按「武器克制」排序——**导弹会被目标点防御拦截**（对 PDS 重的
  目标可打度下降）、**动能对高护盾目标较弱**——于是有导弹的舰优先打**没有**点防御的目标、
  有动能炮的舰避开高盾目标，而不是把导弹浪费在被全面点防的堡垒上。优先级 = 结盟集火 >
  **武器克制** > 打残血 > 就近。确定性、无 RNG。测试：`target_selection_respects_weapon_advantage`
  （导弹舰避开 PDS 目标）；37 单测 + 5 长局守卫全过。
  ⚠️ 实盘调参教训：把「武器克制」放在**残血之后**会让 `world_is_multipolar` 挂（最强势力
  轮换被破坏）——选目标若优先补刀残血，会陷入与克制武器的消耗战、某势力被拖垮。**先武器
  克制后残血**反而更利于多极轮换（避开被克制武器拖入消耗战）。即使如此，个别种子在某个
  检查点会出现**临时性**深下挫（如 seed 42 r1000：6 舰/15 城/67 价值），但会在 ~2000 回合
  恢复到 41-59 舰/22 城——属「战争潮汐」而非永久崩坏（无僵尸、多极保持）。若要更平滑，
  可考虑把 `weapon_fit` 的钝化系数调低/把波动视为真实要素。
- `[x]` **威胁响应造舰（战时多造重舰）**（已实现，`sim.rs::choose_next_class` 的
  `faction_at_war` + `war_bonus=spec.attack*0.03`）：交战中，AI 比和平更倾向造攻击力高的
  重型战斗舰（战列/航母/巡洋），和平则更均衡/偏护卫与资源经济。确定性、无 RNG。测试：
  `choose_next_class_builds_heavier_navy_at_war`（抽样对比战/平重舰比例）。
  **副作用（实测，正面）**：除了让海军按威胁响应构建，还**显著减小了 seed 42 之前的战争
  潮汐深下挫**——重舰海在战时更守得住城市（之前 r1000/r2000 掉到 6-7 舰/8-15 城，现在
  各检查点都稳定在 34-69 舰/19-22 城）。3 种子 × 3000 回合：0 僵尸、~19-22 城、36-63 舰。
  注意：这是**新建舰厂/重殖民**的舰级偏好，对既有船坞的 `ship_type` 影响有限——要让整支
  舰队随威胁重构（把护卫船坞改造成战列船坞），需要更大的「船坞重定向」改动（下一项）。
- `[x]` **整支舰队随威胁重构（战时重定向船坞）**（已实现，`sim.rs::retool_shipyards` +
  `step_construction` 里的重定向 pass）：交战中，若某势力的舰队被单一舰型**严重**统治
  （占比 > 0.60），就把它产出该舰型的最小 id 船坞重定向到 `choose_next_class` 选出的**战局
  感知新舰型**（含战争加分——多造重舰；去重加分——避免单调）。让**威胁响应作用于整支舰队**
  而不是只作用于新建舰厂。和平时不重定向。每次至多重定向一个船坞、只在严重单一时触发
  （保守阈值），避免抖振。确定性（seeded RNG）。测试：`war_retools_over_abundant_shipyard_toward_a_war_class`。
  **调参教训**：`over_share` 若设 0.45，会让舰队过重、胜负更「均匀」、导致**从未出现霸权→
  反制联盟**（`world_is_multipolar` 的 `coalition_seen` 挂掉）；调到 **0.60** 才既保留
  重定向又保住「霸权→联盟」的合纵连横节奏。41 单测 + 5 长局守卫全过。
- `[x]` **护航（保护高价值舰种/旗舰）**（已实现，`sim.rs::fleet_flag` + `resolve_target` 的
  护航分支 + `CombatConfig::escort_range`(10.0)）：交战时，**闲着**（无敌对目标、无可殖民
  空白点）且距本势力**旗舰（第一艘航母）**在 `escort_range` 内的 AI 舰，会就近护卫它
  （`TargetShip{attack:false}` 守卫行为：贴近旗舰 + 拦截进射程之敌），防止高价值舰被轻易
  打掉。确定性、无 RNG。测试：`fleet_flag_is_the_factions_first_carrier`。
  ⚠️ 触发较窄：仅在「无任何敌对目标/空白点」（如战后或孤立）时才会护卫旗舰——因为现有
  目标选择（`pick_target`）会选**任何**敌对目标（即使很远），所以「想追远处敌人」的舰不会
  转去护航。要让护航更多触发（当舰**射程内无敌人**时就护卫而非跨图追击），需要把「不跨图
  追远敌」纳入目标选择（见下一条「更拟人的追击/驻守」）。
- `[x]` **更拟人的追击/驻守（不追远敌）**（已实现，`CombatConfig::pursuit_range`(12.0) +
  `pick_target` 的军舰目标门控）：AI 舰只在 **追击半径内** 才去追敌对舰——**不跨越全图去追
  一艘远逃的敌舰**（避免过度延伸、漂移、被伏击）；超过则转而轰炸城市/殖民/护卫。城市
  （轰炸/殖民）**不受**此限制（征服仍值得远征）。确定性、无 RNG。测试：
  `ships_do_not_chase_enemies_beyond_pursuit_range`。
  **附带效果（实测）**：这同时让**上一条护航更易触发**（舰不追远敌→更多时候闲着→就近
  护卫旗舰），且**让世界战争更少见**（舰不追远敌→少过度延伸战争→城市更稳、各检查点
  更平稳，seed 42 之前的战争潮汐也基本消失）。3 种子 × 3000 回合：0 僵尸、~19-22 城、
  values 少数种子较大（<2M，仍受 `world_value_is_bounded` 约束），舰数 35-57。
- `[x]` **舰队防空（有 PD 的舰替附近友舰拦导弹）**（已实现，`CombatConfig::pd_radius`(3.0) +
  `sim::cluster_pd_cover` + `fire` 里 `pd = 自身拦截 + cluster_pd_cover`）：舰的点防御除了拦
  自己吃到的导弹，还会在 **`pd_radius` 内替附近友舰拦截**（随距离线性衰减、封顶 30）——
  有 PD 的舰组成防空圈、护卫航母/友舰（与护航衔接：护航舰贴近旗舰时提供防空）。确定性、
  无 RNG。**性能优化**：只在**攻击方有导弹武器**时才做这次 O(舰队) 的防空扫描（导弹才是
  会被拦截的），平时跳过。测试：`fleet_air_defense_covers_nearby_missile_targets`。
  ⚠️ 参考轨迹里它基本未生效（舰队很少在 3 AU 内紧密聚团），属**正确但微妙**的机制——真正
  有意义要等「编队聚团」或「导弹武备更普遍」；当前至少不破坏平衡（守卫全过、诊断不崩）。
- `[~]` **AI 军事智能（更拟人，结）**：撤退/集火/混编/武器克制/威胁响应造舰/舰队重定向/
  护航/不追远敌/舰队防空已做。军事战斗范畴的「AI 更智能」点子基本落地；剩余更多在
  **长局可玩性/经济**：舰船退役换装、护盾对物理/能量更深差异、市场用款更智能、以及
  「编队聚团/阵型」这类能放大防空与掩护价值的协同（较复杂、低优先级）。
- `[~]` **武器/护盾失衡观测与调参**：选装已加**类别配比**（上面 `[x]`）。可再跑 `probe` 类
  观测看**实盘**各舰级实际选装的武器:防御比例，据此微调 `FOCUS_FINISH`/权衡系数，避免
  「全玻璃大炮或全乌龟」——但先要解决上面「海军多样」问题才有实盘意义。
- `[ ]` **舰船退役 / 换装**：目前组件出厂即定型，无法随资源变化重装配。可加「服役若干
  回合后可按新资源优势重装」（增加指令面与长局策略深度），但需权衡复杂度。
- `[ ]` **护盾对物理/能量防御的差异深挖**：已有 shield_mult/hull_mult，可再细分
  「对船/对城」（如导弹对城市建筑有加成、动能对船体有加成），让「选什么武器打谁」更值得
  取舍。
- `[x]` **损坏/战术损伤（模块损毁）**（已实现，`Ship::component_hp` + `sim::fire` 的
  `component_spill` + `model::component_effectiveness`/`component_integrity`）：被击中的舰，
  其**每件组件有一个完整度**（`component_hp`，初始 = `component_integrity`），战斗中每点
  船体伤害按 `component_spill`(0.05) 比例「溢出」去损坏组件（最脆的先掉）。组件**有效度 =
  当前完整度/初始完整度**（0..1），`ship_panel`/`ship_weapons` 按其缩放——受伤的武器打折扣、
  受伤的护盾减量、被击毁(0)的组件彻底失效。军舰因此**随战况渐进丧失战力**，而非满血抗到
  壳破。确定性（无 RNG）。agent 每舰暴露 `component_hp`。
  **调参教训（实测）**：把功能做成「比例有效度」后一度让 `world_is_multipolar` 挂掉
  （组件损毁让战斗更锐化→赢家雪球、最强势力轮换被冻）；把 `component_spill` 从 0.12 降到
  **0.05** 后才既保留功能（round 2000 实盘组件如 railgun hp 17.4/16.3 确实下降）又保住多极
  轮换。即使如此，个别种子在某个检查点会**临时性**深下挫（seed 42 r1000：5 舰/8 城，r3000
  恢复到 35 舰/22 城）——属战争潮汐而非永久崩坏。若要更平滑，可把 `component_spill` 再调低
  或与更强的均势外交联动。
- `[x]` **修船/修复模块**（已实现，`CombatConfig::component_repair`(0.04) + `sim::step_military`
  再生循环里的组件修复）：受损组件的完整度每回合按 0.04 恢复（占组件初始完整度）；在
  **友方本土/母港（首都 `home_radius` 内）修得更快（×2.5）**——与自保撤退行为闭环成
  「**打残→撤→修→再来**」。确定性（无 RNG）。测试：`damaged_components_repair_in_friendly_territory`；
  39 单测 + 5 长局守卫全过；3 种子 × 3000 回合健康（0 僵尸、~20-22 城）。
  它是**恢复性**机制（让打残的舰存活更长、战争少些无谓消耗），对长局稳定有正面作用
  （不像损毁那样锐化战斗）。⚠️ seed 42 在 r1000/r2000 仍有临时性深下挫（6/15/67、7/9/436），
  但 r3000 恢复到 43 舰/21 城——战争潮汐。
- `[ ]` **换模块 / 再装配**：目前组件出厂后固定（只能修复不能更换）。可加「服役若干回合
  后可按当前资源优势重装组件」（增加指令面与长局策略深度），但需权衡复杂度与确定性。

---

## 10. 舰级数值按最新 spec 对齐（新增「点防御修正 pd_mult」舰级属性） — `[x]`

已落地：`.agents/spec.md` 的最新修订把每级舰重新定性（护卫舰=哨戒、驱逐舰=远洋部署、
巡洋舰=扛伤害、航空母舰=远程放风筝、战列舰=玻璃大炮），并**新增一个舰级属性
「点防御修正 pd_mult」**——舰级=平台修正器的一环（此前缺失该斜率，点防不随舰型变化）。
改动：
- `src/model.rs`：`ShipSpec` 新增 `pd_mult`（`#[serde(default=default_mult)]`，旧配置无它也
  能加载）；`ship_panel` 的点防拦截改为 `cs.intercept × base.pd_mult × eff`。
- `config/game.ron`：五级舰全部按 spec 重填——新增 `pd_mult`（护卫/战列 1.4 高、驱逐 1.0
  中、巡洋/航母 0.9 低），并重分布其余修正系数（护卫=加速度极高 1.8 + 点防高 + 速度中；
  驱逐=高速 1.3 + 高再生；巡洋=重甲重盾 + 盾再生高；航母=超远程 1.8 + 各项低；战列=攻
  极高 2.0 + 射程高 1.3 + 点防高，槽位降为 3）。战列 hull 26→30（中）。
- `src/sim.rs`：`choose_loadout` 的组件得分把 `intercept` 乘上本舰级 `pd_mult`（点防模块在
  pd_mult 高的舰上更值得装）。新增单测 `ship_panel_scales_intercept_by_class_pd_mult`。
- `src/agent.rs`：meta 的 `ships` 表暴露 `pd_mult`。`src/main.rs` help 文案同步。
- 验证：`cargo test --lib` 45 项全过；`cargo test --test longhorizon` 5 个长局守卫全过
  （多极/反僵尸/价值有界/确定性/无 NaN）；`diagnose_long_horizon`（3 种子 × 3000 回合）
  0 僵尸、~20-21 城、44-50 舰、无 NaN。

---

## 11. Schema/查询架构再造（降低「schema 查询维护」心智负担） — `[ ]`

> 调研设计文档见 `.agents/schema-query-architecture.md`（诊断+分层方案俱全）。此节只列
> 「未落地的候选改造」，供下一步直接照做。**现状四痛点**：① `State` 真身之外还有 4 套
> 并行手写投影（`agent.rs` 的 `AgentState`、`meta_value`，`web.rs` 的 `MetaView` 等，
> control/diff 面）；② schema 隐式，agent 要背；③ query 引擎宽容（缺字段→null，漂移
> 静默失效）；④ 无 `schema_version`/迁移，旧 `.ron` 语义变化被静默错载。

- `[x]` **P0 修 `meta_value` 漂移（先止血）**（已落地，`src/agent.rs`）：`meta_value` 的
  `economy/combat/diplomacy/market/governance/mond/balance` 与 `structures/buildings/ships/
  components` 段改用 `config_json`（`serde_json::to_value` + 递归浮点圆整）从各 `GameConfig`
  子结构**直接派生**，删手写字段清单。**已知漂移**（已修）：`combat` 原来漏了
  `component_spill`/`component_repair`/`escort_range`/`pursuit_range`/`pd_radius` 这 5 个
  近期字段，现在都出现在 meta 里。整数型字段（`slots`/`min_members`/id）保持整数不被圆整。
  `resources`（raw key→中文名）与 `story`（平铺 trigger/effects）仍为刻意保留：前者是
  显示名翻译表、后者是真转换。新增守卫单测
  `agent::tests::meta_derives_all_combat_fields_and_keeps_ints`。**未做**：`web.rs` 的
  `MetaView` 仍是独立一套（可后续复用同源）。
- `[x]` **P1 schema 自描述**（已落地，`src/agent.rs` + `src/main.rs` + `Cargo.toml`）：
  `Cargo.toml` 加 `schemars = 0.8`（derive）；agent 视图类型（`AgentState`/`AgentCity`/…/
  `AgentOrder`）加 `#[derive(Serialize, JsonSchema)]`；`agent::schema_value()` 用
  `schemars::schema_for!(AgentState)` 生成 JSON Schema；`main.rs` 加 `schema [<jq>]` REPL
  命令。agent 从「背 schema」变「查 schema」（嵌套类型走 `$ref`/`$defs`）。`meta`/`state`
  顶层**未**塞 schema（避免每回合行肿胀），独立命令即可。
- `[x]` **P2 查询响亮失败**（已落地，`src/query.rs` + `src/main.rs`）：`query.rs` 加
  `apply_strict`/`apply_strict_lines`，strict 模式下 `Path` 访问不存在字段 / 越界索引返回
  `Err(unknown field …)`（而非 null）；CLI 加 `--strict`，REPL `q --strict <jq>`，默认仍
  宽容。新增 3 个守卫单测（未知字段报错 / 存在字段通过 / 越界索引报错）。让 schema 漂移
  立即可见、不静默。
- `[x]` **P3 版本化+迁移**（已落地，`src/model.rs` + `src/config.rs` + `src/world.rs`）：
  `State` 加 `schema_version: u32`（`#[serde(default)]`，旧档为 0）+ `SCHEMA_VERSION = 1`；
  `config.rs` 的 `load_state`/`load_checkpoint` 解析后调 `model::migrate(&mut state)` 逐档
  升级；版本过新则显式报错（宁抛错不错载）。`world::default_state` 写 `schema_version`。
- `[ ]` **P4 收敛并行投影**：把 `agent.rs` 的 `AgentState`/`AgentCity`/… 镜像 struct
  拆掉，改为对真身 `State` 用 serde 属性（`skip_serializing_if`/`rename`/`serialize_with`）
  做用户态裁剪；至少先做到「从真身 derive/生成、不手抄」。工作量最大，建议后续渐进
  （先拆 `AgentShip` 再拆 `AgentCity`…）。P1 已给这些镜像 struct 加了 `JsonSchema`
  derive——若最终拆掉它们，`schema_value()` 的生成目标也要换成新派生视图。

---

## 12. 语义化「视图/工具」API（把裸 jq 降为逃生舱） — `[ ]`

- `[~]` **命名语义工具集**（对应设计文档 Layer 2）：把 agent 主界面从「裸 jq 探 JSON」
  升级为稳定、有版本、参数化的语义操作，由模拟自己生成数据（复用 `sim.rs` 现有的
  `balance_picture`/`governance_distance` 等），不要求 agent 知道底层 JSON 树。候选：
  `view sitrep` / `view economy <faction>` / `view fleet <faction>` / `view frontier`
  （边缘失稳城）/ `view threat`（威胁评估）/ `view market`。`q`/`--query` 降为逃生舱。
  **已落地一个零改动 Rust 的 PoC**（`play/jq/view_sitrep.jq`，另加 `view_profile.jq`）：
  把一回合 33 KB 的裸 JSON 压成 **956 字节**的语义 sitrep
  `{round,world{wars,factions[],top{share}}}`（wars 从 relations≤-20 推断、top 取城数最多者
  及其占比），压缩 ~34×，agent 无需手写 join/计算、也不受 PowerShell 引号坑影响。
  Rust 侧价值更大：把 `top_share` 换成真实 `balance_picture`、`frontier` 用
  `governance_distance`/`loyalty` 列出边缘失稳城、`economy` 用 `resource_value` 加权库存，
  并做成 `--view <name>` / `view <name>` 命令（带参数校验、版本化）。
- `[ ]` **工具层校验**：语义工具做参数校验（如 `order 0 attack 99` 明确报「目标舰不存在」），
  把错误在工具层显式化，而不是丢给宽容的 jq。
- `[ ]` **Layer 3 方向（长期/可选，非现在必须）**：ECS 组件化存储 + 事件溯源投影，把
  「资源/建筑/舰级用字符串键+配置表」这套数据驱动模式推广到实体属性；新增机制=加组件
  +加 system/投影，不动既有结构体。投入大、破坏 `.ron` 可读性，仅当扩展需求真正爆了才做。

---

## 13. Agent 讲故事模式：轨迹生成器 + 外部 jq（已落地） — `[x]`

> 形态转变：agent 从「玩家（每回合 advance→读→apply diff）」变成「**导演/说书人**」——
> 跑一段轨迹、**任意时间轴查询**、讲给用户看。已决定并实现：**外部 jq 为必选依赖；
> 删掉自研 jq 引擎与查询 REPL；agent 仍可用 `--apply` diff 影响故事走向；Web
> （`planet_x_web`）是玩家界面，完整保留。** 落地设计见
> `.agents/schema-query-architecture.md` §10。

- `[x]` **删掉自研 jq 引擎**（`src/query.rs` 整文件 + `lib.rs` 的 `pub mod query` + 其全部
  12 个单测）。raw 任意查询全权交给外部 jq。
- `[x]` **删掉查询 REPL 与 `--query`/`--strict`/`--script`**；`main.rs` 重写为**批次轨迹
  生成器**：
  - `--round N`：输出 JSON Lines（回合 0 先，之后每回合一行 agent 视图，含 `.events`）——
    `| jq` 任意时间轴查询（`jq -s` 可整段收集）。
  - `--traj N`：输出一个自包含 `{schema_version, meta, story, trajectory:[...]}`（story pack）。
  - `--meta`/`--schema`/`--story`/`--control`：各自输出一个 JSON 值（规则字典 / agent 视图
    schema / 编年史 / 可编辑控制面模板）。
  - `--apply <diff>`：叠加控制 diff 定向故事；`--start <ckpt>`+`--save <ckpt>` 分段续玩
    （每段纯函数，seed/checkpoint 保证可复现）。
- `[x]` **验证**：`cargo build` 全目标成功；`cargo test --lib` 37 passed（原 49 = 删掉
  query 模块 12 个单测）；`cargo test --test longhorizon` 5 passed / 4 ignored。smoke：
  `--round 30 | jq` 时间序列、`--traj` pack（schema_version=1、trajectory_len=13、
  story_len=7）、`--schema`（title=AgentState）、`--control`（control+scope）、`--apply`
  定向（faction0 iron budget=999 mode=Player 确实落入控制面）、分段
  `--start ckpt +--round 6` 与直跑 `--round 12` 的末行**逐字节一致**（确定性满足）。
- `[ ]`（事实，非待办）**Web 玩家界面未动**：`planet_x_web` 保留有状态 advance/apply_patch
  交互；`web::control_surface`/`apply_patch` 与 CLI `--apply` 共享同一控制/diff 契约。
- `[x]` **`jq` 成为宿主运行时依赖**（本次已落地，`tools/jq.exe`）：主机未预装 jq 时，
  `Invoke-WebRequest https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-windows-amd64.exe`
  下载独立二进制到 `tools/jq.exe`，`Get-Command jq` 前先 `& tools\jq.exe`（或加目录到 PATH）。
  PowerShell 直传 jq 程序的**引号会被吞**（`"1"` 变 `1`、`"\(..)"` 变 `..`），**可靠做法是把
  jq 过滤写成 `.jq` 文件再 `jq -s -f file.jq`**——已用此范式完成 `--round`/`--traj`/`--digest`
  全量查询验证（脚本见 `play/jq/*.jq`）。
- `[~]` §12 的「命名语义工具」仍可作为 Layer 2 方向，但有了外部 jq 的全量语法后，「裸 jq
  探 JSON」不再棘手；语义工具聚焦「稳定契约 + 参数校验 + 由模拟生成数据」，而非替代 jq。

---

## 14. 权威 schema 贯彻：WYSIWYG 资源 key + 拆掉 `AgentState` 镜像（在 auth-schema worktree） — `[x]`

> 用户拍板（方案 X）：**资源 key 直接用可读中文名（WYSIWYG），不再有 raw key ↔ 显示名翻译**；
> 并且「既然都是 jq 查询了」，**拆掉手写的 `AgentState` 镜像投影**，让 agent 视图直接复用
> 权威模型类型（单一来源、schema 自描述、无镜像）。Web 仍为玩家界面，不动。

- `[x]` **资源 key 全局改成可读名（WYSIWYG）**：`config/game.ron` 的 `resources` 表、各
  `build_cost`/`cost`、story `GrantResources`；`world.rs` 的 `deposit`/`stockpile`；
  `sim.rs`/`longhorizon.rs`/`web.rs` 测试里的资源字面量——全部由 raw（`water_ice`/`iron`…）
  改成中文名（`水冰`/`铁`…）。`config.resource_name` 变成恒等，`meta.resources` 的
  raw→中文 翻译表**删除**；agent 看到的资源 key 就是它在 `--apply` 里能写的 key。
- `[x]` **拆掉 `AgentState` 镜像，agent 视图 = 权威 `Trajectory`**：删除
  `AgentState`/`AgentFaction`/`AgentBody`/`AgentOrbit`/`AgentSettlement`/`AgentDeposit`/
  `AgentCity`/`AgentBuilding`/`AgentShip`/`AgentOrder`（10 个镜像 struct）+ `from_state` +
  `game_event_value`/`faction_name`。改为一个
  `#[derive(Serialize, JsonSchema)] pub struct Trajectory`，字段直接用权威类型
  `Vec<Body>/Vec<City>/Vec<Faction>/Vec<Ship>` + `Vec<GameEvent>/Vec<ChronicleEntry>`
  （不含控制面 `control`/`scope`）。`schema_value()` 用 `schemars::schema_for!(Trajectory)`
  ——schema 是权威类型派生的自描述，与发射 JSON 同源、不会漂移。
- `[x]` **给模型类型加 `#[derive(JsonSchema)]`**：`Body`/`Orbit`/`Settlement`/`ResourceDeposit`/
  `City`/`Building`/`Faction`/`Ship`/`GameEvent`/`ChronicleEntry`。新增 `use schemars::JsonSchema;`。
- `[x]` **守卫测试**：
  - `meta_value_covers_every_config_section`：`meta` 必须覆盖每个 config 段（key 集合是
    config 的超集）——拦住「配置加了字段、meta 漏了」这类漂移（就是 `component_spill` 那类）。
  - `agent_view_is_self_described_by_schema`：`state_json` 发射的顶层 key 都在 `schema_value()`
    的 `properties` 里，且实体数组被声明——schema↔发射一致性显式化。
- `[x]` **验证**：`cargo build` 全目标无 warning；`cargo test --lib` 39 passed（37 + 2 守卫）；
  `cargo test --test longhorizon` 5 passed / 4 ignored。smoke（`--seed 7`）：`--round 12`
  的 faction[0]（联合国）资源 key 是 `氦-3/碳/铁`（中文）、relations 按 id、events 带
  `type`；`--schema` title=`Trajectory`、顶层 key 与发射一致（bodies/chronicle/cities/
  events/factions/round/ships/time_month）；`--traj` pack（traj_len=7、story_len=4、
  meta.resource_value 存在）。city/ship 用权威字段（body_id/faction_id/loyalty/buildings/
  components/class）。
- `[ ]`（须知）**行为变化**：agent 视图不再预计算派生字段——舰的 `panel`（attack/range/
  speed/upkeep）、城 `armor`/`gov_distance`/`owner_name`/`body`名、`relations` 从「按名字」
  变「按 id」、`events`/`chronicle` 保留。agent 需用 jq 现场 join/计算（`--meta`/`--schema`
  提供规则与字段；若要省事，可后续加几个 Layer-2 语义视图命令）。

---

## 15. 超长轨迹的「粗粒度 / 降采样视图」 — `[~]`

> 问题：`--round 3000` 输出 3001 行全量 JSON，agent 上下文/管道撑不住。要让 agent 先看
> 「粗粒度」再决定是否放大。落了一套，剩下的是候选。

**已落地（`src/main.rs`）**：
- `[x]` **`--every K`（降采样快照）**：`--round N --every K` 只发 round 0、K、2K… 行（仍是
  **全量** agent 视图），超长轨迹秒变可读采样。`--traj N --every K` 同理降采样 `trajectory`
  数组。配 `--start`/`--save` 做「先粗看、再放大」的分段叙事。
- `[x]` **`--digest K`（语义编年史窗口 / 故事板）**：`--round N --digest K` 每 K 回合发**一行
  语义摘要**——该窗口各势力 `city_count/ship_count/fleet_value`、总 `power_share`/`hegemon`/
  `coalition_members`、`wars`（交战对）、`events`（各类型计数）、`story`（本窗口触发的剧情节拍
  id）。`--digest 100 --round 3000` → 30 行，就是「编年史的粗帧」。这是超长轨迹真正能
  读的故事板：agent 扫完决定放大哪些窗口，再 `--start ckpt --round <span>` 精读。
- smoke：`--round 50 --every 10` → 6 行（0,10,…,50）；`--round 48 --digest 12` → 4 行，
  win1 的 `story=[northern_watch,first_raze,kuiper_boom]`、`wars=(1,3)…(5,8)`、`events`
  有 attack/siege/revolt/…；`--traj 30 --every 5` → 7 帧。

**候选（未做）**：
- `[~]` **极致「画像」**：只发一条极简标量时序（每窗口：城数/舰数/世界价值/最强势力份额），
  几乎零上下文，适合先看走势。**已落地一个纯 jq 版**（`play/jq/view_profile.jq`，一并算出
  wars 数）：`jq -cs -f play/jq/view_profile.jq traj.jsonl` → 每回合一行
  `{round,cities,razed,ships,fleet_hull,top_name,top_share,wars}`。seed 7 240 回合：整条
  轨迹从 **8.7 MB 压到 ~29 KB**，一眼看清「22 城→13 城(9 夷平)→再殖民→尾声 17 城 7 舰队」
  的走势与 top_share 轮换（0.18→0.31→0.53(矿业)→0.5→…→0.53(中国)）。**Rust 侧**可把
  `top_share` 换成真实 `balance_picture.power_share`、并加 `--profile K` 窗口化（见 §12）。
- `[ ]` **分层缩放 CLI**：`--zoom from to --every k`（在已存 checkpoint 上精读某窗口）——
  现在可手工 `--start ckpt --round <span> --every k` 达成，值得包装成一条命令。
- `[ ]` **窗口事件文案**：`--digest` 的 `events` 计数之外，附几条**一句话**事件摘要
  （如「第 210 回合：联军包围霸权，铂被封锁」）——半叙事粗帧。
- `[ ]` **jq 现成的粗聚合**（零代码，留给 agent）：`planet_x --round 3000 | jq -s '[group_by
  (.round/100|floor)[] | {from:.[0].round, ncity:(map(.cities|length)|add/length)}]'`。

---

## 16. 统一「总结」= 步进函数的中间计算变量 — `[x]`（feature/unified-metrics）

> 问题：agent 除了直接状态，还看到很多**总结**（实力占比/霸权/联盟/制裁/交战、各势力城市·
> 舰队·人口·库存价值、世界总量）。这些总结其实就是**步进函数里算的那些中间变量**，不该由
> `--digest` 或别处**独立重算**一遍——否则容易与模拟漂移、重复劳动。

**已落地（`src/model.rs` / `src/sim.rs` / `src/agent.rs` / `src/main.rs`）**：
- `[x]` **`RoundMetrics` / `FactionMetrics` / `CityMetrics`**（`src/model.rs`，均
  `#[derive(..., JsonSchema)]`）：一回合的总结指标类型。存量/政治：世界级
  `cities/ships/fleet_value/population`、`power_share`、`hegemon`、`coalition_members`、
  `sanctioned`、`wars`（交战对，无序），以及 `factions`（key=faction id →
  `FactionMetrics{city_count,ship_count,fleet_value,population,market_value,at_war}`）。
  **流量**：`factions[].production_value/production/upkeep/governance_cost/
  governance_coverage` 与 `city_production`（key=city id →
  `CityMetrics{population,loyalty,production_value,production}`）。
- `[x]` **`RoundFlow` / `GovernanceFlow`**（`src/model.rs`）：一回合的**流动性中间量捕获**——
  `advance` 在步进时把产出/维护/治理写入它并返回（它是 `advance` 的返回值），使这些量与
  模拟逐回合一致，而非事后从状态反推。
- `[x]` **`sim::advance(state, config, rng) -> RoundFlow`**：在 `step_production`/`step_upkeep`/
  `step_governance` 里把「中间量」记入 `RoundFlow`（每城每资源产出、每势力每资源产出、
  舰队维护费、治理总开销/覆盖率），并在回合末随 `flow` 返回。
- `[x]` **`sim::round_metrics(state, config, &flow)`**（`src/sim.rs`）：**唯一权威**的总结聚合器，
  复用步进函数本身的计算——一次 `balance_picture`（内部 `faction_power_share`+`coalition_of`）、
  一次 `sanctioned_hegemon`、一次 `war_pairs`，再补世界/各势力的城市/舰/兵力/人口/库存价值，
  并把 `flow` 里的产出/维护/治理并入。纯函数、无 RNG，同种子完全复现。
- `[x]` **agent 视图带 `metrics`**（`src/agent.rs`）：`Trajectory` 增加 `pub metrics: RoundMetrics`；
  `state_json(state, config, &flow)`/`render_state(state, config, &flow)` 带 flow，内部调
  `sim::round_metrics`。因此 agent 在每个回合 JSON 里**同时看到直接状态 + 存量总结 + 流量
  总结**，且 schema 自描述（`agent_view_is_self_described_by_schema` 增加对 `metrics` 及
  流量字段的断言）。回合 0（起点未步进）用 `RoundFlow::default()`（流量为 0）。
- `[x]` **`--digest` 与逐回合视图同源**（`src/main.rs`）：`run_digest` 在窗口末态调用
  `sim::round_metrics(state, config, &flow)`，`digest_value` 直接读它（删除重复的
  `active_war_pairs` 与独立的势力/世界聚合逻辑）；并**逐回合累计**各方产出，
  `factions[].production` 为窗口总量。digest 也随之**新增**
  `population/market_value/at_war/sanctioned/production/upkeep/governance_cost/coverage`。
- `[x]` **`-0.0` 规整**：`r2`/`round_value` 加 `+ 0.0`，避免 `f64::round` 保留的负零让 agent
  看到 `-0.0`。
- 验证：`cargo build` 无 warning；`cargo test --lib` 39 passed；`cargo test --test longhorizon`
  5 passed / 4 ignored（含 `same_seed_reproduces_identically` 现用末回合 flow 渲染，验证流量
  同样可复现）。smoke（`--seed 7`）：`--round 5` 的 `metrics.factions[3]` 有
  `production={硅/碳/铁}, production_value=64.75, upkeep=23.5, governance_cost=4.8,
  governance_coverage=1.0`；`city_production[0]` 有 `production_value=28.4`；
  `--digest 12` 的 `factions[].production` 是窗口累计值（如 美国=347.45）。

**候选（留待后续）**：
- `[ ]` 把治理中间量（`governance_total`/`coverage`/距首都距离）也并入 `RoundMetrics`——给
  agent 一个「帝国为何要崩」的预警阅读，代价是每回合多一遍治理公式。
- `[ ]` Web（玩家界面）要不要共享同一份 `round_metrics`（现在刻意不动它）——若玩家也想要
  「世界一目了然的概览面板」可复用，但形状是面向玩家，另行设计。

---

## 17. Lazy 间接索引 + agent 可读 schema + Python/pandas（uv）分析层 — `[x]`（feature/lazy-index 原型）

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
- `[ ]` **统计函数库**（Python 侧，`planet_xq` 内）：`series(faction,metric,every)`、
  `rolling_mean/max`、`histogram(metric,bins)`、`hegemon_timeline()`、`leader_rotation(metric)`、
  `war_durations()`、`gini(stockpile)`——把「方便做统计」做成库而非让 agent 每次手写 pandas。
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
  另：轮换本身在 seed 7 成立，说明**目前丢的不是「轮换」而是「末态均衡」**，宜作为 §2 的一条
  独立可量化守卫（断言 240 回合末最强势力城占比 ≤ 0.5）。
- **长局性能**：`tests/longhorizon.rs` 的 `diagnose_long_horizon`（3 种子 × 3000 回合）较慢
  （~1 分钟），已 `#[ignore]` 化；默认 `cargo test` 只跑快守卫（~13s），别把慢测得放回默认。
- **确定性**：新增机制全部为确定性（无 RNG 或仅用种子 RNG）；`same_seed_reproduces_identically`
  守卫可复现性。任何新机制不要引入未播种的随机性。

---

## 快速参考：验证手段

- 单元测试：`cargo test --lib`
- 长局快守卫：`cargo test --test longhorizon`
- 长局诊断（慢，可打印）：`cargo test --test longhorizon diagnose_long_horizon -- --ignored --nocapture`
- 一次简短观察：`cargo run --bin planet_x -- --seed 7 --round 30 --digest 10`（每 10 月一行故事板）
- 一键拿故事素材：`planet_x --seed 7 --round 60 --index out/`，再用 `play/planet_xq`
  (`planet_xq.load('out').facts`) 读主流与 `chronicle`（累计编年史按 `(round,id)` 去重）。

---

## 18. Agent 游玩体验优化（游玩摩擦点） — `[ ]`

> 本轮实测（`--seed 7`，agent 用 `--index` + `planet_xq` 分析、`--apply` 下指令）暴露的
> 「agent 玩起来难受」的摩擦点。按「痛点强度 × 影响面 × 改动量」排序。多与 §8 / §12 / §15 /
> §17 已有的候选点子重叠，这里补**闭环**视角（读完→决策→下发→观察→复现）与**语义护栏**。

### 18.1 指令下发太难写（最高优先级）
agent 为下一次 `--apply` 决策，要手写 `control/scope` 且背一堆整数 id（`city`/`building`/
`ship`/资源 key）。实测为「让中国在地球造战列舰」要查出 building id=3=船坞、city id=0、
`ship_type` 是 `battleship`…… 纯手搓，易错。

- `[ ]` **语义指令助手**：把 `--apply` 的叶子 diff 生成包成**意图式**高层命令，由 Rust 解析成
  叶子（复用 §8 的 `order-fleet-to-hold <body>` / `invest-loyalty <city> <budget>`）：
  - `--fight <faction> <enemy_faction>`：把该势力现有舰全部 `TargetShip{attack:true}` 指向最近的
    敌对舰（`pick_target` 已有逻辑）。
  - `--build <faction> <ship_class> [<city>]`：给该势力（某城）的某个建造区 `ship_type` 设为该
    舰级（+ `build_weights` 加权）。
  - `--colonize <faction> <body>`：下令 `Colonize{body}`。
  - `--hold <faction> <body>`：把舰队调去守卫/驻守该天体（复用 `dock`/`guard`）。
  - `--loyalty <faction> <city> <value>`：`loyalty_budget`。
  - 原则：**每个命令只出它改的那几片叶子**，返回「实际落地的 diff + 每条的稳定可读 id」，让
    agent 不用记整数 id 与 JSON 树。
- `[ ]` **`--schema-control`**：`--schema` 现只描述 `Trajectory`（状态视图），**没有描述控制面/
  diff**（`CommandReq`/`FactionControlPatch`/`scope`/behavior 的两种写法）。给控制面也派一个
  JSON Schema（`schemars` 派生，同 §11 P1 思路），agent 据此写 diff 而不是靠背。

### 18.2 预算语义黑盒、无「成本→收益」预览
实测把 `construction_budget` 拉满 → 维护费 79/月 > 产出 65/月 → 经济崩、共和国溃散（第 15 月
城清零）。agent 只能靠试错才知道「这个预算养得起几艘舰」。

- `[ ]` **`--control-plan <faction>`**：给当前控制面算**稳态剖面**——该势力在此预算/权重下
  「每回合产入 vs 维护 vs 治理开销 vs 可造舰上限」，并给出 `fleet_value`/`upkeep`/`city_count`
  的近似平衡点，让 agent 在写 diff 前看到代价，而不是提交后观崩盘。复用 `read_budget`/
  `ship_panel`/`round_metrics` 现有计算，纯函数、无 RNG。
- `[~]` **预算语义入 `--meta`**：`meta.economy` 已有 `upkeep_reserve_mult`/`invest_fraction`/
  `production_rate`；把「造舰预算 = 库存×invest_fraction，但先留 upkeep×4」这一换算写进 `meta`
  的说明或示例，agent 能直接按公式推，不必反推代码。

### 18.3 从「aggregate」到「该干嘛」缺一座桥
`metrics` 告诉 agent 城数/产出/份额，但要"哪座城是我的短板（低忠诚/高治理距离）""哪个邻居是
我该抱团扁的霸权""我缺哪种关键矿物"，agent 得自己 join/手算。

- `[ ]` **语义视图命令（§12 Layer 2 落地）**：`--view sitrep` / `--view economy <faction>` /
  `--view fleet <faction>` / `--view frontier`（边缘失稳城：按 `governance_distance`+`loyalty`
  列出）/ `--view threat`（威胁评估：最近的敌对战力）/ `--view market`（富余/稀缺矿物）。由
  模拟直接生成（复用 `balance_picture`/`governance_distance`/`resource_value`），带参数校验与
  版本化，agent 不需知道 JSON 树。这是把「裸 jq 探 JSON」正式降为逃生舱的关键一步。
- `[ ]` **`--focus <faction>`**：把观察面收敛到某势力（其城市/舰/关系/预算），大幅降 token
  （现 `--index` 无此裁剪；§旧 REPL `control 3` 曾把 7578→1700 字符）。语义视图命令天然是
  单势力/局部视角，可一并做。

### 18.4 观察→决策→下发→观察 的闭环松散
要玩一段（30 月）需跨 4 条命令（`--index`/`--round`→写 python→写 diff→`--apply --save`），
且没有「当前计划」的持久化位置。

- `[ ]` **`--play <session>`（脚本化回合循环）**：一个会话目录/主键（`--start`+`--save`+一段
  `--round`），agent 每回合循环「读 metrics → 写 diff → apply → 存」；命令 `--play` 批量跑一段
  + 记录每步 diff（做成 `--traj` 同源的 steer 日志），让「讲一段被干预的故事」如 `--traj` 一样
  可复现、可检查。优先做一个**轻量脚本**：`planet_x --seed S --plan plan.jsonl`（plan 是
  「到某回合应用某 diff」的序列），一次性重放更省事。

### 18.5 长局 / 统计库（§17 候选落地）
- `[ ]` **`planet_xq` 统计函数库**：`series(faction, metric, every)`、`rolling_mean/max`、
  `leader_rotation()`、`hegemon_timeline()`、`war_durations()`、`gini(stockpile)`、`frontier()`、
  `threat()`。让 agent 直接调用而非每次手写 pandas；这是「上千回合仍多极」这套调研的自然工具。
- `[ ]` **`--profile <K>`（§15 极致画像的 Rust 版）**：每 K 月一行极简标量时序
  `{round,cities,ships,fleet_value,hegemon,power_share,wars}`，几乎零上下文先看走势，再放大。
- `[ ]` **lazy 加重型字段**：`ships[].components/component_hp`、`cities[].buildings`、超大
  `events`/`chronicle` 也标 lazy（§17 候选），让 agent 默认面更小。

### 18.6 已知「玩成什么样」的护栏该喂给 agent
agent（尤其想「称霸」的）会踩「单极→被联合制裁→反噬」。宜把设计护栏写进游玩文档/`--meta`
说明，让 agent 的决策与「上千回合多极博弈」这一目标对齐。

- `[ ]` **策略护栏入文档**：在 `.agents/agent-play.md` 里把「别过度造舰（maintenance 预留）、
  别长期单极（会招 coalition+制裁）、边地要 loyalty、想多极就靠合纵」写成明确提示——已加在
  §6，但可再给出**量化阈值**（如 `upkeep`/`production_value` 的可持续比值、`power_share`
  触发 coalition 的阈值），让 agent 有「数字红线」。

> 量化依据（`--seed 7`，`--index` + `planet_xq`）：①agent «中国» 野蛮扩张
> (`construction_budget` 硅3/碳9/铁10) → 13 月 `upkeep 79.5 > production_value 64.8` → 15 月城清零；
> ②稳重局（经济压住 + loyalty 2.0 + 防御）→ 120 月 11 城/份额 0.62 → 触发全球 coalition+sanction →
> 179 月崩回 1 城，世界仅 12 城/10 舰；③基线 AI 全程 20 城/48 舰、霸权在 矿联↔中国↔俄罗斯 轮换。
> 结论：**「玩得太好」会触发系统的均势反噬**——这正是主线想要的，agent 应学会在「份额涨」时
> 主动收手/制衡，而不是硬顶。
