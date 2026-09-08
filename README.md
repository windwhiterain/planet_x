# 行星X — 太空沙盘

《行星X》是一个回合制（每回合 = 1 个月）的太阳系沙盘轨迹生成器：经济（采矿 / 人口 / 建设）、飞船战斗、围城、外交全部在一个可确定复现的模拟器里推进。天体、资源、建筑、舰船均由 `config/game.ron` 数据驱动，不硬编码。

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
planet_x --seed 42 --query '.cities[] | select(.owner_name=="中国") | {name, population, defense}'

# 打完 5 回合后，看受伤的船
planet_x --seed 42 --rounds 5 --query '[.ships[]] | map(select(.hull < .hull_max)) | .[] | {name, hull}'

# 打完 8 回合后，看被攻打的城
planet_x --seed 42 --rounds 8 --query '.cities[] | select(.defense < 40.0) | {name, owner_name, defense}'

# 势力名单 / 交战对象
planet_x --seed 42 --query '.factions[] | select(.wars | length > 0) | .name'
```

**查询 REPL**（`planet_x`，无 `--round`）：stdin 发命令，stdout 回 JSON。

```
q <jq>                  # 对当前状态执行任意 jq 过滤（JSON Lines）
summary                 # 紧凑雷达（回合/时间/计数/各势力交战）
advance [n]             # 推进 n 回合（默认 1），然后打印 summary
control                 # 输出当前可控制状态（control + scope）JSON，即 agent 可编辑的模板
meta [<jq>]             # 输出游戏配置（规则字典）：资源 raw key→中文名、建筑/舰船全表、经济/战斗/外交常量
apply <file.json>       # 把一份控制状态 diff 叠加到状态上，然后回读 control
cities / ships / factions / bodies
city <id> / ship <id> / faction <id> / body <id>   # 单实体详情
guide / help            # 命令目录（JSON）；q/query、s/summary、a/advance、quit/exit
quit / exit
```

`--script <file>` 从文件非交互读取上述命令并执行后退出（stdout 仍是纯 JSON Lines）。`--apply <file.json>` 在任何命令运行前把一份控制状态 diff 叠加到状态上，二者常与 `--start` 组合成一个回合的 agent 决策循环。

```bash
printf 'summary\nadvance 1\nq .ships[] | select(.order.type != "idle") | {name, order}\nquit\n' \
  | planet_x --seed 42
```

支持的 jq 子集：管道 `|`、路径 `.a.b` / `.[]` / `[0]` / 切片、`select` / `map` / `map_values`、`{..}` 对象构造（含简写）、数组 `[..]`、`sort` / `sort_by` / `reverse` / `length` / `keys` / `unique` / `add` / `first` / `contains` / `startswith` / `endswith` / `type` / `empty`，比较与逻辑运算 `== != < <= > >= and or not`、算术 `+ - * /`、`//`。字符串与数字字面量，数字按值比较（`2 == 2.0`）。

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
  "buildings":  { "residential": { "label":"居住点","role":"housing","construction_speed":1.0,
                   "build_cost":{…},"staff_per_area":0.0,"productivity":1.0,"default_invest_weight":1.5 }, … },
  "ships":      { "corvette": { "label":"护卫舰","hull":12.0,"attack":5.0,"speed":1.6,
                   "attack_range":0.4,"build_points":15.0,"build_cost":{…} }, … },
  "economy":    { "production_rate":0.5, "pop_growth":0.04, "min_efficiency":0.1,
                  "invest_fraction":0.3, "housing_buffer":1.25 },
  "combat":     { "war_threshold":-20.0, "siege_range":0.25, "arrival_eps":0.06,
                  "defense_initial":40.0, "defense_reset":35.0 },
  "diplomacy":  { "attack_delta":-3.0, "capture_delta":-25.0, "relax_rate":0.5 }
}
```

**命名约定**：`meta` 的一切 key 都用**原始配置 key**（`carbon`/`corvette`/`residential`），与 `control`
模板、`apply` 的机器格式一致；只有 `resources` 表给出 key→中文名 的映射。状态视图里资源/矿藏用**中文名**
（人看直读），要写回 `control` 时经 `meta.resources` 反查原始 key。

### 1.1 下发指令（可控制状态 diff）

模块化约束把「可控制状态」（指令）与「实体演化结果」分开：`State::control` 是每个势力的
（舰船指令 `ship_orders` / 资源预算 `budget` / 建设投资权重 `invest_weights`），每个叶子带 `mode`
（`Ai`|`Player`|`null`=继承）；`State::scope` 是一棵「谁负责决策」的作用域树（全局→势力→天体→城市）。

用 `control` 读出可编辑模板，改动后以同一 JSON 形状写回，`--apply file.json` 或 REPL 的
`apply <file>` 会**结构化、多层级**地应用它——**只触碰文件里出现的势力/叶子**：

```bash
# 圆 0：读取模板，然后把中国(3) 的 0 号舰改成「玩家控制、攻击 2 号美舰」
planet_x --seed 42 --script control.txt        # control.txt: `control\nquit`
# 写 diff.json：
# {"control":[{"faction_id":3,"ship_orders":[{"ship":0,"behavior":{"TargetShip":{"ship":2,"attack":true}},"mode":"Player"}]}]}

# 应用 diff，推进 6 回合，输出编年史
planet_x --seed 42 --start state_round_0000.ron --apply diff.json --rounds 6
```

**最小修改语义**：只列出想改的叶子即可；某叶子里省略 `value`/`behavior` 保留其当前值，省略
`mode` 保留其当前模式；整叶不出现则完全不动。既可只移动一条船，也可整体替换一个势力，
或任何中间层级。`mode: null` 表示继承上层作用域。

> 在 agent 语义下 `--round N` 只输出 JSON Lines，不写 `trajectory/*.ron`。需要从某回合续玩时，
> 预先用 `--start` 指定的 `.ron` 状态文件（可由 web 端 `PLANET_X_START` 或外部工具生成）。

---

## 命令行参数

| 参数 | 说明 |
|---|---|
| `--seed <SEED>` | 确定性随机种子。数字或 `random` / `随机`（默认），默认随机生成。 |
| `--start <PATH>` | 从指定的初始 `State`（`.ron`）加载；不传则程序化生成默认太阳系。 |
| `--round <N>` | 运行 `N` 个回合并把每个回合输出为一行 JSON（回合 0 先）；别名 `--rounds`。 |
| `--query <JQ>` | 对 agent 状态执行一个 jq 过滤并输出 JSON Lines；带 `--round N` 先推进 N 回合。配合 `--meta` 时对配置做 jq。 |
| `--meta` | 输出整份游戏配置（规则字典）为一行 JSON 后退出；resource key→中文名、建筑/舰船全表、经济/战斗/外交常量。 |
| `--apply <PATH>` | 把一份控制状态 diff（JSON，同 web `POST /api/command` 的 `{control,scope}` 形状）结构化成多层级补丁叠加到状态，再继续其它模式。 |
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
               "settlement_area": 120.0,
               "settlement": { "ecological_capacity": 25.0, "construction_speed_mod": 2.4,
                               "construction_resource_mod": 1.0,
                               "deposits": [{ "resource": "水冰", "area": 40.0 }, … ] } }],
  "cities": [{
    "id": 0, "name": "长三角城市群", "body": "地球",
    "owner": 3, "owner_name": "中国", "population": 1400, "defense": 40.0,
    "building": "corvette",                                            // 当前建造的舰级
    "ship_build_progress": 0.0,                                        // 距造出该舰还差的建造点
    "buildings": [{ "kind": "residential", "resource": null, "area": 56.0, "deployed": 56.0 }]
  }],
  "ships": [{
    "id": 0, "name": "护卫舰-3", "class": "corvette",
    "owner": 3, "owner_name": "中国", "position": [-0.52, -0.67],
    "hull": 12.0, "hull_max": 12.0,
    "order": { "type": "target_ship", "ship": 2, "attack": true }      // tag 联合：idle / move / target_ship / target_settlement
  }]
}
```

- `order.type` 取值：`idle`、`move`、`target_ship`、`target_settlement`。
- `settlement_area` / `settlement` 为非定居点天体为 `null`（`settlement_area` 是便捷字段 = `settlement.total_area`）。
- 矿藏/资源在状态里用**中文名**直读；要写回 `control`（原始 key）时经 `meta.resources` 反查。
- 引用一律用整数 id（`faction` / `body` / `city` / `ship`），名称字段便于直读。

---

## 复现

`--seed` 决定整条轨迹；同一 `seed` + 相同配置 + 相同回合数 → 输出逐字节一致。用 `--start` 可从任意保存的状态续玩。

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
