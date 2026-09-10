# 《行星X》 Agent 游玩指南

> 给 **LLM agent** 的实操手册：如何读这个世界、如何下指令、如何观察到后果并调整。
>
> **分析层 = `planet_x --index` 投影 + `play/planet_xq`（uv / pandas）。jq 已移除，本指南一律
> 用 `planet_xq` 读数。** 逐回合全量 JSON（`--round`/`--traj`）仍在，但那用于外部任意的
> 时间轴查询 / 存档封包；agent 日常读世界用 `--index` 的 lean 投影。

---

## 0. 快速上手（30 秒跑一圈）

1. **读世界**：`planet_x --seed 42 --round N --index out/`，然后用 Python kit `planet_xq`
   （`cd play/planet_xq && uv sync`）一行拿决策视图：
   `q.faction_snapshot(r, "中国")` → 该势力的库存/外交/经济/舰队/所属城。
2. **下指令**：写一个 `{control:[...]}` diff（`--apply` 文件），把目标势力/舰/预算钉成
   `"mode":"Player"`，再 `planet_x --start ckpt.ron --apply diff.json --round K --save ckpt.ron`
   （`--start`/`--save` 保 RNG，可复现、可回滚）。
3. **看点子**：详细 loop、可选指令面、坑与边界，见下（这是本手册的正文）。

### 三条最容易踩的坑（先记住）

- **id 是「有名字的实体」的唯一名**（舰/城/势力/天体/定居点 = 它的名字），不是整数编号；
  `--apply` diff 里的 `city`/`ship`/`faction_id`/`body` 写名字。
  **唯一例外：`building` 是 u32 下标**，而且在**城内部**才唯一——同一个下标换个城就是
  另一栋楼。`invest_weights`/`build_weights` 里的 `city`+`building` 必须**配套**用。
- **`--apply` 会告诉你哪些叶片没落地——所以一定要读 stderr。** diff 里点名的实体不存在
  （舰已战沉/改名成 `长城2`、城已易主、building 下标来自别的城）时，那条叶片会被丢掉，
  但退出码仍是 0、stdout 仍是正常状态流。丢掉的东西以
  `{"code":"WARN_APPLY_SKIPPED", "applied":N, "skipped":[…带 path/value/reason…]}` 印在
  **stderr**。**没输出 = 全部落地**。不读它就会带着「命令已下达」的错觉玩下去。
- **别让舰队维护费越过生产**：造舰预算会被「维护费 ×4 预留」封顶，但**流水的产出 vs 流水的
  维护/治理**才是生死线——`--control-plan <faction>` 先看 `verdict`，`bleeding`（净流为负）时
  先扩产、再扩军。否则帝国会被造船潮拖垮（几十回合内从霸权塌成 1 城）。
- **别当永久单极**：某势力实力占比超阈值 → 全网合纵 + 经济制裁 + 治理成本放大，过度扩张必被
  「均势」拉回来。想世界健康就做「强而不独」，不是「称霸到底」。

> **「攻击」和「轰炸」不是指令。** 敌舰进入攻击半径、敌对城进入围城射程就**自动**开火/轰炸，
> 玩家与 AI 一视同仁（`sim` 里 `autocontrol::auto_combat`）。所以**没有** `target_ship` /
> `target_settlement` 这两种行为（它们已被移除）。你要下的是**移动/停泊**指令，接战自己会发生：
> 追袭某舰 = `follow` 那艘（敌舰）；守卫友舰 = `follow` 友舰；压向某城 = `dock_city`。

### 读世界在哪读、控制面在哪写

- **读（观察）**：`--index` 投影 + `planet_xq`（lean 主流 + 按 id 索引的 lazy 表：
  `ships`/`cities`/`factions`/`bodies`/`settlements`/`events`）。`factions` 表直接给每势力的
  `relations`（外交）/`resources`（库存）/自有城与舰；`ships` 给 effective 面板
  （attack/range/speed/upkeep/components）。要看 `State` 全量（relations 之外的原始结构）可
  `planet_x --round 0`（或 `--start ckpt --round 0`）拿单行完整 JSON。
- **写（控制）**：`planet_x --control` 拿可编辑模板，改进 `--apply` diff；`--control-plan
  <faction>` 先算「成本→收益」；`--control-schema` 查 diff 能写哪些字段。
- **写错了问谁**：`--apply` 的报错与丢弃报告**都在 stderr**。行为名写错会在**解析阶段**就
  报错并列出合法行为（退出码 10）；指着不存在的实体则报 `WARN_APPLY_SKIPPED`（退出码 0）
  ——两者都点名到叶（`中国.ship_orders[0]`）。**别猜，读 stderr。**

下面再给一份可直接跑的通用手把手示例（bash）：
```bash
# 1) 生成一轮投影（回合 0..N 的 lean 主流 + 按 id 索引的 lazy 表 + 投影 schema）
planet_x --seed 7 --round 60 --index out/

# 2) 用 planet_xq 读（uv 管理的 Python kit）
cd play/planet_xq && uv sync            # 首次：建 .venv（pandas）
uv run python -c "
import planet_xq
q = planet_xq.load('out')
print(q.facts[['round','metrics']].head())   # 每回合的轻盈决策视图
print(q.join('ships', round=10))             # 第 10 月所有舰（按 id join）
"

# 3) 决定一条指令，写成 diff 文件
# 4) 应用并推进，保存 checkpoint 以便复现续玩
planet_x --seed 7 --apply steer.json --round 30 --save ckpt30.ron
#    ⚠ 一定要看 stderr：{"code":"WARN_APPLY_SKIPPED",...} = 有叶片没落地（没输出 = 全落地）
```

---

## 1. 心智模型：你「玩」的是什么

- 每回合 = **1 个月**；一个 `State` 快照确定性推进（`sim::advance`），全部数值由
  `config/game.ron` 数据驱动。
- 世界：18 天体 / 22 定居点 / 9 势力 / 五级舰（护卫·驱逐·巡洋·航母·战列）。舰船是
  **势力动态定制**的（出厂时按资源优势选装组件，打完会修、会丢模块）。
- **你不是「打爆别人」而是「导一场上千回合仍有多方参与的博弈」。** 游戏刻意设置了反制
  机制：任何势力实力占比超过阈值 → 其余势力合纵（`coalition_members`）+ 经济制裁
  （`sanctioned`）+ 治理成本放大 → 过度扩张必被「均势」拉回来。**玩得「太独」会被系统教做人。**

### 想清楚这几个主轴
| 轴 | 关键量 | 玩法含义 |
|---|---|---|
| 存量/政治 | `power_share`、`hegemon`、`coalition_members`、`sanctioned`、`wars` | 谁是霸权、谁在抱团、谁被封锁、谁在打谁 |
| 经济 | `production_value`、`upkeep`、`governance_cost` | 撑不撑得起舰队、养不养得起帝国 |
| 治理 | 每城 `loyalty`、距首都距离 | 边地会不会叛乱夷平（`Revolt`） |
| 军事 | `ship_count`/`fleet_value`、舰级混编 | 打不打得动、守不守得住 |
| 剧情 | `chronicle`/`events` | 主线节拍（如「行星X 现身」）在哪些月发生 |

---

## 2. 读世界：一个回合的观察面

`--index` 投影里，每回合一行 **lean 主流**（`main.jsonl`），eager 字段（所有 id 都是字符串名）：

```jsonc
{ "round": 12, "time_month": 12.0,
  "events": [...], "chronicle": [...],
  "metrics": { "cities": 17, "ships": 26, "fleet_value": 578.0, "population": 8800,
               "power_share": {"中国": 0.13, ...}, "hegemon": "中国", "sanctioned": "中国",
               "coalition_members": ["美国", ...], "wars": [["美国","中国"], ...],
               "factions": {"中国": {"city_count":4,"ship_count":6,"fleet_value":120,
                                     "production_value":64.8,"upkeep":30.0,
                                     "governance_cost":4.8,"at_war":true}, ...},
               "city_production": {"长三角": {"production_value":28.4, ...}} },
  "ship_ids": ["长城", ...], "city_ids": ["长三角", ...], "faction_ids": ["中国", ...],
  "body_ids": ["地球", ...], "settlement_ids": ["长三角", ...] }
```

重型实体**不内联**，只在 `idx/*.jsonl` 表里、按 id（=名字）索引：

| 表 | 粒度 | 关键列 |
|---|---|---|
| `factions` | 每回合 | `faction_id resources(库存) relations(外交) capital_body alignment aggression home_* city_ids ship_ids` |
| `ships` | 每回合 | `ship_id faction_id class x y hull hull_max shield shield_max velocity components component_hp attack attack_range speed accel hardness intercept shield_regen hull_regen upkeep` |
| `cities` | 每回合 | `city_id faction_id body_id name population loyalty razed deployed_area building_count buildings[]`（`buildings[].id` = `--apply` 要的 u32 下标） |
| `bodies` | 全局一次 | `body_id name x y perihelion_distance aphelion_distance period settlement_count` |
| `settlements` | 全局一次 | `settlement_id body_id name total_area ecological_capacity construction_speed_mod resources` |
| `events` | 每回合 | `event_id("回合:序号") type headline weight salience actor_*/target_*/extra + data`（类型列叫 **`type`**） |

另有**派生表**（`idx/*.jsonl`，按同样的名字 join）：数据**不在状态里**，是引擎算出来的量——

| 派生表 | 粒度 | 关键列 | 回答什么 |
|---|---|---|---|
| `flow` | 每回合 × 势力 | `faction_id production{} upkeep governance_total governance_coverage` | 这回合产出/维护/治理到底是多少（`main` 的 `metrics` 里也有嵌套的一份，这是可 join 的平铺版） |
| `city_flow` | 每回合 × 城 | `city_id body_id faction_id razed production{}` | 每座城每回合在挖多少（含已夷平的空城） |
| `control` | 每回合 × 叶片 | `faction_id kind key sub value mode` | **谁在控制什么**（`kind` = ship_order/default_ship_order/default_doctrine/default_kiting/各类预算与权重/capital） |
| `scope` | 每回合 × 显式节点 | `level(global/faction/body/city) key mode` | 作用域树里谁有意见 |

`ships` 表另有几列是**引擎解析后的答案**，别自己重算链：`order_leaf_mode`、
`order_default_mode`、`order_effective_mode`、`order_effective`、`doctrine`、`kiting`。
单点查（不想跑整个 `--index`）：`planet_x --start ckpt.ron --derived` 给出这一回合存下来的
`{round, source, pre, post}`（`post.flow` 就是上面那张 flow 表的来源；没档时会按当前状态重算
并附 `note`，那种情况下 flow 是空的）。

**「我的舰为什么跑到那儿去送死？」** —— 那条指令是 AI 写的，判定过程**不发事件也不落状态**，
只有 `decisions` 表有：一行一条判定（`verdict` = `withdraw`/`engage`/`colonize`/`bombard`/
`move`/**`hold`**，`hold` = 这回合 AI **没给它派活**），`detail` 里是判定的**输入**
（`hull_ratio` vs `retreat_hull`、`kiting`、`enemy_in_range`）——"为什么"是这些数，不是它的自述。
`q.decisions(round=12)`；注意一艘舰一回合**最多两行**（先机动、到位后再判一次），
聚合前看 `detail.after_move`。

### planet_xq 快速配方
```python
import planet_xq
q = planet_xq.load("out")
q.facts                                  # 轻盈主流 DataFrame（每回合一行）
q.facts.metrics                          # 逐回合的总结指标（object 列）
q.factions(round=12)                     # 每势力：库存/relations/自有城与舰
q.faction_snapshot(12, "中国")           # 一键决策视图（metrics+库存+relations+自有城/舰）
q.ships(round=12)                        # 第 12 月全部舰（含 effective 面板）
q.cities(round=12)                       # 第 12 月全部城（含 buildings 清单）
q.bodies() ; q.settlements()             # 天体 / 定居点主表
q.decisions(round=12)                    # 本回合 AI 的判定（逐舰 verdict + 判定的输入 + 船坞改装）
q.fleet(12, "中国") ; q.city_buildings(12, "中国")   # 某势力的舰 / 城
q.ids("ships", 12)                       # 第 12 月的 ship_id（=舰名）数组
q.join("ships", round=12)                # explode 主流 ship_ids 并按 (round,id) merge 完整对象
```

常用判断：
```python
# 谁是霸权、谁被制裁
m = q.facts.iloc[-1].metrics
m["hegemon"], m["sanctioned"], m["coalition_members"], m["wars"]

# 我（中国）的走势
f = q.facts.metrics.apply(lambda m: m["factions"].get("中国", {}))
f.apply(lambda d: (d["city_count"], d["ship_count"], round(d["production_value"],1)))

# 我的外交 + 库存（重点：这能直接看会不会被制裁/缺什么矿）
snap = q.faction_snapshot(q.facts.iloc[-1]["round"], "中国")
snap["relations"]; snap["resources"]

# 边缘失稳城（忠诚低，易叛乱）
q.join("cities", round=12).query("faction_id=='中国' and loyalty < 0.5")

# 舰队维护费 vs 生产（翻车前看这个：upkeep > production → 先扩产）
snap["metrics"]["upkeep"], snap["metrics"]["production_value"]
```

> **语义视图（纯读取，把模拟算好的打包给你）**：`q.view_sitrep(12)`（世界政治：霸权/联盟/制裁/战争/
> 实力占比/各势力）；`q.view_frontier(12, "中国")`（我的失稳城，含 sim 算好的 `loyalty`/
> `gov_distance`/`revolt_risk`）；`q.view_market(12, "中国")`（库存按市场价）；`q.view_economy(12, "中国")`
> （产/维护/治理/净流/止血标记）；`q.resource_series("中国","铁")`（某资源逐月库存走势，看是否被抽干）。
> **这些只读、不重算游戏公式**。唯一要「游戏逻辑」判断的
> ——「我下令的造舰预算可持续吗？」——用 Rust 的 `--control-plan <faction>` 看 `verdict`，
> 不要在 Python 里自己估。

> 先 `--digest K --round N` 看整段走势的故事板，再对感兴趣窗口 `--index` 精读，别一把梭全量。

> **接手一局旧存档时**：先 `planet_x --start ckpt.ron --round 0 --notables 40` 读「这局最近在打什么」
> ——窗口层（`State::notables`）里是**后续计算要回看的那一段历史**（当前 = 开战/停战，供「记恨
> 地板」判定）。它随 checkpoint 存活，不需要当初的 `--index` 目录。
>
> ⚠ **不要**用 `--milestones` 当「这局发生过什么」——**按当前判据那一层是空的**（`count: 0`）。
> 判据是「后续计算需要访问哪一段历史」，不是「重要性」：没有任何模拟逻辑读无限过去，所以没有
> 事件属于它。要读「一座城的一生 / 谁打沉的谁」，用 **`--index` 投影**：
> `q.history("city", 城名)` / `q.cause("ship", 舰名)` / `q.storyboard(window)`（按 `weight`
> 压成故事板）。**「重要 ≠ 分层」**——想挑值得读的事件看 `weight` 列，别看 `salience`。

---

## 3. 决策面：你能控制什么（指令 / control）

每个势力有一个 `ControllableState`，叶子都带 `mode`（**三态**）：`"Player"`（你说了算，系统
只读你的值）/ `"Auto"`（系统自动决定）/ `"Inherit"`（这一层没有说话，往上继承）。
`scope` 是一棵「谁负责」的作用域树（全局→势力→天体→城市）。旧档里的 `"Ai"` 与 `null`
分别按 `Auto` / `Inherit` 读入（`SCHEMA_VERSION` v4→v5 的零损失映射）。

**归属链（舰的指令与风格都是这条）**：`叶 → 舰队默认 → 势力 scope → 全局 scope`，
最具体的那层**有意见**（`Player`/`Auto`）就它说了算；一路 `Inherit` 就到 `Auto`。
所以「新舰出厂归谁、干什么」的答案是**舰队默认**，不必逐舰点名。

| 指令面 | 含义 | 关键点 |
|---|---|---|
| `ship_orders` | 每艘舰的**移动/停泊**行为 | `Idle / Move / Follow / DockCity / Dock / Colonize`（见下） |
| `default_ship_order` | **舰队默认指令**（势力级一片） | `{"behavior":…,"mode":…}`。**新舰出生就继承它**；一次性指令执行完也回落到它 |
| `default_doctrine` | **舰队默认行为风格**（势力级一片） | `{"temper":…,"lone_wolf":…,"mode":…}`。全舰队一个风格 = 一片叶 |
| `default_kiting` | **舰队默认风筝↔贴脸**（势力级一片） | `{"kiting":…,"mode":…}` |
| `ship_doctrine` | 每舰**行为风格**（per-舰叶片） | `temper`（理智↔热血，欺软怕硬↔飞蛾扑火）、`lone_wolf`（护航↔独狼），各 `[-1,1]`、`0`=基线 |
| `ship_kiting` | 每舰**风筝↔贴脸**姿态（per-舰叶片） | `[-1,1]`、`0`=基线。**软属性**：Move/Follow/Dock/Idle 都是软目标，附近有敌舰时自动微调位置，**玩家也不能硬控制** |
| `investment_budget` | **建设**投资预算（每资源 / 月） | 用于建建筑、扩生产 |
| `construction_budget` | **造舰**建造预算（每资源 / 月） | 用于造舰；会先给维护费留**预留**（见下） |
| `invest_weights` | 各建设任务优先级 | 谁先吃投资预算。key = `city` + `building`（`building` 是**城内的 u32 下标**） |
| `build_weights` | 各建造区优先级 | 哪个船坞先造。key 同上 |
| `loyalty_budget` | 每城娱乐/福利（月） | 提「忠诚」压低叛乱 |
| `capital` | **迁都**：换首都天体 | `{"value":"<天体名>","mode":"Player"}`；首都=光速治理/本土防御锚点 |
| `buildings` | 结构性增删改建 | 加/删建筑、改 `structure`、改 `ship_type`（只对建造区有效） |

> **风格与指令是同一个形状**（`--apply` 里的两片名字不同，语义同构）：写值即接管、
> 缺省轴保留现值、`mode` 显式给出时以它为准。
> **读面里的风格值是「有效值」**（叶 → 舰队默认 → 舰上记录值），而 `mode` 是**叶片自己的
> 表态**——所以整面 dump 回来安全；但**改值请把 `mode` 一起写成 `Player`/`Auto`**，
> 只改值而留 `Inherit` 等于说"这一层没有意见"（除非舰队默认也是 `Player`，那个值不会被采用）。

#### `ship_orders` 的六种行为（**攻击/轰炸不在其中**）

| 行为 | 写法 | 干什么 |
|---|---|---|
| `Idle` | `"Idle"` 或 `{"type":"idle"}` | 原地保持（不移动）。**不会停止开火**——射程内照样自动接战 |
| `Move` | `{"type":"move","position":[x,y]}` | 驶向一个 2D 坐标（AU） |
| `Follow` | `{"type":"follow","ship":"<舰名>"}` | 持续驶向某舰当前位置。**所随的可以是友舰（护航）也可以是敌舰（追袭）**；跟随本身不开火，但射程内自动接战 |
| `DockCity` | `{"type":"dock_city","city":"<城名>"}` | 驶向某城；**若该城敌对且进入围城射程则自动轰炸** |
| `Dock` | `{"type":"dock","body":"<天体名>"}` | 跟随某天体轨道巡航/停靠（**守家最常用**） |
| `Colonize` | `{"type":"colonize","body":"<天体名>"}` | 前往定居点天体并（再）建一座城 |

- **守卫友舰 = `follow` 那艘友舰**（没有 `guard`/`guard_ship` 这个类型；旧手册里的
  `TargetShip{attack:false}` 已随行为重构移除）。
- **追袭敌舰 = `follow` 那艘敌舰**（旧写法 `TargetShip{attack:true}` 同样已移除）。
- 目标舰/城**消失**时（战沉、夷平），sim 会把该指令降级成 `Idle` 并发一条 `StaleOrder`
  事件——所以「指令还在不在」可以从 `--index` 的 `events` 表里查 `stale_order`。

> **迁都的代价**：`mode=Player` 时你说的算、AI 不覆盖（除非首都亡城——硬规则仍强迁到
> 人口最高的活城）。自动控制的势力每 `capital_review_every` 回合重估：候选=人口最高的活城，
> 仅当它让全势力各城的总治理距离成本低 `capital_relocate_threshold` AU 以上才迁。迁都按
> **旧首都人口占比**扣全国忠诚（`capital_share_relocate_cost`，占比越大越动荡）；首都人口
> 占比越高，全国每城目标忠诚又加成（`capital_share_loyalty_buff`）。所以—想稳定就把首都
> 放在人口中心，但迁都是豪赌不是免费优化。

### 四条必须懂的语义
1. **两条预算独立、按权重竞争**：`investment_budget` 建「楼」，`construction_budget` 造「舰」；
   各自内部按权重（`invest_weights`/`build_weights`）分钱，互不竞争。
2. **造舰先给维护费留底**：`upkeep × upkeep_reserve_mult`(≈4) 的市场价值**先被预留**，
   剩下的才用于造舰——**别把造舰预算拉满到经济承载之上**，否则维护费拖垮经济、引发治理
   崩溃与叛乱（这是最常见的翻车方式：造一堆养不起的船，帝国崩给你看）。
3. **舰型 ≠ 预算**：某建造区造什么由该建筑的 `ship_type`（如 `corvette`/`battleship`）决定，
   不是靠提高预算自动变高级舰。要让「整支舰队随威胁重构」，得改 `buildings[].ship_type` 或
   用 `build_weights` 加权。**注意 `ship_type` 只对建造区（`is_shipyard`）有效**，写在开采区
   上会被丢弃并报 `not_a_shipyard`。
4. **攻击是自动的，指令只管「去哪」**：见 §0 的提示。想让舰队「守住地球」就 `dock` 地球，
   而不是找一条「攻击」指令——敌舰进射程会自动打。

> 想**整体接管**一个势力：`{"scope":{"factions":[["中国","Player"]]}}` —— 但这**管不了已经
> 自己有叶片的舰**（叶比 scope 更具体）。要让全舰队真正听话，两条一起做：
> ① 势力级**舰队默认**（`default_ship_order` / `default_doctrine` / `default_kiting`）=
> 新舰与"没说话"的舰的答案；② 逐舰把叶片交回上层（`{"ship":"长城","mode":"Inherit"}`，
> 只写 mode 不动值）或钉成 `Player`。
> **写值即接管**：diff 里只写值、不写 `mode` ⇒ 那片叶变 `Player`（回执 `NOTE_APPLY_TOOKOVER`
> 会点名）。想只改"流水记录"而不接管，显式写 `"mode":"Auto"`/`"Inherit"`。
> 旧手册那句「注意这会让新造出来的舰默认 Idle」现在有了正解：**给势力设舰队默认**，
> 新舰出厂就继承意图，不必每段重新点名。

---

## 4. 下一条指令：写 `--apply` 的 diff

`--apply <file.json>` 接受 `{control:[...], scope:{...}}`（与 web `POST /api/command` 同形）。
它是**多层级结构化补丁**：只触碰 diff 里出现的势力/叶子；某个叶子省略 `value`/`behavior`
保留当前值；**省略 `mode` 时：写了值就接管（变 `Player`），什么都没写才保留当前模式**。
风格轴同理（`ship_doctrine` / `ship_kiting` / `default_doctrine` / `default_kiting`）。

### 4.1 先从模板改
```bash
planet_x --seed 7 --control    # 整面可编辑模板（每势力：ship_orders/预算/权重/scope）
# 这是「整面」模板（所有势力都在）；挑出你要改的那几片叶子写进 diff 即可。
# 想看/控制面收敛到单个势力：见 agent-play.md 的 §3/§4（按 scope 接管），或先 `--index` +
# planet_xq 的 q.faction_snapshot(r, 名字) 只读你关心的那个势力。
```

### 4.2 diff 实例（中国：追袭、守家、稳忠诚——**局部接管**）
> **有名字的实体一律用名字**（舰/城/势力/天体）。以下用真实开局实体：中国=`中国`、舰=
> `长城`/`赤霄`/`北斗`、城=`长三角`、美国舰=`华盛顿`。`building` 例外，是**城内的 u32 下标**。
```jsonc
{
  "control": [{
    "faction_id": "中国",
    "construction_budget": [
      {"resource": "硅", "value": 1.0, "mode": "Player"},
      {"resource": "碳", "value": 2.0, "mode": "Player"},
      {"resource": "铁", "value": 2.5, "mode": "Player"}
    ],
    "loyalty_budget": [ {"city": "长三角", "value": 2.0, "mode": "Player"} ],
    "ship_orders": [
      {"ship": "长城", "behavior": {"type": "follow", "ship": "华盛顿"}, "mode": "Player"},
      {"ship": "北斗", "behavior": {"type": "follow", "ship": "赤霄"},   "mode": "Player"},
      {"ship": "赤霄", "behavior": {"type": "dock",   "body": "地球"},   "mode": "Player"}
    ]
  }]
}
```
- 这是**局部接管**：只把「造舰预算 / 某城忠诚 / 三艘舰」钉成 `Player`，其余（投资、其它舰、
  生产）仍交给 AI。
- 舰"长城"去**追袭**美国舰"华盛顿"（`follow` 一艘**敌**舰 = 追袭/接战）；
  舰"北斗"**守卫**"赤霄"（`follow` 一艘**友**舰 = 护航）。
- "赤霄"自己 `dock` 地球 = 回本土驻守。**三艘舰都在射程内自动开火**——你不需要（也不能）
  写「攻击」。
- ⚠ **新造出来的舰不在 diff 里**：它们的指令取决于**舰队默认**（`default_ship_order`）。
  只接管几艘舰时，"新舰谁来指挥"由那一片叶回答——所以**接管单个势力时，第一件事通常是
  写一片舰队默认**：
  ```jsonc
  {"control":[{"faction_id":"中国",
    "default_ship_order":{"behavior":{"type":"dock","body":"地球"},"mode":"Player"},
    "default_kiting":{"kiting":-1.0,"mode":"Player"}
  }]}
  ```
  这两片一写：**所有"没有说话"的舰（含以后下水的）都按它走**，单舰特例仍写在 `ship_orders`。
- 若想**整体接管**一个势力（所有叶子都归你），写
  `{"scope":{"factions":[["中国","Player"]]}}`。注意**叶比 scope 更具体**：已经自己有叶片的舰
  不会被 scope 翻转，要逐舰写 `{"ship":"长城","mode":"Player"}`（只写 mode，不动值）或
  `"Inherit"`（交回上层）。整面接管意味着经济/造舰决策也归你扛。

### 4.3 behavior 两种写法都认
- **tagged 形式**（就是你从舰的 `order` 字段里看到的）：`{"type":"follow","ship":"华盛顿"}`、
  `{"type":"idle"}`、`{"type":"dock","body":"地球"}`、`{"type":"dock_city","city":"长三角"}`、
  `{"type":"move","position":[-0.5,0.3]}`、`{"type":"colonize","body":"火星"}`。
- **默认枚举形式**：`"Idle"`、`{"Follow":{"ship":"华盛顿"}}`、`{"Dock":{"body":"地球"}}`。
- 把 `--control` 里的 `order` 原样粘进 diff 即可（`--apply` 会自动归一化）。
- **写错 tag 会当场报错**（退出码 10），错误里会给合法 tag 全表；若你写的是已移除的
  `target_ship`/`target_settlement`，错误里还会直接给出替代写法。**不要靠试错猜行为名——
  读报错，或先 `--control-schema` 看 `ShipBehavior` 的 `oneOf`。**

### 4.4 关键：**有名字的用名字，`building` 用下标**
`city`/`faction`/`ship`/`body`/`settlement` 一律用**名字**（唯一名），不是整数编号。
**唯一的例外是 `building`**：它是 u32 下标，**只在所属城内部唯一**，所以
`invest_weights`/`build_weights` 的 `{"city":…,"building":…}` 必须配套——同一个下标换座城
就是另一栋楼。查 id：`planet_x --control`（可编辑模板，含每城的 building 下标与它的
`kind`/`ship_type`）、`--index` + `planet_xq` 的 `q.cities(r)`（`buildings[]` 里带 `id`）；
资源 key 同理（WYSIWYG，状态里的「铁」就是 diff 里的「铁」）。

### 4.5 `--apply` 的回执：**读 stderr**
`--apply` 的语义是「只触碰 diff 里出现的叶片」，所以引用一个已经不存在的实体**不是错误**
——但它的后果和「成功」在 stdout 与退出码上一模一样。因此：

| stderr | 退出码 | 含义 |
|---|---|---|
| （空） | 0 | 全部落地 |
| `{"code":"WARN_APPLY_SKIPPED","applied":N,"skipped":[…]}` | 0 | diff 本身合法，但有些叶片**没落地**；每条带 `path`（点名到叶）、`value`、`code`、`reason` |
| `{"code":"ERR_APPLY","message":…}` | 10 | diff **没被接受**（行为名非法、字段名拼错、JSON 坏了）；`message` 点名到叶 |

`skipped[].code` 的取值与含义：

| code | 意思 / 怎么办 |
|---|---|
| `no_such_ship` | 舰名不存在——多半是**战沉后换代**了（`长城` → `长城2`）。重新查名再下 |
| `not_your_ship` | 那舰存在，但不是这个势力的 |
| `no_such_faction` | 势力名打错。**从前这会凭空造出一个幽灵势力污染 `--control`，现在整条被丢弃** |
| `no_such_city` / `not_your_city` | 城不存在（被夷平后从活城列表消失）／不是你的 |
| `no_such_building` | 该城里没有这个下标；`reason` 会列出**该城真实的下标列表** |
| `no_such_resource` | 资源 key 写错（用 `--meta` 的 `resources` 看全表） |
| `not_a_shipyard` / `not_a_mining_building` | 把 `ship_type` 写在非建造区上／把 `resource` 写在非开采区上 |
| `no_such_kind` / `no_such_structure` | 建筑类型 / 结构名非法，`reason` 列出可选项 |
| `no_such_body` | 天体名非法（迁都） |

> 典型的「以为自己下达了」：`q.fleet` 里查到舰叫 `长城`，下了指令，但那艘在**上一回合**
> 刚被击毁——`applied:0` + `no_such_ship` 就是唯一能让你发现这件事的信号。

---

## 5. 游玩循环（观察→决策→应用→**读回执**→复现）

```
观察  planet_x --seed 7 --index out/ ;  planet_xq 读
决策  写 diff.json（见 §4）
应用  planet_x --seed 7 --apply diff.json --round 30 --save ckpt30.ron 2>apply.jsonl
回执  看 apply.jsonl：WARN_APPLY_SKIPPED = 有叶片没落地（见 §4.5）。别跳过这一步
再看  planet_x --start ckpt30.ron --round 0 --notables 40  （这局最近在打什么，一句话一条）
续玩  planet_x --start ckpt30.ron --round 30 --index out2/   （续玩 + 精读）
```

- **确定性**：同 seed（或同 checkpoint + 已保存 RNG 位置）→ 后续逐字节一致。
  `--save`/`--start` 是分段续玩 / 复现的关键；`--start` 也接受旧单 `State` `.ron`（此时重播
  seed、随机流不延续）。
- **`--save` 在 `--index` 模式下也生效**：`--start ckpt --apply diff.json --round 30 --save ckpt.ron
  --index out/` 一次同时拿到「续玩存档 + 精读投影」，不用跑两遍。
- **省 token**：先 `--digest K --round N`（每 K 月一行语义故事板），锁定要放的窗口，再
  `--index` 精读那一段，最后 `--round`/`--apply` 介入。别把 `--round 3000` 的全量 JSON 灌进上下文。
- **分段介入的推荐姿势**：先用**同一条 seed 的基线投影**看清世界怎么走，再决定在哪个回合
  切进去——`--apply` 与不 `--apply` 是同一条确定性时间线，所以「有我的干预」与「没我的干预」
  可以直接逐回合对比（这也是唯一能证明「我的决策真的改变了结果」的办法）。

---

## 6. 策略要点（设计护栏，别踩）

1. **别把一家玩成永久单极**：某势力占比过高 → `coalition_members` 抱团 + `sanctioned` 制裁 +
   治理/行政成本放大（`sanction_cost_mult`）→ 边地更贵、更易叛离。想让世界健康，就让「霸权
   轮换」而不是「称霸到底」。
2. **别过度造舰**：`upkeep` 是每月固定开销；造舰预算会被 `upkeep×4` 预留封顶。经济撑不起的舰队
   会反过来拖垮帝国（生产 < 维护 + 治理 → 叛乱连环）。**先 `--control-plan <faction>` 看
   `verdict`**：`bleeding`（净流为负）时停扩军、先扩产；`over-committed` 表示你的命令造舰预算
   超过了 AI 的保守上限（维护费预留后），大概率会持续失血。
3. **边地要喂忠诚**：治理模型按「距首都距离 × 人口超载」叠惩罚；低忠诚城会 `Revolt` 夷平为空白。
   给偏远/新占城市投 `loyalty_budget`；必要时放弃过度扩张的远端殖民地。
4. **多极的杠杆**：弱者抱团制衡霸权是系统自动的（合纵/遏制/集体安全/制裁）——你的角色通常是
   「在合适时机加入或维持制衡」，而不是靠武力硬吃。
5. **别只看城数**：`production_value`/`upkeep`/`governance_cost` 的差值决定一个帝国能不能撑过
   制裁期；`power_share` 决定它何时招恨。

### 6.1 红线是量出来的，不是猜的（`--seed 7` 实测）

> 下面这组数字是**一次真实的 agent 游玩**留下的（同一 seed，基线 vs 被引导，逐回合可对上）。
> 它们不是平衡常数，而是「这套机制在这条 seed 上到底怎么咬人」的坐标。

| 读数 | 什么时候算危险 | 实测 |
|---|---|---|
| `net_flow` = `production_value − upkeep − governance_cost` | < 0（`--control-plan` 的 `bleeding`） | 见下 |
| `upkeep / production_value` | **> 0.8 就该收手** | r75 时 `97.9 / 83.1 = 1.18` → 15 个月后帝国只剩 1 城 |
| `power_share` | **≳ 0.5 必招全网合纵** | r60 `0.51` → r90 被削回 `0.02` |
| `governance_coverage` | < 1 表示边地已失控（看 `view_frontier`） | — |

**两条实测轨迹**（同一个 `--seed 7`，同一套机制，只有「有没有 agent 引导」不同）：

- **被动基线**：`中国` 出兵去追打强敌 → 舰队在 r20–30 全灭（`0` 舰）→ r29–34 四座城被
  欧盟/美国舰队逐个夷平 → r35 只剩 1 城，此后 AI 继续造养不起的舰（`upkeep` 一路涨到 53）。
- **被引导**：把舰钉在**本土**（`dock 地球`）并调成**风筝姿态**（`ship_kiting: -1.0`）→
  r30 时 6 艘舰**满血**（基线 0 艘）、4 城全在、世界仍是多极（无霸权）。
  **但**：一路赢下去到 r60 变成 **11 城 / 份额 0.51** → 招来全网合纵 + 制裁 →
  r75 `upkeep 97.9 > 产出 83.1` → **r90 崩回 1 城**。

> **结论（这是主线想教你的事）**：`中国` 的死法**不是**「太弱」，而是**两种同样的过度**——
> 先是「舰队冲出去打光」，后是「赢到招恨 + 造到养不起」。agent 要练的不是「怎么赢」，
> 而是**在份额涨起来、维护费逼近产出时主动收手/制衡**。每逢 20–30 回合跑一次
> `--control-plan <自己>`，把 `verdict` 当刹车灯看。

---

## 7. 命令行速查（当前接口）

| 参数 | 作用 |
|---|---|
| `--seed <S>` | 确定性种子（数字），默认 `random` |
| `--start <ckpt.ron>` | 从 checkpoint（State+RNG）续玩 |
| `--round <N>` | 输出 N+1 行全量状态 JSON（回合 0 先） |
| `--traj <N>` | 一个自包含 `{schema_version,meta,story,trajectory:[...]}` |
| `--apply <diff.json>` | 叠加控制 diff 后继续 |
| `--save <ckpt.ron>` | 结束后写 checkpoint |
| `--meta` | 游戏规则字典（resources/buildings/ships/economy/combat…） |
| `--schema` | 状态视图的 JSON Schema |
| `--story` | 剧情编年史（叙事弧） |
| `--notables [<N>]` | **窗口层**：后续计算要回看的那一段历史（当前 = 开战/停战），带窗口宽度；`N` = 只出最近 N 条。**续玩前先读它** |
| `--milestones [<N>]` | **里程碑层**：后续计算需要**无限过去**的事件。**按当前判据为空（`count: 0`）**——见下方「接手旧存档」那条警告；要读一整局的历史用 `--index` + `planet_xq` |
| `--control` | 可编辑控制面模板。**读面不舍入**：里面的数就是状态里存的数（逐位），所以"原样回传"是**无损**的——只改你想改的那几行 |
| `--control-schema` | `--apply` diff 能写哪些字段的 JSON Schema |
| `--derived` | 这一回合存下来的派生态 `{round, source, pre, post}`（`post.flow` = 本回合产出/维护/治理的中间量；`post.flow.decisions` = **本回合 AI 的判定**；与 `--index` 的同名派生表同值） |
| `--control-plan [<faction>]` | 给势力算「成本→收益」（产出/维护/治理/净流/可养舰上限/清算倒计时） |
| `--every <K>` | 每 K 回合一个全量快照（降采样） |
| `--digest <K>` | 每 K 回合一行语义故事板（世界/各势力/战争/事件计数/剧情节拍） |
| `--index <DIR>` | 投影成 lean 主流 + lazy 表（ships/cities/factions/bodies/settlements/events）+ **派生表**（flow/city_flow/control/scope/decisions）+ schema.json（planet_xq 读） |

> 注意：**没有交互式 REPL**、没有 `--query`。这正是设计：stdout 零噪声、确定性、可复现；
> 分析在外部（planet_xq / pandas）做，控制走 `--apply` diff。
> **`--apply` 的回执在 stderr**（`ERR_APPLY` 退出码 10；`WARN_APPLY_SKIPPED` 退出码 0）——
> 用 `2>file` 收好它，别 `2>/dev/null`（那等于把「命令没下达」这条唯一线索扔掉）。

---

## 8. 坑与边界

- **jq 已移除**：分析用 `--index` + `planet_xq`；不要假定 `jq` 在 PATH。
- **`--round`/`--traj` 是全量快照**（几千回合会撑爆上下文）；省 token 用 `--index` + `--digest`。
- **别把 `--apply` 的 stderr 丢掉**：`WARN_APPLY_SKIPPED` 是唯一能告诉你「这条命令没有落地」
  的信号（见 §4.5）。stdout 永远只是状态流，成功时不打印任何回执。
- **有名字的实体用名字**（舰/城/势力/天体/定居点 = 唯一名）；`--index` 的 lazy 表、`--apply`
  diff、`metrics.factions` 的 key 全用名字，不是整数。**例外：`building` 是城内的 u32 下标。**
  同一实体在扩张/重建后名字可能变（如 "长城2"），拿它当 key 用，别假设数字下标或长期稳定。
- **舰/城会死，名字会换代**：`--apply` 点名的舰如果已战沉，那条指令会被丢弃（报
  `no_such_ship`）。玩长局时**每回合重新查一次名字**，别把上一回合的 diff 直接重放。
- **外交/库存看 `factions` 表**：`--index` 的 `idx/factions.jsonl` 直接给每势力的
  `relations`（两两关系）与 `resources`（库存），`q.faction_snapshot(r, 名字)` 一行拿全——
  不用再为了看外交/库存去 `--round 0` 全量 dump。
- **`metrics.factions` 是 dict（key= faction id 的字符串名）**，不是数组；`production` 是每资源
  dict，总产出用 `production_value`。
- **`chronicle` 是累计的**（每行都带整段叙事弧到当回合），逐行读会重复；按 `(round, id)` 去重。
- **`q.view_*` 的返回类型不统一**：`view_sitrep`/`view_market`/`view_economy` 是 dict
  （`view_sitrep` 里**嵌着 DataFrame**，直接 `json.dumps` 会炸），`view_frontier` 是 DataFrame。
  按需 `.to_dict()`/`to_string()`，别假设能一把 `json.dumps`。
- **`events` 的类型列叫 `type`**（不是 `kind`）；`q.events(type="siege")` 直接按类型取（字段稠密），
  实体历史用 `q.history(kind, id)`（这里的第一个参数才叫 `kind`，取 `"ship"`/`"city"`）。
- **回合 0 未步进**，`metrics` 流量为 0；看真实流量从回合 1 起。
