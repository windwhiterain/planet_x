# 《行星X》 Agent 游玩指南

> 给 **LLM agent** 的实操手册：如何读这个世界、如何下指令、如何观察到后果并调整。
>
> **分析层 = `planet_x --index` 投影 + `play/planet_xq`（uv / pandas）。jq 已移除，本指南一律
> 用 `planet_xq` 读数。** 逐回合全量 JSON（`--round`）仍在，但那用于外部任意的
> 时间轴查询 / 存档封包；agent 日常读世界用 `--index` 的 lean 投影。

---

## 0. 快速上手（30 秒跑一圈）

1. **读世界**：`planet_x --seed 42 --round N --index out/`，然后用 Python kit `planet_xq`
   （`cd play/planet_xq && uv sync`）一行拿决策视图：
   `q.faction_snapshot(r, "中国")` → 该势力的库存/外交/经济/舰队/所属城。
2. **下指令**：写一个 `{control:[...]}` diff（`--apply` 文件），把目标势力/舰/预算钉成
   `"mode":"Player"`，再 `planet_x --start ckpt.json --apply diff.json --round K --save ckpt.json`
   （`--start`/`--save` 保 RNG，可复现、可回滚）。
3. **看点子**：详细 loop、可选指令面、坑与边界，见下（这是本手册的正文）。

### 三条最容易踩的坑（先记住）

- **id 是「有名字的实体」的唯一名**（舰/城/势力/天体/定居点 = 它的名字），不是整数编号；
  `--apply` diff 里的 `城`/`舰`/`势力`/`天体` 写名字。
  **唯一例外：`建筑` 是 u32 下标**，而且在**城内部**才唯一——同一个下标换个城就是
  另一栋楼。`建设权重`/`建造权重` 里的 `城`+`建筑` 必须**配套**用。
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
  ——两者都点名到叶（`中国.指令[0]`，路径用的是**中文叶名**）。**别猜，读 stderr。**

下面再给一份可直接跑的通用手把手示例（bash）：
```bash
# 1) 生成一轮投影（回合 0..N 的 lean 主流 + 按 id 索引的 lazy 表 + 投影 schema）
planet_x --seed 7 --round 60 --index out/

# 2) 用 planet_xq 读（uv 管理的 Python kit）
cd play/planet_xq && uv sync            # 首次：建 .venv（pandas）
uv run python -c "
import planet_xq
q = planet_xq.load('out')
print(q.facts[['round','view']].head())      # 每回合的轻盈决策视图
print(q.join('ships', round=10))             # 第 10 月所有舰（按 id join）
"

# 3) 决定一条指令，写成 diff 文件
# 4) 应用并推进，保存 checkpoint 以便复现续玩
planet_x --seed 7 --apply steer.json --round 30 --save ckpt30.json
#    ⚠ 一定要看 stderr：{"code":"WARN_APPLY_SKIPPED",...} = 有叶片没落地（没输出 = 全落地）
```

---

## 名词与解释（`--nouns`）

读面上每个**名词**（字段名/列名）都有一句解释，`planet_x --nouns` 一次发全
（`{state, view, projection, control}` 四半，与 `GET /api/schema` 同实现）。
「这个字段是什么意思」先查它，别猜、也别去翻源码里的结构体。

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
  "view": { "city_count": 17, "ship_count": 26, "fleet_value": 578.0, "population": 8800,
            "power_share": {"中国": 0.13, ...}, "hegemon": "中国", "sanctioned": "中国",
            "coalition_members": ["美国", ...], "wars": [["美国","中国"], ...],
            "factions": {"中国": {"city_count":4,"ship_count":6,"fleet_value":120,
                                  "production_value":64.8,"upkeep":30.0,
                                  "governance_cost":4.8,"governance_coverage":1.0,
                                  "net_import":-12.5,"at_war":true}, ...},
            "cities": {"长三角": {"population":410,"loyalty":0.86,
                                  "production_value":28.4, ...}},
            "decisions": { ... } },
  "舰名表": ["长城", ...], "城名表": ["长三角", ...], "势力表": ["中国", ...],
  "天体名表": ["地球", ...], "定居点表": ["长三角", ...] }
```

> `view` = 这一回合的**一份视图**（与 `--derived` 的 `post`、`--schema` 里 `Trajectory.view` 同构）：
> 世界总量/政治/市场（观测）+ **每势力一行、每城一行**——一行里**同时**装着一个势力的观测
> （城数/舰数/人口/库存价值/是否交战）与**本回合的过程量**（产出/维护费/治理/贸易）。过程量在
> 回合开始的那份视图（`pre`）里是 0/空：那不是「没有」，是「还没算」。

重型实体**不内联**，只在 `idx/*.jsonl` 表里、按 id（=名字）索引：

| 表 | 粒度 | 关键列 |
|---|---|---|
| `factions` | 每回合 | `势力 符号 capital_body 阵营倾向 好战度 本土半径 本土攻击倍率 本土再生加成 思潮 资源(库存) 关系(外交) 名声 MOND 掌握度 mond_ships_in_band mond_frontier_au war_quota … 城名表 舰名表`（写世界的那几条 `*_quota` 逐列解释见 `--nouns` 的 `derived.factions.column_docs`） |
| `ships` | 每回合 | `舰名 势力 舰级 x y 船体 船体上限 护盾 护盾上限 速度 组件 组件耐久 attack attack_range speed accel hardness intercept shield_regen hull_regen 载货 cargo_capacity upkeep order_leaf_mode order_effective_mode order_effective order_source 风格 姿态 角色 role_mode 出厂图 blueprint_mode 下水回合` |
| `cities` | 每回合 | `城名 天体名 定居点 势力 人口 忠诚度 已焚毁 deployed_area building_count 建筑 gov_distance depot_value revolt_risk`（`建筑[]` 里每栋有 `建筑编号`（= `--apply` 要的 u32 下标）`类型 开采资源 建造舰级 设计图 结构 面积` …逐列见 `--nouns`） |
| `bodies` | 全局一次 | `天体名 近日点距离 远日点距离 公转周期 x y settlement_count` |
| `settlements` | 全局一次 | `定居点 天体名 index 总面积 生态容量 建设速度修正 建设资源修正 资源` |
| `events` | 每回合 | `event_id("回合:序号") type headline weight salience actor_*/target_*/extra + data`（类型列叫 **`type`**） |

另有**派生表**（`idx/*.jsonl`，按同样的名字 join）：数据**不在状态里**，是引擎算出来的量——

| 派生表 | 粒度 | 关键列 | 回答什么 |
|---|---|---|---|
| `faction_process` | 每回合 × 势力 | `势力 production{} upkeep governance_total governance_coverage` | 这回合产出/维护/治理到底是多少（`view.factions[<势力>]` 是**同一个来源**的另一份读法，这是可 join 的平铺版） |
| `city_process` | 每回合 × 城 | `城名 天体名 势力 已焚毁 production{}` | 每座城每回合在挖多少（含已夷平的空城） |
| `control` | 每回合 × 叶片 | `势力 kind key sub value mode` | **谁在控制什么**（`kind` = **上面那张表的中文叶名**，逐字等于 `--control-schema` 的 `leaves[].field`；两边由 `play/tests/g4_spec.py` 对账）。⚠ **没有舰队默认指令**那一片（2026-10 删除：指令是即时操作，只写逐舰叶）。⚠ **设计图库不在本表**（它是结构叶，见下一行） |
| `scope` | 每回合 × 显式节点 | `level(global/faction/body/city) key mode` | 作用域树里谁有意见 |
| `blueprints` | 每回合 × 设计图 | `势力 图名 舰级 选装[] 风格{} 姿态 角色 mode effective_mode ship_count class_slots component_cost launch_waiting` | 这个势力的**设计图库**：一张图 = 「还不存在的舰」的出厂规格（舰级 + 选装 + **倾向三轴**：风格 / 风筝姿态 / 角色）。`ships.blueprint` 与 `cities.buildings[].blueprint` 都 join 它 |

`ships` 表另有几列是**引擎解析后的答案**，别自己重算链：`order_leaf_mode`、
`order_default_mode`、`order_effective_mode`、`order_effective`、**`order_source`**
（= **这条意图是谁供的值**：`leaf` / `blueprint:<图名>` / `fleet_default`；`scope`/`record`
在指令链上不会出现）、`doctrine`、`kiting`、`blueprint`（本舰是哪张图印出来的）、
`spawned_round`（下水回合；`null` = 旧档 ⇒ 未知）。
⚠ `order_source` 把「**叶不存在**」与「叶写着 `Inherit`」**分开报**：后者报 `leaf`——那时值
真的来自那片叶（`leaf.map(|l| l.value).unwrap_or(..)`），只有叶不存在才可能落到图/舰队默认。
单点查（不想跑整个 `--index`）：`planet_x --start ckpt.json --derived` 给出这一回合存下来的
**视图对** `{round, source, pre, post}`——两个槽都是 `RoundView` 且**同形**：`pre` = 回合开始时
看到的世界（过程量全 0/空），`post` = 回合结束时的世界 **+ 本回合过程量**（就是上面那几张派生表
的来源）。没档时会按当前状态重算：此时 `pre` 与 `post` 相同、过程量全 0，并附 `note` 说明。

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
q.facts.view                             # 逐回合的视图（object 列：观测 + 每势力/每城一行 + AI 判定）
q.factions(round=12)                     # 每势力：库存/relations/自有城与舰
q.faction_snapshot(12, "中国")           # 一键决策视图（view+库存+relations+自有城/舰）
q.ships(round=12)                        # 第 12 月全部舰（含 effective 面板）
q.cities(round=12)                       # 第 12 月全部城（含 buildings 清单）
q.bodies() ; q.settlements()             # 天体 / 定居点主表
q.decisions(round=12)                    # 本回合 AI 的判定（逐舰 verdict + 判定的输入 + 船坞改装）
q.blueprints(round=12)                   # 设计图库（一行一图：舰级/选装/默认意图/归属/造过多少艘）
q.fleet(12, "中国") ; q.city_buildings(12, "中国")   # 某势力的舰 / 城
q.ids("ships", 12)                       # 第 12 月的舰名数组
q.join("ships", round=12)                # explode 主流 `舰名表` 并按 (round,id) merge 完整对象
```

常用判断：
```python
# 谁是霸权、谁被制裁
m = q.facts.iloc[-1].view
m["hegemon"], m["sanctioned"], m["coalition_members"], m["wars"]

# 我（中国）的走势
f = q.facts.view.apply(lambda m: m["factions"].get("中国", {}))
f.apply(lambda d: (d["city_count"], d["ship_count"], round(d["production_value"],1)))

# 我的外交 + 库存（重点：这能直接看会不会被制裁/缺什么矿）
snap = q.faction_snapshot(q.facts.iloc[-1]["round"], "中国")
snap["relations"]; snap["resources"]

# 边缘失稳城（忠诚低，易叛乱）
q.join("cities", round=12).query("势力 == '中国' and 忠诚度 < 0.5")

# 舰队维护费 vs 生产（翻车前看这个：upkeep > production → 先扩产）
snap["view"]["upkeep"], snap["view"]["production_value"]
```

> **语义视图（纯读取，把模拟算好的打包给你）**：`q.view_sitrep(12)`（世界政治：霸权/联盟/制裁/战争/
> 实力占比/各势力）；`q.view_frontier(12, "中国")`（我的失稳城，含 sim 算好的 `loyalty`/
> `gov_distance`/`revolt_risk`）；`q.view_market(12, "中国")`（库存按市场价）；`q.view_economy(12, "中国")`
> （产/维护/治理/净流/止血标记）；`q.view_spending(12, "中国")`（**钱去哪了**：批了多少 − 花了多少
> = 没花掉的、造舰是缺钱还是缺产能、每艘舰这个月被锈掉多少船体）；`q.resource_series("中国","铁")`
> （某资源逐月库存走势，看是否被抽干）。
> **这些只读、不重算游戏公式**。唯一要「游戏逻辑」判断的
> ——「我下令的造舰预算可持续吗？」——用 Rust 的 `--control-plan <faction>` 看 `verdict`，
> 不要在 Python 里自己估。

> 先 `--digest K --round N` 看整段走势的故事板，再对感兴趣窗口 `--index` 精读，别一把梭全量。

> **接手一局旧存档时**：先 `planet_x --start ckpt.json --round 0 --index out/` 读「这局最近在打什么」
> ——窗口层（`State::notables`）里是**后续计算要回看的那一段历史**（当前 = 开战/停战，供「记恨
> 地板」判定），在投影里是 `q.notables()`（按 `salience` 列筛出来的 `war_started`/`war_ended`）。
> 它随 checkpoint 存活，不需要当初的 `--index` 目录。
>
> ⚠ **不要**把「里程碑层」当「这局发生过什么」——**按当前判据那一层是空的**（`q.milestones()` 返回空表）。
> 判据是「后续计算需要访问哪一段历史」，不是「重要性」：没有任何模拟逻辑读无限过去，所以没有
> 事件属于它。要读「一座城的一生 / 谁打沉的谁」，用 **`--index` 投影**：
> `q.history("city", 城名)` / `q.cause("ship", 舰名)` / `q.storyboard(window)`（按 `weight`
> 压成故事板）。**「重要 ≠ 分层」**——想挑值得读的事件看 `weight` 列，别看 `salience`。
>
> ⚠ 2026-10 CLI 精简：`--traj` / `--story` / `--notables` / `--milestones` 四个 dump 开关**已删**
> ——它们的信息全在 `--index` 投影里（编年史在每行的 `chronicle`、两层历史按 `salience` 筛）。
> 新增 `--quiet`（只要最终 state 时用）。见 [笔记：CLI 读面](notes/cli-surface.md)。

---

## 3. 决策面：你能控制什么（指令 / control）

每个势力有一个 `ControllableState`，叶子都带 `mode`（**三态**）：`"Player"`（你说了算，系统
只读你的值）/ `"Auto"`（系统自动决定）/ `"Inherit"`（这一层没有说话，往上继承）。
`scope` 是一棵「谁负责」的作用域树（势力→天体→城市；⚠ **2026-10 用户裁决删掉了「全局」那一档**
——它唯一有区别的取值是「整个世界归玩家」，没人要）。旧档里的 `"Ai"` 与 `null`
分别按 `Auto` / `Inherit` 读入（`SCHEMA_VERSION` v4→v5 的零损失映射）。

**归属链（舰的指令与风格都是这条）**：`叶 → 出厂图（设计图）→ 舰队默认 → 势力 scope`，
最具体的那层**有意见**（`Player`/`Auto`）就它说了算；一路 `Inherit` 就到引擎兜底 `Auto`。
所以「新舰出厂**归谁**」的答案是**作用域链**（默认全归 AI）；而「新舰出厂**是什么**」的答案是
**设计图的倾向三轴**（角色 / 风格 / 姿态，见 §3 的 `blueprints`）——**不是**指令：
指令是**即时操作**（2026-10 用户裁决），只写逐舰叶，谁都没说话就是 `Idle` + 等 AI 接手。
⚠ 图的每条倾向轴**默认沉默**：建图（哪怕归玩家）**不等于**表态，只有图上真写了那条轴，
它才参与；而且只在该图的归属解析为 `Player` 时才供值。

**叶名一律是中文名词**（2026-10 起）：读面 `--index` 的 `control` 表 `kind` 列、写面
`--apply` 的键、`--control-schema` 的 `leaves[].field` 是**同一个词**（下表第一列）。
身份键 / 值字段也是中文（`舰`/`资源`/`城`/`建筑`/`图名`、`行为`/`值`/`姿态`/`角色`/`舰级`/`选装`），
归属字段叫 `归属`、删叶叫 `删叶`。

| 叶（= `--control-schema` 的 `field`） | 含义 | 身份键 → 值字段 |
|---|---|---|
| `指令` | 每艘舰的**移动/停泊**行为（即时操作） | `舰` → `行为`（`"Idle"` / `{"Follow":{"ship":"<舰名>"}}`…见下） |
| `舰队默认风格` | **舰队默认行为风格**（势力级一片，长期倾向） | 无 → `temper` + `lone_wolf`（**两轴一片叶**，新建必须一起给） |
| `舰队默认姿态` | **舰队默认风筝↔贴脸**（势力级一片） | 无 → `姿态` |
| `舰队默认角色` | **舰队默认角色**（势力级一片，第三条风格轴） | 无 → `角色`（`War`/`Freight`/`Observe`）。角色决定自动控制**派哪种活**（战舰找仗打／运输舰跑集货／观测舰蹲异常区喂 MOND 掌握度），不解除武装 |
| `风格` | 每舰**行为风格**（per-舰叶片） | `舰` → `temper` + `lone_wolf`（各 `[-1,1]`、`0`=基线） |
| `姿态` | 每舰**风筝↔贴脸**姿态（per-舰叶片） | `舰` → `姿态`（`[-1,1]`、`0`=基线。**软属性**：附近有敌舰时自动微调位置，玩家也不能硬控制） |
| `角色` | 每舰**角色**（per-舰叶片） | `舰` → `角色`。⚠ 这片叶**自动控制每回合也会写**（按积压定编集货 + 派舰去异常区），玩家钉 `归属=Player` 之后它不再碰 |
| `投资预算` | **建设**投资预算（每资源 / 月） | `资源` → `值` |
| `建造预算` | **造舰**建造预算（每资源 / 月） | `资源` → `值`（会先给维护费留**预留**，见下） |
| `福利预算` | 势力级福利预算（每资源 / 月） | `资源` → `值` |
| `建设权重` | 各建设任务优先级 | `城` + `建筑`（**城内的 u32 下标**） → `值` |
| `建造权重` | 各建造区优先级 | `城` + `建筑` → `值` |
| `城市福利预算` | 每城娱乐/福利（月） | `城` → `值` |
| `开发货币预算` | 每城开发货币（市场价值/月，国内市场开启时才有用） | `城` → `值` |
| `建造货币预算` | 每城建造货币（同上） | `城` → `值` |
| `首都` | **迁都**：换首都天体 | 无 → `值`（`{"值":"<天体名>","归属":"Player"}`）；首都=光速治理/本土防御锚点 |
| `设计图库` | **设计图库**（势力级，一张图一片叶） | `图名` → `舰级`/`选装`/`风格`/`姿态`/`角色`——见 §3 末尾。⚠ 图上**不能**写指令 |
| `建筑`（**命令，不是叶**） | 结构性增删改建 | `城`+`建筑` → 加/删建筑、改 `结构`、改 `建造舰级`（只对建造区有效）、**挂/拆设计图指针**（`{"城":…,"建筑":…,"设计图":"<图名>"}`；`"设计图": null` = 拆掉指针回到自动选装） |

> ⚠ **这张表是文档，不是权威**：权威是引擎发的 `--control-schema` 的
> `leaves` / `actions` 段（`src/control/leaves.rs`：每片叶的键名 / **身份键** / **值字段** /
> 只读派生列）。**先跑那个，别看这张表**——2026-10 实测它就漏了两片叶
> （`舰队默认角色` / `角色`，角色轴那次），补上就是因为这件事。
> 三端（引擎 / web 的 `views.json` / Python kit）现在读同一份声明，
> 纪律见 [`notes/web-control-spec.md`](notes/web-control-spec.md) 与 `play/tests/g4_spec.py`。

> **设计图（blueprint）= 「还不存在的舰」的出厂规格**：建造区**指向**一张图
> （`buildings[].blueprint`），下水那一刻把图**印成**一艘舰（`components` 是**快照**，
> 之后改图**不动**已有的舰）。`mode`：`Player` = 系统不许重估这张图（出厂按图装配，
> 图上写了某条**倾向轴**时那条轴也归你）/ `Auto` = 系统可重估（`retool_shipyards` 会改它的
> 舰级）/ `Inherit` = 这一层没说话（沿 scope 链解析）。
> **四条硬规则**（违反了会被点名丢弃）：
> * 口径 A：图的 `class` 必须 == 该建造区的 `ship_type` ⇒ **图与区要一起写**（`blueprint_class_mismatch`）；
> * 选装不许重复（`duplicate_component`）、不许超过该舰级槽位（`too_many_components`）、组件必须存在（`no_such_component`）；
> * 图名不存在时**不许凭空造图**（只写 `mode` ⇒ `no_such_blueprint`）；
> * **悬空指针 ⇒ 那个建造区停产**（指针指向一张被改名/删掉的图时，进度不再增加；读面把指针
>   原样输出，你可以据此看出「这个区为什么不出舰」）。指针写 `null` 才是拆掉它。
> ⚠ **玩家归属的图买不起就不下水**：进度继续攒、下回合再试（蓝图表 `launch_waiting` 列会
> 标出来）。`Auto` 图与无图**保持旧行为**（生成器自己保证买得起）。

> **风格与指令是同一个形状**（`--apply` 里的两片名字不同，语义同构）：写值即接管、
> 缺省轴保留现值、`mode` 显式给出时以它为准。
> **读面里的值是「有效值」**（指令：叶 → 出厂图 → 舰队默认；风格：叶 → 舰队默认 → 舰上记录值），
> 而 `mode` 是**叶片自己的表态**——所以整面 dump 回来安全；但**改值请把 `mode`
> 一起写成 `Player`/`Auto`**，只改值而留 `Inherit` 等于说"这一层没有意见"（除非舰队默认也是
> `Player`，那个值不会被采用）。
> ⚠ **`指令` 叶每艘舰都有一行**（与三条风格轴一致），`行为` 是**有效值**：
> `"行为": null` = **链上没有任何一层说话**（叶不存在 + 出厂图没写意图 + 舰队默认不是
> 玩家的），引擎才按 `Idle` 兜底——它不是"有人说了待命"。`归属` 是你/系统在那片叶上的表态
> （没有叶 = `Inherit`）。整面原样回传是**无损**的：读面 → `--apply` → 读面**逐字节相同**
> （`行为: null` 的那一行不会凭空建出一片叶来）。

#### `指令` 叶的六种行为（**攻击/轰炸不在其中**）

⚠ 写法是 serde 的**外部标签**枚举（不是 `{"type":…}`）：无参变体就是一个字符串，带参变体是
`{"变体名":{…}}`。变体名与参数字段名**有意保持英文**（它们是行为词汇，不是叶名；见
`notes/field-naming.md` 的「有意保持英文的字符串」）。

| 行为 | 写法（写进 `指令` 叶的 `行为` 字段） | 干什么 |
|---|---|---|
| `Idle` | `"Idle"` | 原地保持（不移动）。**不会停止开火**——射程内照样自动接战 |
| `Move` | `{"Move":{"position":[x,y]}}` | 驶向一个 2D 坐标（AU） |
| `Follow` | `{"Follow":{"ship":"<舰名>"}}` | 持续驶向某舰当前位置。**所随的可以是友舰（护航）也可以是敌舰（追袭）**；跟随本身不开火，但射程内自动接战 |
| `DockCity` | `{"DockCity":{"city":"<城名>"}}` | 驶向某城；**若该城敌对且进入围城射程则自动轰炸** |
| `Dock` | `{"Dock":{"body":"<天体名>"}}` | 跟随某天体轨道巡航/停靠（**守家最常用**） |
| `Colonize` | `{"Colonize":{"body":"<天体名>"}}` | 前往定居点天体并（再）建一座城 |

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
1. **两条预算独立、按权重竞争**：`投资预算` 建「楼」，`建造预算` 造「舰」；
   各自内部按权重（`建设权重`/`建造权重`）分钱，互不竞争。
2. **造舰先给维护费留底**：`upkeep × upkeep_reserve_mult`(≈4) 的市场价值**先被预留**，
   剩下的才用于造舰——**别把造舰预算拉满到经济承载之上**，否则维护费拖垮经济、引发治理
   崩溃与叛乱（这是最常见的翻车方式：造一堆养不起的船，帝国崩给你看）。
3. **舰型 ≠ 预算**：某建造区造什么由该建筑的 `ship_type`（如 `corvette`/`battleship`）决定，
   不是靠提高预算自动变高级舰。要让「整支舰队随威胁重构」，得改 `buildings[].ship_type` 或
   用 `建造权重` 加权。**注意 `建造舰级`（建筑行里那个 `ship_type` 列）只对建造区有效**，写在开采区
   上会被丢弃并报 `not_a_shipyard`。
4. **攻击是自动的，指令只管「去哪」**：见 §0 的提示。想让舰队「守住地球」就 `dock` 地球，
   而不是找一条「攻击」指令——敌舰进射程会自动打。

> 想**整体接管**一个势力：`{"scope":{"factions":[["中国","Player"]]}}` —— 但这**管不了已经
> 自己有叶片的舰**（叶比 scope 更具体）。要让全舰队真正听话，两条一起做：
> ① 势力级**长期倾向的默认**（`舰队默认风格` / `舰队默认姿态` / `舰队默认角色`）=
> "没说话"的舰的答案（⚠ **指令没有**势力级默认：它是即时操作，只写逐舰叶）；② 逐舰把叶片
> 交回上层（`{"舰":"长城","归属":"Inherit"}`，只写归属不动值）或钉成 `Player`；
> ③ 想让全舰队去干同一件事就**逐舰点名**（`{"舰":"…","行为":…}` 多写几行——这是唯一的路）。
> **写值即接管**：diff 里只写值、不写 `归属` ⇒ 那片叶变 `Player`（回执 `NOTE_APPLY_TOOKOVER`
> 会点名）。想只改"流水记录"而不接管，显式写 `"归属":"Auto"`/`"Inherit"`。
> 旧手册那句「注意这会让新造出来的舰默认 Idle」的答案现在是：**新舰出厂默认归 AI**
> （作用域链），它会自己决定干什么；要它按你的意思来，就在**设计图**上写**角色**
> （运输舰图 / 战舰图）——那是"这型舰是什么"，而不是"这艘舰现在去哪"。

---

## 4. 下一条指令：写 `--apply` 的 diff

`--apply <file.json>` 接受 `{control:[...], scope:{...}}`（与 web `POST /api/command` 同形）。
它是**多层级结构化补丁**：只触碰 diff 里出现的势力/叶子；某个叶子省略 `值`/`行为`
保留当前值；**省略 `归属` 时：写了值就接管（变 `Player`），什么都没写才保留当前模式**。
风格轴同理（`风格` / `姿态` / `舰队默认风格` / `舰队默认姿态`）。

### 4.1 先从模板改
```bash
planet_x --seed 7 --control    # 整面可编辑模板（每势力：指令/预算/权重/scope）
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
    "势力": "中国",
    "建造预算": [
      {"资源": "硅", "值": 1.0, "归属": "Player"},
      {"资源": "碳", "值": 2.0, "归属": "Player"},
      {"资源": "铁", "值": 2.5, "归属": "Player"}
    ],
    "城市福利预算": [ {"城": "长三角", "值": 2.0, "归属": "Player"} ],
    "指令": [
      {"舰": "长城", "行为": {"Follow": {"ship": "华盛顿"}}, "归属": "Player"},
      {"舰": "北斗", "行为": {"Follow": {"ship": "赤霄"}},   "归属": "Player"},
      {"舰": "赤霄", "行为": {"Dock":   {"body": "地球"}},   "归属": "Player"}
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
- ⚠ **新造出来的舰不在 diff 里**：它出厂时**归 AI**（作用域链），指令是空的（`Idle`）。
  只接管几艘舰时，"新舰干什么"由**图的角色**回答（"这型舰是运输舰" ⇒ 它一出来就被派去跑
  集货路线）；要它去某个具体地方，就**逐舰下指令**（一次写一队也行：`指令` 里多写几行）：
  ```jsonc
  {"control":[{"势力":"中国",
    "指令":[{"舰":"长城","行为":{"Dock":{"body":"地球"}}},
            {"舰":"北斗","行为":{"Dock":{"body":"地球"}}}],
    "舰队默认姿态":{"姿态":-1.0,"归属":"Player"}
  }]}
  ```
  ⚠ **指令没有"舰队默认"那一片叶**（2026-10 裁决：它是即时操作）——写 `default_ship_order`
  会被引擎**拒绝**（未知字段）。舰队级只剩长期倾向三片（`舰队默认风格` / `舰队默认姿态` /
  `舰队默认角色`），写了它们，**所有"没有说话"的舰（含以后下水的）**在那三条轴上按它走。
- **按舰级编排**（"新造的护卫舰守家、巡洋舰远征"）用**设计图**，而不是给每艘舰点名：
  ```jsonc
  {"control":[{"势力":"中国",
    "设计图库":[{"图名":"护卫-守家","舰级":"corvette",
                   "选装":["kinetic","ion_drive"],             // 空数组 = 出厂时交给生成器现算
                   "角色":"War",                                // 图上写了它，这条轴才参与
                   "姿态":-0.5,                                 // 第二条轴：稍微贴脸一点
                   "归属":"Player"}],
    "建筑":[{"城":"珠三角","建筑":7,"建造舰级":"corvette","设计图":"护卫-守家"}]
  }]}
  ```
  这一份 diff 说：`珠三角` 的 7 号建造区以后按「护卫-守家」出厂（选装钉死 + 这型舰是**战舰**、
  稍微贴脸），而且这张图**归玩家**——AI 不许重估它。**图与建造区的舰级必须一起写**（口径 A）。
  ⚠ 图上**不能**写指令（`"order"` 已是未知字段）：要"这型舰守地球"，就写 `"role":"War"` 让它去
  找仗打、再用**逐舰** `指令` 点几艘名；或者干脆把那张图当作"选装模板"，倾向全留空
  （`"role": null` 这种 = 该轴沉默，交给舰队默认 / 出厂快照）。
- 若想**整体接管**一个势力（所有叶子都归你），写
  `{"scope":{"factions":[["中国","Player"]]}}`。注意**叶比 scope 更具体**：已经自己有叶片的舰
  不会被 scope 翻转，要逐舰写 `{"舰":"长城","归属":"Player"}`（只写归属，不动值）或
  `"Inherit"`（交回上层）。整面接管意味着经济/造舰决策也归你扛。

### 4.3 behavior 两种写法都认（**读面只发一种**）
- **官方枚举形式** = **读面发的那种**（`--control` 的 `指令[].行为`、`--index` 的 `ships.order_effective`
  与 `control` 表的 `value`）：`"Idle"`、`{"Follow":{"ship":"华盛顿"}}`、`{"Dock":{"body":"地球"}}`、
  `{"DockCity":{"city":"长三角"}}`、`{"Move":{"position":[-0.5,0.3]}}`、`{"Colonize":{"body":"火星"}}`。
  ⚠ 变体名与参数字段名**有意保持英文**（行为词汇，不是叶名）。
- **tagged 简写**（写面额外收下、`--apply` 会归一化成上面那种；**读面不会发它**）：
  `{"type":"follow","ship":"华盛顿"}`、`{"type":"idle"}`、`{"type":"dock","body":"地球"}`、
  `{"type":"dock_city","city":"长三角"}`、`{"type":"move","position":[-0.5,0.3]}`、
  `{"type":"colonize","body":"火星"}`。合法 tag 全表在 `--control-schema` 的 `ShipBehavior.oneOf`
  与引擎的 `BEHAVIOR_TAGS`（一处声明）。
- 把 `--control` 里的 `行为` 原样粘进 diff 即可（`--apply` 两种都认）。
- **写错 tag 会当场报错**（退出码 10），错误里会给合法 tag 全表；若你写的是已移除的
  `target_ship`/`target_settlement`，错误里还会直接给出替代写法。**不要靠试错猜行为名——
  读报错，或先 `--control-schema` 看 `ShipBehavior` 的 `oneOf`。**

### 4.4 关键：**有名字的用名字，`building` 用下标**
`城`/`势力`/`舰`/`天体`/`定居点` 一律用**名字**（唯一名），不是整数编号。
**唯一的例外是 `建筑`**：它是 u32 下标，**只在所属城内部唯一**，所以
`建设权重`/`建造权重` 的 `{"城":…,"建筑":…}` 必须配套——同一个下标换座城
就是另一栋楼。查下标：`planet_x --control`（可编辑模板，含每城的建筑下标与它的
`类型`/`建造舰级`/`设计图`）、`--index` + `planet_xq` 的 `q.cities(r)`（`建筑[]` 里带 `建筑编号`）；
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
应用  planet_x --seed 7 --apply diff.json --round 30 --save ckpt30.json 2>apply.jsonl
回执  看 apply.jsonl：WARN_APPLY_SKIPPED = 有叶片没落地（见 §4.5）。别跳过这一步
再看  planet_x --start ckpt30.json --round 0 --index out3/  （q.notables()：这局最近在打什么）
续玩  planet_x --start ckpt30.json --round 30 --index out2/   （续玩 + 精读）
```

- **确定性**：同 seed（或同 checkpoint + 已保存 RNG 位置）→ 后续逐字节一致。
  `--save`/`--start` 是分段续玩 / 复现的关键；`--start` 也接受旧单 `State` `.json`（此时重播
  seed、随机流不延续）。
- **`--save` 在 `--index` 模式下也生效**：`--start ckpt --apply diff.json --round 30 --save ckpt.json
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
   给偏远/新占城市投 `城市福利预算`；必要时放弃过度扩张的远端殖民地。
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
- **被引导**：把舰钉在**本土**（`Dock` 地球）并调成**风筝姿态**（`姿态: -1.0`）→
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
| `--start <ckpt.json>` | 从 checkpoint（State+RNG）续玩 |
| `--round <N>` | 输出 N+1 行全量状态 JSON（回合 0 先） |
| `--quiet` | 与 `--round` 连用：**只推进、不吐轨迹**（1000 回合省掉 40 MB stdout）。要留东西配 `--save`/`--index`/`--digest` |
| `--apply <diff.json>` | 叠加控制 diff 后继续 |
| `--save <ckpt.json>` | 结束后写 checkpoint |
| `--meta` | 游戏规则字典（resources/buildings/ships/economy/combat…） |
| `--schema` | 状态视图的 JSON Schema |
| `--control` | 可编辑控制面模板。**读面不舍入**：里面的数就是状态里存的数（逐位），所以"原样回传"是**无损**的——只改你想改的那几行 |
| `--control-schema` | `--apply` diff 能写哪些字段的 JSON Schema，**外加控制叶的结构事实**（`leaves` / `actions` / `owner_field` / `remove_field`）：每片叶的键名、**身份键**（`keys` 空 = 势力级单叶）、**值字段**、随行属性 `carries`、只读派生列 `read_only`。这一份是 web 与 kit 共用的唯一声明（`src/control/leaves.rs`，纪律见 `play/tests/g4_spec.py`） |
| `--derived` | 这一回合的**视图对** `{round, source, pre, post}`（两个槽都是 `RoundView` 且同形：`pre` = 回合开始时的世界、`post` = 回合结束时的世界 + 本回合过程量；`post.decisions` = **本回合 AI 的判定**）；与 `--index` 的过程量表同值 |
| `--control-plan [<faction>]` | 给势力算「成本→收益」（产出/维护/治理/净流/可养舰上限/清算倒计时） |
| `--every <K>` | 每 K 回合一个全量快照（降采样）。**只管 stdout 轨迹，不动 `--index` 投影** |
| `--digest <K>` | 每 K 回合一行语义故事板（世界/各势力/战争/事件计数/剧情节拍） |
| `--index <DIR>` | 投影成 lean 主流 + lazy 表（ships/cities/factions/bodies/settlements/events）+ **派生表**（faction_process/city_process/control/scope/decisions/blueprints/market_trades/haul_steps/round_inputs）+ schema.json（planet_xq 读）。**编年史与两层历史都在这里**（`chronicle` 列 / `events` 的 `salience` 列） |

> 2026-10 起 CLI 精简过一轮：`--traj` / `--story` / `--notables` / `--milestones`（以及
> `--rounds` 这个别名）**已删**——它们给的东西投影里都有；**裸调用 `planet_x` 现在打 help**
> 而不是回一条 `ERR_USAGE`。见 [笔记：CLI 读面](notes/cli-surface.md)。

> 注意：**没有交互式 REPL**、没有 `--query`。这正是设计：stdout 零噪声、确定性、可复现；
> 分析在外部（planet_xq / pandas）做，控制走 `--apply` diff。
> **`--apply` 的回执在 stderr**（`ERR_APPLY` 退出码 10；`WARN_APPLY_SKIPPED` 退出码 0）——
> 用 `2>file` 收好它，别 `2>/dev/null`（那等于把「命令没下达」这条唯一线索扔掉）。

---

## 8. 坑与边界

- **jq 已移除**：分析用 `--index` + `planet_xq`；不要假定 `jq` 在 PATH。
- **`--round` 是全量快照**（几千回合会撑爆上下文）；省 token 用 `--index` + `--digest`，
  连轨迹都不要就用 `--quiet`。
- **别把 `--apply` 的 stderr 丢掉**：`WARN_APPLY_SKIPPED` 是唯一能告诉你「这条命令没有落地」
  的信号（见 §4.5）。stdout 永远只是状态流，成功时不打印任何回执。
- **有名字的实体用名字**（舰/城/势力/天体/定居点 = 唯一名）；`--index` 的 lazy 表、`--apply`
  diff、`view.factions` 的 key 全用名字，不是整数。**例外：`building` 是城内的 u32 下标。**
  同一实体在扩张/重建后名字可能变（如 "长城2"），拿它当 key 用，别假设数字下标或长期稳定。
- **舰/城会死，名字会换代**：`--apply` 点名的舰如果已战沉，那条指令会被丢弃（报
  `no_such_ship`）。玩长局时**每回合重新查一次名字**，别把上一回合的 diff 直接重放。
- **外交/库存看 `factions` 表**：`--index` 的 `idx/factions.jsonl` 直接给每势力的
  `relations`（两两关系）与 `resources`（库存），`q.faction_snapshot(r, 名字)` 一行拿全——
  不用再为了看外交/库存去 `--round 0` 全量 dump。
- **`view.factions` 是 dict（key= faction id 的字符串名）**，不是数组；`production` 是每资源
  dict，总产出用 `production_value`。
- **`chronicle` 是累计的**（每行都带整段叙事弧到当回合），逐行读会重复；按 `(round, id)` 去重。
- **`q.view_*` 的返回类型不统一**：`view_sitrep`/`view_market`/`view_economy` 是 dict
  （`view_sitrep` 里**嵌着 DataFrame**，直接 `json.dumps` 会炸），`view_frontier` 是 DataFrame。
  按需 `.to_dict()`/`to_string()`，别假设能一把 `json.dumps`。
- **`events` 的类型列叫 `type`**（不是 `kind`）；`q.events(type="siege")` 直接按类型取（字段稠密），
  实体历史用 `q.history(kind, id)`（这里的第一个参数才叫 `kind`，取 `"ship"`/`"city"`）。
- **回合 0 未步进**，`view` 里的**过程量全为 0/空**（`pre` 面永远如此：那时还没算）；看真实流量从回合 1 起。
