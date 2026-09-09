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

- **id 永远是字符串名**（舰/城/势力/天体/定居点 = 它的唯一名），不是整数编号；`--apply`
  diff 里的 `city`/`ship`/`faction_id` 写名字。
- **别让舰队维护费越过生产**：造舰预算会被「维护费 ×4 预留」封顶，但**流水的产出 vs 流水的
  维护/治理**才是生死线——`--control-plan <faction>` 先看 `verdict`，`bleeding`（净流为负）时
  先扩产、再扩军。否则帝国会被造船潮拖垮（几十回合内从霸权塌成 1 城）。
- **别当永久单极**：某势力实力占比超阈值 → 全网合纵 + 经济制裁 + 治理成本放大，过度扩张必被
  「均势」拉回来。想世界健康就做「强而不独」，不是「称霸到底」。

### 读世界在哪读、控制面在哪写

- **读（观察）**：`--index` 投影 + `planet_xq`（lean 主流 + 按 id 索引的 lazy 表：
  `ships`/`cities`/`factions`/`bodies`/`settlements`）。`factions` 表直接给每势力的
  `relations`（外交）/`resources`（库存）/自有城与舰；`ships` 给 effective 面板
  （attack/range/speed/upkeep/components）。要看 `State` 全量（relations 之外的原始结构）可
  `planet_x --round 0`（或 `--start ckpt --round 0`）拿单行完整 JSON。
- **写（控制）**：`planet_x --control` 拿可编辑模板，改进 `--apply` diff；`--control-plan
  <faction>` 先算「成本→收益」；`--control-schema` 查 diff 能写哪些字段。

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
| `cities` | 每回合 | `city_id faction_id body_id name population loyalty razed deployed_area building_count buildings[]` |
| `bodies` | 全局一次 | `body_id name x y perihelion_distance aphelion_distance period settlement_count` |
| `settlements` | 全局一次 | `settlement_id body_id name total_area ecological_capacity construction_speed_mod resources` |

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

---

## 3. 决策面：你能控制什么（指令 / control）

每个势力有一个 `ControllableState`，叶子都带 `mode`：`Player`（你说了算，系统只读你的值）
/ `Ai`（系统自动决定）/ `null`（继承上面 `scope`）。`scope` 是一棵「谁负责」的作用域树
（全局→势力→天体→城市）。

| 指令面 | 含义 | 关键点 |
|---|---|---|
| `ship_orders` | 每艘舰的行为 | `idle / move / target_ship / target_settlement / dock / colonize` |
| `investment_budget` | **建设**投资预算（每资源 / 月） | 用于建建筑、扩生产 |
| `construction_budget` | **造舰**建造预算（每资源 / 月） | 用于造舰；会先给维护费留**预留**（见下） |
| `invest_weights` | 各建设任务优先级 | 谁先吃投资预算 |
| `build_weights` | 各建造区优先级 | 哪个船坞先造 |
| `loyalty_budget` | 每城娱乐/福利（月） | 提「忠诚」压低叛乱 |
| `buildings` | 结构性增删改建 | 加/删建筑、改 `structure`、改 `ship_type` |

### 三条必须懂的语义
1. **两条预算独立、按权重竞争**：`investment_budget` 建「楼」，`construction_budget` 造「舰」；
   各自内部按权重（`invest_weights`/`build_weights`）分钱，互不竞争。
2. **造舰先给维护费留底**：`upkeep × upkeep_reserve_mult`(≈4) 的市场价值**先被预留**，
   剩下的才用于造舰——**别把造舰预算拉满到经济承载之上**，否则维护费拖垮经济、引发治理
   崩溃与叛乱（这是最常见的翻车方式：造一堆养不起的船，帝国崩给你看）。
3. **舰型 ≠ 预算**：某建造区造什么由该建筑的 `ship_type`（如 `corvette`/`battleship`）决定，
   不是靠提高预算自动变高级舰。要让「整支舰队随威胁重构」，得改 `buildings[].ship_type` 或
   用 `build_weights` 加权。

> 想**整体接管**一个势力：`{"scope":{"factions":[[3,"Player"]]}}`（叶子 `mode:null` 都继承
> Player）。想只接管某几艘舰/某条预算：给具体叶子显式 `"mode":"Player"` 即可，别整面接管
> （否则新造出来的舰默认 Idle 没人指挥、经济预算也不再 AI 调）。

---

## 4. 下一条指令：写 `--apply` 的 diff

`--apply <file.json>` 接受 `{control:[...], scope:{...}}`（与 web `POST /api/command` 同形）。
它是**多层级结构化补丁**：只触碰 diff 里出现的势力/叶子；某个叶子省略 `value`/`behavior`
保留当前值、省略 `mode` 保留当前模式。

### 4.1 先从模板改
```bash
planet_x --seed 7 --control    # 整面可编辑模板（每势力：ship_orders/预算/权重/scope）
# 这是「整面」模板（所有势力都在）；挑出你要改的那几片叶子写进 diff 即可。
# 想看/控制面收敛到单个势力：见 agent-play.md 的 §3/§4（按 scope 接管），或先 `--index` +
# planet_xq 的 q.faction_snapshot(r, 名字) 只读你关心的那个势力。
```

### 4.2 diff 实例（中国：造舰、稳忠诚、守地球——**局部接管**）
> **id 一律是字符串名**（舰/城/势力 = 唯一名）。以下用真实开局实体：中国="中国"、舰=
> "长城"/"赤霄"/"北斗"、城="长三角"、美国舰="华盛顿"。
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
      {"ship": "长城", "behavior": {"type": "target_ship", "ship": "华盛顿", "attack": true},  "mode": "Player"},
      {"ship": "北斗", "behavior": {"type": "target_ship", "ship": "赤霄", "attack": false}, "mode": "Player"}
    ]
  }]
}
```
- 这是**局部接管**：只把「造舰预算 / 某城忠诚 / 两艘舰」钉成 `Player`，其余（投资、其它舰、
  生产）仍交给 AI。舰"长城"去攻击美国舰"华盛顿"；舰"北斗"**守卫**"赤霄"（`attack:false` 的
  `target_ship` 就是守卫）。
- **没有 `guard_ship`/`guard` 这个 behavior 类型**——守卫是 `TargetShip{ship:<友舰>, attack:false}`。
- 若想**整体接管**一个势力（所有叶子都归你），才写
  `{"scope":{"factions":[["中国","Player"]]}}`。注意这会让**新造出来的舰默认 Idle**、经济预算也不再
  AI 调——你同时要能扛起这些决策。

### 4.3 behavior 两种写法都认
- **tagged 形式**（就是你从舰的 `order` 字段里看到的）：`{"type":"target_ship","ship":"华盛顿","attack":true}`、
  `{"type":"idle"}`、`{"type":"dock","body":"地球"}`。
- **默认枚举形式**：`"Idle"`、`{"TargetShip":{"ship":"华盛顿","attack":true}}`。
- 把 `--control` 里的 `order` 原样粘进 diff 即可（`--apply` 会自动归一化）。

### 4.4 关键：**id 是字符串名**
`city`/`building`/`faction`/`ship`/`body`/`settlement` 一律用**名字**（唯一名），不是整数编号。
查 id：`planet_x --control`（可编辑模板）、`--index` + `planet_xq` 的 `q.factions()`/`q.ships()`/
`q.cities()` lazy 表；资源 key 同理（WYSIWYG，状态里的「铁」就是 diff 里的「铁」）。

---

## 5. 游玩循环（观察→决策→应用→复现）

```
观察  planet_x --seed 7 --index out/ ;  planet_xq 读
决策  写 diff.json（见 §4）
应用  planet_x --seed 7 --apply diff.json --round 30 --save ckpt30.ron
再看  planet_x --start ckpt30.ron --round 30 --index out2/   （续玩 + 精读）
```

- **确定性**：同 seed（或同 checkpoint + 已保存 RNG 位置）→ 后续逐字节一致。
  `--save`/`--start` 是分段续玩 / 复现的关键；`--start` 也接受旧单 `State` `.ron`（此时重播
  seed、随机流不延续）。
- **省 token**：先 `--digest K --round N`（每 K 月一行语义故事板），锁定要放的窗口，再
  `--index` 精读那一段，最后 `--round`/`--apply` 介入。别把 `--round 3000` 的全量 JSON 灌进上下文。

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
| `--control` | 可编辑控制面模板 |
| `--control-schema` | `--apply` diff 能写哪些字段的 JSON Schema |
| `--control-plan [<faction>]` | 给势力算「成本→收益」（产出/维护/治理/净流/可养舰上限/清算倒计时） |
| `--every <K>` | 每 K 回合一个全量快照（降采样） |
| `--digest <K>` | 每 K 回合一行语义故事板（世界/各势力/战争/事件计数/剧情节拍） |
| `--index <DIR>` | 投影成 lean 主流 + 按 id 索引的 lazy 表（ships/cities/factions/bodies/settlements）+ schema.json（planet_xq 读） |

> 注意：**没有交互式 REPL**、没有 `--query`。这正是设计：stdout 零噪声、确定性、可复现；
> 分析在外部（planet_xq / pandas）做，控制走 `--apply` diff。

---

## 8. 坑与边界

- **jq 已移除**：分析用 `--index` + `planet_xq`；不要假定 `jq` 在 PATH。
- **`--round`/`--traj` 是全量快照**（几千回合会撑爆上下文）；省 token 用 `--index` + `--digest`。
- **一切 id 都是字符串名**（舰/城/势力/天体/定居点 = 唯一名）；`--index` 的 lazy 表、`--apply`
  diff、`metrics.factions` 的 key 全用名字，不是整数。同一实体在扩张/重建后名字可能变（如
  "长城2"），拿它当 key 用，别假设数字下标或长期稳定。
- **外交/库存看 `factions` 表**：`--index` 的 `idx/factions.jsonl` 直接给每势力的
  `relations`（两两关系）与 `resources`（库存），`q.faction_snapshot(r, 名字)` 一行拿全——
  不用再为了看外交/库存去 `--round 0` 全量 dump。
- **`metrics.factions` 是 dict（key= faction id 的字符串名）**，不是数组；`production` 是每资源
  dict，总产出用 `production_value`。
- **`chronicle` 是累计的**（每行都带整段叙事弧到当回合），逐行读会重复；按 `(round, id)` 去重。
- **回合 0 未步进**，`metrics` 流量为 0；看真实流量从回合 1 起。
