# 行星X — 太空沙盘

《行星X》是一个回合制（每回合 = 1 个月）的太阳系沙盘轨迹生成器：经济（采矿 / 人口 / 建设 / 造舰）、飞船战斗、围城、外交、殖民可复现模拟。城市无独立城防——护甲就是其建筑（混凝土/钢结构）的硬度总和；围城直接削建筑、打空即夷平为空白（不攻占），可再殖民。势力有**投资**与**建造**两条独立预算，各按建设/建造投资权重内部竞争。天体、资源、建筑结构、舰船均由 `config/game.ron` 数据驱动，不硬编码。默认世界按 spec 天体表铺开：18 个天体共 **22 个定居点、22 座城**（定居点 ↔ 城市**一一对应**——气态巨行星的定居点为**轨道空间站**；地球坐拥 5 个定居点 = 长三角/珠三角/亚特兰大/巴黎/莫斯科，每城各有自己的区域矿藏）；舰船为 spec 五级（护卫舰/驱逐舰/巡洋舰/航空母舰/战列舰），每级**把自己的招牌数值拉到极高**（护卫舰=极速、驱逐舰=高再生、巡洋舰=重甲、航空母舰=超远程、战列舰=一锤定音）且带**护甲再生**与**维护费 upkeep**。

**动态国际关系**：**开局和平**（无战争状态），关系由 `sim::step_diplomacy` 驱动波动——每对势力按意识形态 `alignment` 的静息亲和漂移（同阵营靠拢、异己升温），好战 `aggression` 加速敌对化；开火/占领压关系入战争，停战后经战争疲态向停战线回落、可再升温。**星际市场**（`sim::step_market`）：每势力自动把富余矿物按参考价值兑换成所缺的关键矿物——解决资源分布不均（如中国 700 铁却缺碳、欧盟不产工业三矿），也提供资源池；**维护费**（`sim::step_upkeep`，每舰每回合按舰级扣资源）则给舰队设上限、避免无限膨胀。

**剧情编年史**：一局不只是状态机——`config/game.ron` 的 `story` 表定义了一条**数据驱动的叙事弧**（`sim::step_story` 每回合评估触发）。每条剧情事件有一个 `trigger`（`RoundAt` 节拍，或 `FirstWar`/`FirstRaze`/`FirstColony`/`WarBetween`/`FactionAtWar`/`RelationBelow` 这类**事件型**触发）——事件型触发恰好落在对应历史事件发生的那个回合，形成「剧情与局势同步」。每条带标题/正文/参与方，并可附带**小幅确定性**机械后果：`Relations`（关系变化）、`GrantResources`（注入资源）、`GrantShip`（在某天体附近为某势力**出厂一艘舰**——给剧情真实的机械分量，如新锐旗舰下水）。触发的剧情记入 `State::chronicle` 编年史（agent 用 `story` 命令或 `--query '.story'` 读取整段弧），也作为一条 `events` 里的 `story` 事件在本回合流水出现；`meta` 的 `story` 数组同时列出每条的 `trigger` 与 `effects`，供 agent 预判剧情节拍与后果。外交跃迁（开战 `war_started` / 停战 `war_ended`）同样作为事件暴露。

---

## 构建

```bash
cargo build                 # debug 构建（本文件所有示例均用 debug 二进制）
# 或
cargo build --release       # 产物在 target/release/planet_x
```

---

## 唯一模式：Agent 零噪声

`planet_x` 只服务 LLM agent：**stdout 只输出 JSON Lines**，无颜色、无星图、无表格、无中文散文、无无关输出；浮点四舍五入到 2 位小数，低于阈值的资源被丢弃。确定性：同一种子输出逐字节一致。它没有 `--agent` 参数——零噪声机器输出就是它唯一的行为。

**批量编年史**（`--round N`）：每回合输出一行紧凑、稳定的状态 JSON（回合 0 先，之后每回合一个）。

```bash
# N+1 行 JSON（回合 0 + N 个回合），一整份编年史
planet_x --seed 42 --rounds 10
```

**查询 REPL**（不带 `--round`）：状态留在进程内，每条命令返回一个 JSON 值，可下钻，避免把整份世界态灌给 agent。

```bash
planet_x --seed 42
```

### 1. 查询与 REPL（`--query` / 命令式）

分层、按需获取信息，避免把整份世界态灌给 agent。

**一次性的 jq 管道**（`--query`）：对 agent 状态执行一个 jq 过滤并输出 JSON Lines。带 `--round N` 时会先推进 N 回合，因此可对「回合末」状态提问。

```bash
# 筛城市：中国的城市
planet_x --seed 42 --query '.cities[] | select(.owner_name=="中国") | {name, population, armor}'

# 打完 5 回合后，看受伤的船
planet_x --seed 42 --rounds 5 --query '[.ships[]] | map(select(.hull < .hull_max)) | .[] | {name, hull}'

# 打完 8 回合后，看被打得够弱的城
planet_x --seed 42 --rounds 8 --query '.cities[] | select(.razed == false and .armor < 15.0) | {name, owner_name, armor, razed}'

# 势力名单 / 交战对象
planet_x --seed 42 --query '.factions[] | select(.wars | length > 0) | .name'
```

**查询 REPL**（`planet_x`，无 `--round`）：stdin 发命令，stdout 回 JSON。

```
q <jq>                  # 对当前状态执行任意 jq 过滤（JSON Lines）
summary                 # 紧凑雷达（回合/时间/计数/各势力交战）
advance [n]             # 推进 n 回合（默认 1），然后打印 summary
control [<faction_id>|<jq>]  # 输出可编辑控制面（control+scope）。裸→整面；control 3 → 只出中国；control <jq> → 对整面做 jq。
meta [<jq>]             # 输出游戏配置（规则字典）：资源 raw key→中文名、建筑/舰船全表、经济/战斗/外交常量
apply <file.json>       # 把一份控制状态 diff 叠加到状态上，然后回读 control
order <ship> attack|guard|siege|move|dock|colonize|idle ...   # 一键下舰指令（Player 模式）。attack <敌舰> 追袭开火；guard <友舰> 守卫；dock <天体> 停泊轨道（随其巡航）；idle 待命原地。
budget <faction> <resource> <value>   # 设该势力「建设建筑」投资预算叶子（Player）
build  <faction> <resource> <value>   # 设该势力「造舰」建造预算叶子（Player）
events                  # 打印本回合事件（开火/被毁/城被夷平/殖民/陈旧指令降级/开战停战/剧情）
story [<jq>]            # 打印剧情编年史：每条叙事事件一行 JSON（round/id/title/body/participants），
                        # 可跟 jq 过滤整段弧（如 `story .[] | select(.round>10)`）。
delta [n]               # 推进 n 回合（默认 1）并打印一段紧凑的状态语义差分：新舰/被毁舰、城归属/夷平/人口变化、各势力资源增量、跨越战争阈值的关系。
save <file.ron>         # 写 checkpoint：当前 State + PRNG 位置（续玩可复现后续回合）
load <file.ron>         # 从 checkpoint 恢复状态与 PRNG 位置（别名 resume）
cities / ships / factions / bodies
city <id> / ship <id> / faction <id> / body <id>   # 单实体详情
guide / help            # 命令目录（JSON）；q/query、s/summary、a/advance、quit/exit
quit / exit
```

> `control` 的定向读取把「读模板」的 token 成本压到单个势力（实测 1700 vs 7578 字符）。
> `order`/`budget`/`build` 是 `apply` 的高层薄封装，底层仍生成同一形状的 diff。
> 玩家指令如果目标失效（目标舰被毁、城被夷平、无定居点），会自动降级为 `Idle`
> 并记一条 `stale_order` 事件，而不是让船飞向太阳中心 `[0,0]`。
> `apply` 里舰的 `behavior` 两种写法都认：默认枚举形式（`{"TargetShip":{"ship":2,"attack":true}}`、
> `"Idle"`），以及**状态视图里 `order` 的 tagged 形式**（`{"type":"target_ship","ship":2,"attack":true}`、
> `{"type":"idle"}`）——所以你可以把 `order` 字段原样粘进 diff，无需手动翻译。
> **作用域接管**：AI 不再把 `mode:"Ai"` 写死进每个叶子，而是写 `mode:null`（继承），所以
> `{"scope":{"factions":[[3,"Player"]]}}` 就能真正整体接管中国（默认 `mode:null` + 全 None 作用域仍归 AI；
> 想单独把某叶子钉成 `Ai`/`Player` 仍可显式设 `mode`）。
> **`events` vs `delta`**：`events` 记「这回合发生了什么」（开火/被毁/夷平/殖民/陈旧指令/开战停战/剧情故事）；
> `delta` 记「状态变成了什么样」——两快照间的语义差分。想看一层楼到底改了什么用 `delta`，
> 想看事件流水用 `events`。
> **守卫**：`TargetShip{ship,attack:false}` 是守卫——护住一艘**友舰**：靠近它并拦截进入自身
> 攻击距离的敌方舰，但**不会**向被保护舰开火。目标是友方（非敌对）舰才有效；敌舰被毁或友舰
> 被毁都会降级为 `Idle` 并记一条 `stale_order`。`order <ship> guard <友舰>` 可一键下达。
> **停泊轨道 / 待命**：`Dock{body}` 停泊轨道——每回合重新驶向天体当前位置、随其轨道巡航；`Idle`
> 待命——原地不动（保持当前坐标）。二者都不会降级、都安全。`order <ship> dock <天体>` / `order <ship> idle`
> 可一键下达。spec 行为枚举已无「无/None」——原地停靠一律用 `idle`。

`--script <file>` 从文件非交互读取上述命令并执行后退出（stdout 仍是纯 JSON Lines）。`--apply <file.json>` 在任何命令运行前把一份控制状态 diff 叠加到状态上，二者常与 `--start` 组合成一个回合的 agent 决策循环。

```bash
printf 'summary\nadvance 1\nq .ships[] | select(.order.type != "idle") | {name, order}\nquit\n' \
  | planet_x --seed 42
```

支持的 jq 子集：管道 `|`、路径 `.a.b` / `.[]` / `[0]` / 切片、`select` / `map` / `map_values`、`{..}` 对象构造（含简写）、数组 `[..]`、`sort` / `sort_by` / `reverse` / `length` / `keys` / `unique` / `add` / `sum` / `count` / `count(expr)` / `group_by(expr)` / `first` / `contains` / `startswith` / `endswith` / `type` / `empty`，比较与逻辑运算 `== != < <= > >= and or not`、算术 `+ - * /`、`//`。字符串与数字字面量，数字按值比较（`2 == 2.0`）。聚合常用形如 `.ships | group_by(.owner) | map({owner: .[0].owner, n: length})` 或 `[.factions[].resources["铁"]] | sum`。

### 1.0 游戏配置（规则字典 `meta`）

agent 的输入不只是状态，还有**规则**：可采什么、什么值多少、造一艘船要不要得起。`meta` 命令（或 `--meta` 标志）一次性输出整份 `GameConfig` 为一行 JSON——这是 agent 的规则字典，也是它把状态里的**中文名**翻译成 `control` 里**原始 key** 的映射表。

```bash
# 一次性读完整个配置
planet_x --meta

# 只取 资源 key→中文名 字典（写 control 预算时用原始 key）
planet_x --meta --query '.resources'

# 看某舰级数值（决定该造什么）
planet_x --meta --query '.ships.corvette'

# REPL 内过滤
planet_x                      # 然后：meta .buildings.residential
```

返回结构：

```jsonc
{
  "resources":  { "water_ice": "水冰", "iron": "铁", … },   // 原始 key → 中文名
  "buildings":  { "residential": { "label":"居住区","role":"housing","construction_speed":1.0,
                   "build_cost":{…},"staff_per_area":0.0,"productivity":1.0,"default_invest_weight":1.5 }, … },
  "ships":      { "corvette": { "label":"护卫舰","hull":12.0,"hull_regen":0.04,"attack":6.0,
                   "speed":2.6,"attack_range":0.4,"build_points":15.0,"build_cost":{…},"upkeep":1.5 }, … },
  "economy":    { "production_rate":0.5, "pop_growth":0.04, "min_efficiency":0.1,
                  "invest_fraction":0.3, "housing_buffer":1.25 },
  "combat":     { "war_threshold":-20.0, "siege_range":0.25, "arrival_eps":0.06,
                  "armor_regen":0.25, "colony_footprint":0.2 },
  "diplomacy":  { "attack_delta":-3.0, "capture_delta":-25.0, "drift_rate":0.02, "war_fatigue":0.12,
                  "ceasefire_relation":-6.0, "affinity_floor":-42.0, "affinity_span":70.0,
                  "noise":1.0, "hostility_floor":-60.0, "friendship_ceiling":40.0 },
  "market":     { "auto_trade_limit":80.0, "working_buffer":6.0, "spread":0.15,
                  "resource_value": { "iron":1.0, "carbon":1.0, "uranium":5.0, … } }
}
```

**命名约定**：`meta` 的一切 key 都用**原始配置 key**（`carbon`/`corvette`/`residential`），与 `control`
模板、`apply` 的机器格式一致；只有 `resources` 表给出 key→中文名 的映射。状态视图里资源/矿藏用**中文名**
（人看直读），要写回 `control` 时经 `meta.resources` 反查原始 key。

### 1.1 下发指令（可控制状态 diff）

模块化约束把「可控制状态」（指令）与「实体演化结果」分开：`State::control` 是每个势力的
（舰船指令 `ship_orders` / 投资预算 `investment_budget` / 建造预算 `construction_budget` /
建设投资权重 `invest_weights` / 建造投资权重 `build_weights`），每个叶子带 `mode`
（`Ai`|`Player`|`null`=继承）；`State::scope` 是一棵「谁负责决策」的作用域树（全局→势力→天体→城市）。

**双层预算**：势力有 `investment_budget`（建设，按各建筑建设投资权重竞争）与 `construction_budget`
（造舰，按各建造区建造投资权重竞争），两者独立、直接设置、不互相竞争。

用 `control` 读出可编辑模板，改动后以同一 JSON 形状写回，`--apply file.json` 或 REPL 的
`apply <file>` 会**结构化、多层级**地应用它——**只触碰文件里出现的势力/叶子**：

```bash
# 圆 0：读取模板，然后把中国(3) 的 0 号舰改成「玩家控制、攻击 3 号美舰」
planet_x --seed 42 --script control.txt        # control.txt: `control\nquit`
# 写 diff.json：
# {"control":[{"faction_id":3,"ship_orders":[{"ship":0,"behavior":{"TargetShip":{"ship":3,"attack":true}},"mode":"Player"}]}]}

# 应用 diff，推进 6 回合，输出编年史
planet_x --seed 42 --start state_round_0000.ron --apply diff.json --rounds 6
```

**建筑增删改（结构命令）**：`control[].buildings` 可增删/改建筑。`remove: true` 删除指定建筑；
`building: null` 表示新增（指定 `kind`/`structure`/`ship_type`/`resource`/`area`）；`building: <id>`
表示改属性（如 `structure` 混凝土/钢结构、建造区 `ship_type` 舰型）。

```jsonc
{"control":[{"faction_id":3,"buildings":[
  {"city":0,"building":null,"kind":"construction","structure":"steel","ship_type":"cruiser","area":6},
  {"city":0,"building":3,"structure":"steel"},
  {"city":0,"building":1,"remove":true}
]},"scope":null}
```

**最小修改语义**：只列出想改的叶子即可；某叶子里省略 `value`/`behavior` 保留其当前值，省略
`mode` 保留其当前模式；整叶不出现则完全不动。既可只移动一条船，也可整体替换一个势力，
或任何中间层级。`mode: null` 表示继承上层作用域。

> 在 agent 语义下 `--round N` 只输出 JSON Lines，不写 `trajectory/*.ron`。需要从某回合续玩时，
> 可用本作的 **checkpoint**：`--round N --save <file.ron>`（或 REPL `save <file.ron>`）会把当前
> `State` **连同 PRNG 位置**写入 `.ron`，之后 `--start <file.ron>`（或 REPL `load <file.ron>`）恢复。
> 因为随机流也一并保存，续玩会**逐字节复现**出若继续运行的那些回合（已用 12 回合连续运行对照验证）。
> `--start` 也兼容旧的单项 `State` `.ron`（此时用 `--seed` 重新播种，随机流不延续）；也接受 web 端
> `PLANET_X_START` 生成的状态文件。

---

## 命令行参数

| 参数 | 说明 |
|---|---|
| `--seed <SEED>` | 确定性随机种子。数字或 `random` / `随机`（默认），默认随机生成。 |
| `--start <PATH>` | 从指定的初始 `State`（`.ron`）加载；也接受 `--save` 写出的 **checkpoint**（含 PRNG 位置，可确定性续玩）。不传则程序化生成默认太阳系。 |
| `--round <N>` | 运行 `N` 个回合并把每个回合输出为一行 JSON（回合 0 先）；别名 `--rounds`。 |
| `--query <JQ>` | 对 agent 状态执行一个 jq 过滤并输出 JSON Lines；带 `--round N` 先推进 N 回合。配合 `--meta` 时对配置做 jq。 |
| `--meta` | 输出整份游戏配置（规则字典）为一行 JSON 后退出；resource key→中文名、建筑/舰船全表、经济/战斗/外交常量。 |
| `--apply <PATH>` | 把一份控制状态 diff（JSON，同 web `POST /api/command` 的 `{control,scope}` 形状）结构化成多层级补丁叠加到状态，再继续其它模式。 |
| `--save <PATH>` | 批量运行（`--round`/`--query`）结束后把当前 `State` 连同 PRNG 位置写入 `.ron` checkpoint，供 `--start` 确定性续玩。 |
| `--script <FILE>` |（别名 `--commands`）从文件非交互读取 REPL 命令执行后退出；stdout 为纯 JSON Lines。 |

其它：配置文件路径由 `PLANET_X_CONFIG` 环境变量指定，默认 `config/game.ron`。

---

## Agent 输出 schema（JSON Lines）

每行一个 JSON 对象，字段顺序固定：

```jsonc
{
  "round": 1, "time_month": 1.0,
  "factions": [{
    "id": 3, "name": "中国",
    "resources": { "水冰": 40.0, "硅": 2.0, "碳": 26.0, "铁": 7.0 },   // 非零、>=0.05
    "relations": { "俄罗斯": -5.98, "欧盟": 10.01, "美国": -49.85, "行星X崇拜教": -52.51 },
    "wars": ["美国", "行星X崇拜教"]                                     // relation <= 交战胜阈值
  }],
  "bodies": [{ "id": 2, "name": "地球", "position": [-0.74, -0.65],
               "orbit": { "perihelion": 0.98, "aphelion": 1.02, "period": 12.0 },
               "settlements": [{ "name": "长三角", "total_area": 120.0,
                                 "ecological_capacity": 25.0, "construction_speed_mod": 2.4,
                                 "construction_resource_mod": 1.0,
                                 "deposits": [{ "resource": "铁", "area": 40.0 }, … ] }, … ] }],
  "cities": [{
    "id": 0, "name": "长三角", "body": "地球",
    "settlement_index": 0, "settlement_name": "长三角",          // 定居点 ↔ 城市 1:1
    "owner": 3, "owner_name": "中国", "population": 1400,
    "razed": false,                                                    // 被夷平为空白，可再殖民
    "armor": 60.0,                                                     // 城市总硬度 = Σ building armor（无独立城防）
    "ship_progress": { "corvette": 30.0 },                             // 建造进度以城市为单位，按舰型
    "buildings": [{ "id": 0, "kind": "residential", "structure": "concrete",
      "resource": null, "ship_type": null, "area": 56.0, "deployed": 56.0,
      "armor": 28.0, "armor_max": 28.0 }]                             // structure: concrete 混凝土 | steel 钢结构
  }],
  "ships": [{
    "id": 0, "name": "护卫舰-3", "class": "corvette",
    "owner": 3, "owner_name": "中国", "position": [-0.52, -0.67],
    "hull": 12.0, "hull_max": 12.0,
    "order": { "type": "target_ship", "ship": 3, "attack": true }      // tag 联合：idle / move / target_ship / target_settlement / dock / colonize
  }]
}
```

- `order.type` 取值：`idle`、`move`、`target_ship`、`target_settlement`、`dock`、`colonize`。
- `events`（每回合一条，`type` tag）：`attack` / `ship_destroyed` / `siege` / `city_razed` /
  `ship_spawned` / `colony_founded` / `stale_order` / `war_started` / `war_ended` / `story`
  （`story` 事件只带 id/title，完整叙事在编年史 `.story`）。外交跃迁 `war_started`/`war_ended`
  在任一势力跨越战争阈值的回合发出。
- `story`：仅在 `--query` / `q`/`story` 命令给出的状态下作为顶层字段出现（`render_state`
  的每回合 JSON 为了省 token **不含**整段编年史；要看本回合剧情看 `events` 里的 `story`，
  要看整段弧用 `story` 命令或 `--query '.story'`）。每条编年史为
  `{ round, id, title, body, participants:[…] }`。
- 定居点 ↔ 城市**一一对应**：`bodies[].settlements` 是天体上的定居点列表（含名字/面积/矿藏），
  `cities[]` 用 `settlement_index` 指向自己占据的那个定居点；一座定居点至多一座城市，
  城市被夷平（razed）后仍占位，只能被**再殖民**回填，不会被叠第二座城。
- 矿藏/资源在状态里用**中文名**直读；要写回 `control`（原始 key）时经 `meta.resources` 反查。
- 引用一律用整数 id（`faction` / `body` / `city` / `ship`），名称字段便于直读。

---

## 复现

`--seed` 决定整条轨迹；同一 `seed` + 相同配置 + 相同回合数 → 输出逐字节一致。用 `--start` 可从任意保存的状态续玩。

要确保续玩的后续回合**也**逐字节一致（随机流不重头），用 checkpoint 保存/恢复：

```bash
planet_x --seed 42 --rounds 20 --save run.ron     # 20 回合后存档（含 PRNG 位置）
planet_x --start run.ron --round 5                # 续玩 5 回合，复现若继续的结果
```

`--start` 也接受旧的单项 `State` `.ron`，此时用 `--seed` 重新播种、随机流不延续（回合末状态正确，但后续随机决策可能与原轨迹不同）。

---

## 快速上手示例

```bash
# 给 agent 读：20 回合 + 初始，共 21 行 JSON
planet_x --seed 42 --rounds 20

# 一次性的 jq 提问
planet_x --seed 42 --query '.factions[] | select(.id==3) | {id, name, wars}'

# 交互式查询 REPL（stdin 发命令，stdout 回 JSON）
planet_x --seed 42

# 从保存的回合状态续玩（继续推进 5 回合）
planet_x --start state_round_0020.ron --round 5
```
