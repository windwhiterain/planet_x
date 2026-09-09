# 《行星X》数据 schema/查询架构再造 — 调研与设计

> 目标：让这套「沙盘 + agent 查询」架构具备**无限的扩展可能性**，同时把「schema
> 查询维护」的**心智负担**压到最低。
>
> 本文是调研+头脑风暴性质：先诊断现状（为什么现在难维护），再给出分层的设计方向
> （从「本周能做的杠杆」到「长期才需要的彻底重构」），最后落到本仓库的具体改造。
> **不涉及向前兼容**（仓库约定），所以可以放手升级。

---

## 0. 一句话结论

> **现在难维护、心智负担重的根因：同一个游戏世界被「手工」维护成了 4–5 套并行的
> schema（`State` / `AgentState` / `meta_value` / web 视图 / control 面），且 schema
> 是**隐式**的——agent 必须把整棵 JSON 树背下来才能写查询；query 引擎又是**宽容**的
> （缺字段返回 `null`，不报错），于是 schema 一漂移，旧查询就**静默**失效。**
>
> 治本方向：让 schema **单一来源 + 自描述 + 可版本化**；让 agent 用**稳定的语义视图
> 工具**而不是裸 jq 路径；raw jq 只当逃生舱。最后才是 ECS/事件溯源那种「无限扩展」的
> 彻底重构（投入大，非现在必需）。

---

## 1. 现状诊断：抽丝剥茧看瓶颈

### 1.1 事实：游戏世界只有一份「真身」，却有 4+ 套投影

真身是 `model.rs` 的巨型 struct `State`（以及其下的 `Body`/`City`/`Faction`/`Ship`/
`Building`/`ControllableState`/`GameConfig`……）。它被 `serde` 直接序列化成 `.ron`
持久化。但为了「agent 可读」和「web 可读」，又手工维护了多套**并行**的 schema：

| 投影 | 位置 | 作用 | 新增一个字段要改的地方 |
|---|---|---|---|
| `State`（真身） | `src/model.rs` | 内部演化 + `.ron` 持久化 | ① 加字段 |
| `AgentState`/`AgentFaction`/… | `src/agent.rs` | LLM 零噪声 JSON 视图 | ② 复制一遍字段③ 手写映射 |
| `meta_value` | `src/agent.rs` | agent 的「规则字典」（resources/buildings/…） | ④ 手写字段清单 |
| `MetaView`/`FactionView`/`ShipOrderEntry`/… | `src/web.rs` | 前端 JSON 视图 | ⑤ 再复制+映射 |
| control 面 / diff 面 | `src/web.rs` + `main.rs` | 可编辑控制面 | ⑥ 再一套 wire type + patch 逻辑 |

**这就是「schema 查询维护困难」的第一层含义**：改一个机制，至少碰 4–5 个文件，还各自
独立演进，极易漏。而且这些投影**语义重复**（`AgentCity` 几乎就是 `City` 的镜像），
改真身时所有投影必须同步，否则 agent 看到的和模拟算的不一致。

### 1.2 证据：schema 已经漂移了

`config/game.ron` 的 `combat` 目前有 `retreat_hull / retreat_min_dist /
component_spill / component_repair / escort_range / pursuit_range / pd_radius` 等字段
（都在 `model.rs` 的 `CombatConfig` 里）。但 agent 的规则字典 `meta_value` 的 `combat`
段只暴露了：`war_threshold / siege_range / arrival_eps / armor_regen / colony_footprint /
retreat_hull / retreat_min_dist`——

**漏了 5 个**：`component_spill`、`component_repair`、`escort_range`、`pursuit_range`、
`pd_radius`。这 5 个恰好是最近几轮加的战斗参数（模块损毁/修复、护航、不追远敌、舰队
防空）。也就是说：**agent 已经拿不到一部分当前生效的战斗规则了**。这就是「维护困难」
的直接证据——每次加配置，`meta_value` 这套手写清单都被漏改。

> 这个漂移为什么不爆炸？因为 query 引擎宽容。见 1.3。

### 1.3 query 引擎：手写的 jq 子集，且「宽容到静默」

`src/query.rs`（1304 行）是一个自研的 jq 子集（`jaq` 不稳定，所以自己写）。它支持的
语法刻意做窄，好处是零依赖、可嵌、可控；代价有两条：

1. **它是又一个要维护的东西**。agent 的查询需求一增长，就要给这个子集加功能
   （`group_by`、`map_values`、`//` 等是后来一个个加的）。等于「查询语言本身」成了
   一个持续要维护的子系统。
2. **语义宽容 → 静默失败**，这是心智负担的元凶：
   - `eval_path` 对 `.不存在的字段` 返回 `Null`（jq 的宽容），不报错；
   - `Compare` 对空流返回 `false`；
   - 所以 `q '.cities[] | select(.morale > 0.5)'` 在 schema 里根本没有 `morale` 时，
     会**安静地返回空**，而不是告诉 agent「字段不存在」。

   当 agent 凭记忆写查询、schema 又漂移时，结果就是**查询悄悄变空/变错**，且没有任何
   信号让 agent 知道是「schema 变了」而不是「世界正巧空」。agent 就得反复试、反复猜，
   这就是「心智负担重」的第二层含义。

### 1.4 无版本化：持久化状态一改就静默坏

`State` 是直接 `serde` 序列化成 `.ron` 的。加字段靠 `#[serde(default)]` 兜底（旧档能
load），但**没有任何 `schema_version` 或迁移钩子**：一旦某个字段语义改了（比如把
`loyalty` 的 0..1 改成 -1..1，或把 `building.kind` 的取值集合改了），旧 `.ron` 会被
**静默**加载成「新语义但旧数据」——错误藏在数据里，而不是在加载时报出来。`guide` 里
虽有 `schema_version: "1.2"`，但那只是个字符串提示，不是真正的迁移机制。

### 1.5 隐式 schema：agent 必须「背下来」

没有机器可读的 schema。agent 想查询，只能靠：(a) 读 `guide`（命令清单，不含字段
结构）；(b) 跑一次 `summary`/`state` 看**当前**实例长什么样，然后照着写；(c) 凭记忆。
每次加字段，agent 的内存图就失效一次。**这本质上把「schema 维护」外包给了 LLM 的
上下文记忆**——而上下文是会蒸发、会过期的。

### 1.6 小结：四个可击穿的耦合点

1. **多套并行 schema**（改一机制碰 5 个文件，且已在漂移）。
2. **schema 不自描述**（agent 要背，不能查）。
3. **query 引擎宽容**（漂移后静默失败，无信号）。
4. **无版本化/迁移**（持久化数据语义变化被静默吞掉）。

后面第 3 节的每一层方案，都对着这四个点去打。

---

## 2. 目标：把「无限扩展」翻译成可度量的指标

「无限的扩展可能性」不能只是口号，要分解成**可验收**的性质：

- **E1 无并行 schema**：新增一个实体属性（比如给 `City` 加 `morale`），只需改
  「真身 + 数据驱动定义」一处；`state`/`meta`/web/control 全部自动跟随。
- **E2 自描述**：agent 可以 `q .schema`（或 `schema` 命令）拿到当前世界的 JSON Schema，
  从而发现 `morale` 存在/类型/范围，**而不是背下来**。
- **E3 响亮失败**：查询到不存在的字段时，**报错**（在 `--strict` 模式下）而不是返回
  `null`；于是「schema 变了」立刻可见。
- **E4 版本化+迁移**：`schema_version: u32` + `migrate(old_version, &mut State)`，
  旧档要么显式迁移，要么明确报「版本不匹配」，绝不静默错载。
- **E5 语义稳定**：agent 主界面是**命名的语义工具**（例如 `view fleet`，`view sitrep`），
  底层 schema 对它是实现细节；新机制 = 新增/调整工具，而不是让 agent 重写一堆 jq 路径。
  （对应「query 维护」：维护的是工具，不是散落的查询。）

- **E6（长期/可选）真正无 schema 上限**：实体用 **组件化（ECS）存储**，组件是数据驱动、
  开放集合；新增机制 = 加组件 + 加 system，不动任何既有结构体，也不动任何既有查询。
  这是「无限扩展」的最终形态，但投入大（见 3.3），**不是现在必须做**。

---

## 3. 推荐架构：三层，按投入递增

> 三层不是互斥选项，而是**递增投入的同一方向**：先做 Layer 1（高杠杆、低风险），
> 视需要再上 Layer 2、Layer 3。

### 3.1 Layer 1 — 单一来源 + 自描述 + 响亮失败 + 版本化（本周可做）

**核心思路：把这四件事都变成「自动生成」而不是「手写」。**

1. **让 schema 自动派生，消灭并行结构体。**
   - 引入 `schemars`，给 `model.rs` 的类型加 `#[derive(JsonSchema)]`（或用一个统一的
     derive 宏把 `Serialize + Deserialize + JsonSchema` 合并），让 `agent.rs` 的
     `AgentState` 和 `meta_value` **从这批类型自动生成**。
   - 更彻底：**删除手写的 `AgentState` 投影**，改为直接对真身 `State` 用 serde 属性做
     「零噪声裁剪」（`#[serde(skip_serializing_if=...)]`、`#[serde(rename)]`、自定义
     `serialize_with`）。这样 `AgentState` / `AgentFaction` …. 这一整层镜像 struct 都能
     拆掉，`State` 就是唯一的视图来源。
     - 若担心「真身含内部字段（capital_body 等）不想暴露」，用一套 `deny_unknown_fields`
       + `#[serde(rename, skip_serializing_if)]` 的「视图 struct 极简集」也可以，但要让
       它**从真身 derive/生成**，而非手抄。
   - `meta_value` 同理：改为**直接序列化 `GameConfig` 里的各子结构**（`CombatConfig`/
     `EconomyConfig`/…），把当前手写的字段清单删掉，`component_spill` 这类字段就再也不会
     漏了。
2. **暴露 JSON Schema 给 agent。**
   - 新增 `agent::schema_value(config) -> serde_json::Value`：用 `schemars` 生成整棵
     `State`（或 agent 视图）的 JSON Schema，塞进 `meta`（`meta.schema`）或 `state`/
     `meta` 文档的顶层，并加一个 REPL 命令 `schema`。
   - agent 从此可以 `q '.schema.properties.cities.items.properties | keys'` 去问「城市有
     哪些字段」，把「背 schema」变成「查 schema」。这是**心智负担最大的一个减项**。
3. **查询「响亮失败」开关。**
   - 给 `query::apply` 加一个 `strict: bool`。strict 模式下，`Path` 访问到不存在的字段时
     返回 `Err(QueryError)`（而不是 `Null`）；`--query --strict` / `q --strict` 开启。
   - 效果：schema 一旦漂移，agent 的旧查询**立刻**报「unknown field `morale`」，而不是
     静默返回空——把「维护负担」从「agent 反复猜」变成「一眼看到错误」。
4. **版本化 + 显式迁移。**
   - `State` 加 `schema_version: u32`（`#[serde(default="...")]`，旧档默认 0）。
   - `load_state`/`load_checkpoint` 在解析后调用 `migrate(&mut state)`：一个按版本上升
     的迁移函数，逐档升级；遇到**无法迁移**或字段语义变化需要重算的，回显式错误而非
     继续。
   - 这解决「改字段语义 → 旧 `.ron` 被静默错载」的隐患。

**收益**：E1（一处改）、E2（查 schema）、E3（响亮失败）、E4（版本化）一次到位；
**成本**：加 `schemars` 依赖 + 改 `agent.rs`/`query.rs`/`config.rs` 的几处，**不动模拟
逻辑**，风险低。

### 3.2 Layer 2 — 语义「视图/工具」API 为主，raw jq 退为逃生舱（中期）

**核心思路：agent 的「主界面」从「裸 jq 探 JSON」升级为「命名的语义工具」。**

- 现状其实已有雏形：`summary`、`delta`、`control`、`cities`、`ships`、`city <id>`、
  `order`、`budget`…… 这些**就是**语义工具。问题在于它们大多是「一条固定 jq 的薄壳」，
  工具集**小且不系统**，主界面仍靠 `q` 裸查。
- 方向：把这些提升为**稳定的、有版本的、参数化的语义操作**，例如：
  - `view sitrep`（当前局势雷达）、`view economy <faction>`、`view fleet <faction>`、
    `view frontier`（边缘/失稳城）、`view threat`（威胁评估）、`view market`…
  - 每个工具由**模拟自己**生成所需数据（`sim.rs` 里已有的 `balance_picture`、
    `governance_distance` 等计算可直接复用），**不要求 agent 知道底层 JSON 树**。
  - `q` / `--query` 保留为**逃生舱**（高级/一次性分析用），不再是主路径。
- 好处：**「query 维护」变成「工具维护」**。加一个新机制，要么它自然融入某个已有工具，
  要么新增一个语义工具——agent 的学习成本是「多认识一个动词」，而不是「重写 N 条 jq」。
  且工具可以**做校验**（比如 `order 0 attack 99` 时明确报「目标舰不存在」），把错误在
  工具层就显式化，而不是往下丢给宽容的 jq。

**成本**：中等，主要是**设计**若干语义工具的输入输出契约，并让它们稳定；不涉及模拟改动。

### 3.3 Layer 3 — ECS + 事件溯源：真正「无限扩展」的终局（长期/可选）

当实体类型和属性增长到「每加一个机制都要改 struct」成为常态、或需要在运行时热插拔
机制/模组时，才值得上 ECS：

- **ECS/组件化**：实体（天体/城/舰/建筑/势力）是一个 `id`，其上的**组件**是
  `BTreeMap<String, Component>` 或类型化的组件集合；组件集合**开放**，新增机制 =
  新增一类组件 + 一个 `system`（`advance` 里的处理函数），**不动任何既有结构体**。
  `config/game.ron` 已经这么干了（resources/buildings/ships/components 全是字符串键 +
  配置表），把「资源/建筑/舰级」这套模式**推广到实体属性**即可。
  - 查询也随之更稳：不是探一棵固定 JSON 树，而是「按组件 filter」，新机制带来的新组件
    不破坏既有 filter。
- **事件溯源**：以**追加式事件日志**为真身（现在已有 `State::events` 和 `chronicle`，
  但没有把事件当唯一事实源），用投影（projection）派生出供查询/展示的视图。schema
  演化变得**相加**：新事件类型是新增，旧事件永不改写；投影可以独立重建，也可以并存
  多版。
- **权衡**：这是把整仓从「单一 struct + serde」迁到「组件存储 + 事件日志」的大重写，
  **会破坏 `.ron` 的直观可读性、也非一蹴而就**。所以把它定位为「长期/当需求真的爆了
  才上」。

> 一个重要判断：**本项目的「无限扩展」需求目前还没有强到必须上 Layer 3**。它对模组/
> 运行时机制的诉求是「加配置、加数据驱动表」，而这已由 `config/game.ron` 满足大半。
> 真正的痛点是「agent 查询 + 手工投影」，那正是 Layer 1 / Layer 2 精准命中的地方。
> 所以**优先做 Layer 1（甚至 Layer 2），Layer 3 留作演进方向**。

---

## 4. 具体改造：本仓库「最省事、收益最高」的清单

按优先级排（每项标注涉及文件，均不涉及向前兼容，可放手改）：

> **实现状态（已做 P0–P3，剩 P4）**：
> - P0 已落地（`meta_value` 从 config 结构体派生，`combat` 那 5 个漏字段已补齐）。
> - P1 已落地（`schemars` 生成 `schema_value()`，REPL `schema` 命令可查状态 schema）。
> - P2 已落地（`apply_strict` + `--strict` / `q --strict`，未知字段报错）。
> - P3 已落地（`State::schema_version` + `SCHEMA_VERSION` + `model::migrate`）。
> - P4 未做（拆 `AgentState` 镜像 struct，工作量最大）。详见 `.agents/ideas.md` §11。

### P0 — 让 `meta` 的规则字典不再漂移（先止血）
- **问题**：`agent::meta_value` 手写字段清单，漏了 `combat.component_spill/…
  escort_range/pursuit_range/pd_radius` 等 5 个（实测已漂移）。
- **改法**：把 `meta_value` 的 `economy/combat/diplomacy/market/governance/mond/balance`
  各段改为**直接 `serde_json::to_value(&config.economy)` 后四舍五入**，或对每个子结构
  写一个 `JsonSchema`/`Serialize` 自动转。删掉手写字段清单。
- **文件**：`src/agent.rs`（`meta_value`）。可顺带把 `web.rs` 的 `MetaView` 一并改为
  复用同一来源。

### P1 — 让 schema 自描述（agent 不再背）
- **改法**：
  - `Cargo.toml` 加 `schemars = "0.8"`（`derive` 特性）。
  - 给 `model.rs` 关键类型加 `#[derive(JsonSchema)]`（或统一一个 `derive` 宏）。
  - `agent.rs` 加 `pub fn schema_value(config) -> serde_json::Value`，用 `schemars` 生成
    `State` 的 JSON Schema，塞进 `meta`/`state` 顶层 `schema` 字段；`main.rs` 加
    `schema` REPL 命令。
- **文件**：`Cargo.toml`、`src/model.rs`（derive）、`src/agent.rs`（`schema_value`）、
  `src/main.rs`（命令 + guide）。

### P2 — 查询「响亮失败」开关
- **改法**：`query::apply(input, filter)` 增加 `apply_strict(input, filter)`；`eval_path`
  在 strict 模式下对不存在的字段返回 `Err(QueryError::new("unknown field …"))`。CLI 加
  `--strict` 与 `q --strict`。默认仍宽容（保证旧查询不因模式更严而炸）；strict 用在
  「新写/校验查询」的场景。
- **文件**：`src/query.rs`、`src/main.rs`。

### P3 — 版本化 + 迁移钩子
- **改法**：`State` 加 `pub schema_version: u32`（`#[serde(default = "default_sv")]`），
  `config.rs` 的 `load_state`/`load_checkpoint` 在解析后调用
  `crate::model::migrate(&mut state)`（一个 `match version` 的逐档升级函数）。无法迁移
  就返回显式错误。
- **文件**：`src/model.rs`、`src/config.rs`。

### P4 — 收敛并行投影（若有余力）
- **改法**：逐步把 `agent.rs` 的 `AgentState` 镜像 struct 拆掉，改为对 `State` 直接用
  serde 属性做用户态裁剪；至少先做到 `AgentCity`/`AgentFaction` 等**从真身生成**而非
  手抄。这是工作量最大的一项，建议放在 P0–P3 之后，且**可以分模块渐进**（先拆
  `AgentShip`，再拆 `AgentCity`…）。
- **文件**：`src/agent.rs`。

---

## 5. 权衡与取舍

- **自研 jq vs 全量 jq（`jaq`）**：现在是自研子集（稳+零依赖），代价是要持续扩。若
  查询需求持续涨，可考虑换/补 `jaq`。但考虑到 Layer 2 已经把主路径从「裸 jq」转向
  「语义工具」，自研子集作为逃生舱**足够用**，不必急着换。
- **语义工具 vs 裸 jq**：语义工具稳定、可校验、学习成本低，但要维护工具**契约**；
  裸 jq 灵活但脆弱。结论：**主语义、副裸 jq**（对应 Layer 2）。这个折衷既保住灵活性，
  又把「查询维护」从每个 agent 反复写 jq 的泥潭里捞出来。
- **ECS vs 保持 struct**：搞 ECS 能最大化扩展性，但破坏 `.ron` 可读性、重写量大。本项目
  目前用「数据驱动字符串键 + 配置表」已覆盖大部分扩展需求，**不必现在上 ECS**；留作
  Layer 3 方向。
- **自动 schema 生成 vs 手写视图**：生成免漂移，但可能暴露不该暴露的内部字段。用
  `#[serde(skip)]`/自定义 `serialize_with` 控制暴露面，仍比维护一套镜像 struct 便宜。
- **strict 查询默认值**：默认宽容是为了不炸旧脚本；strict 用于校验。可考虑在 `guide`
  里建议「agent 写任何新查询先用 strict 跑一遍」。

---

## 6. 触手可及的首步（如果现在就做一项）

> **做 P0（修 `meta_value` 漂移）+ P1（schema 自描述）**：这俩直接命中「agent 拿不到
> 当前规则 + 要背 schema」两大痛点，改动小、不碰模拟、可单独提交验证。
>
> 验证方式（沿用项目既有手段）：
> - `cargo run --bin planet_x -- --seed 7 --round 1 --meta` 应能看到 `combat.component_spill`。
> - 新增 `schema` 命令后：`cargo run --bin planet_x -- --seed 7 --round 0`（或 REPL
>   `schema`），`q '.schema.properties.cities | keys'` 能列出城市字段。
> - `cargo test --lib` 与 `cargo test --test longhorizon`（快守卫）全过。

---

## 7. 附：为什么这个方向能真正「无限扩展」

「无限扩展」的真正敌人不是「类型不够多」，而是**改一个机制要同步 N 套定义、且失败是
静默的**。上面方案逐一拆掉：

| 敌人 | 拆法 | 对应层 |
|---|---|---|
| N 套并行 schema | schema 自动生成 / 从真身 derive，删镜像 struct | L1 |
| 背 schema 才能查询 | 暴露 JSON Schema，agent 直接查 | L1 |
| 漂移后静默失败 | strict 查询：未知字段即错 | L1 |
| 旧档语义被静默错载 | `schema_version` + `migrate()` | L1 |
| 每加机制都要重写散落 jq | 命名语义视图工具为主接口 | L2 |
| 实体属性仍是硬编码 struct | 组件化存储（ECS）——数据驱动任意扩 | L3 |

到那时，新机制 = 「改/加一段数据驱动定义 + 加一个确定性的 system 或工具」，**既有的
查询和投影自动跟随**，agent 的心智负担从「维护一套隐式 schema」降到「多用几个稳定
动词」。

---

## 8. 附：schema「载体格式」选型（回应「纯 Rust dict 还是数据库」）

> 关键：把 **schema**（描述）、**运行时数据**（存储）、**持久化**（落盘）三件事拆开，
> 否则很容易把「格式」选错。结论：**不用数据库；schema 载体用 JSON Schema（生成、
> 自描述）；运行时数据用内存 Rust；持久化用确定性 RON。**

| 层 | 载体 | 理由 |
|---|---|---|
| **权威 schema（唯一真身）** | **Rust 类型**（`model.rs` struct） | 模拟真相就在类型里；schema 从它派生、不手写 → 防漂移 |
| **机器可读「统一 schema」** | **JSON Schema**（`schemars` 从类型**自动生成**） | 标准、能自省、能校验、能 version；agent 可 `q .schema` 查 |
| **运行时查询载荷** | **JSON**（`serde_json::Value`） | agent 用 jq 最顺；schema 描述的就是这份 JSON |
| **持久化** | **确定性 RON**（不换） | 人类可读、易 git diff；配 `schema_version` + `migrate()` |
| **将来热扩展时的实体存储** | 内存**组件包**（`BTreeMap<String, Value>`），**仍非数据库** | 组件是开放集合；也可为它生成 JSON Schema 描述 |

**为什么不用数据库（SQLite/Postgres）**：
1. 确定性是硬约束——DB 的隐藏排序、非确定查询计划、并发访问，与「同一 seed 永远同一部
   历史」直接冲突。
2. 单进程、无并发、状态小（22 城/几十舰），ACID/索引/事务全是白付的运维复杂度。
3. 持久化已是确定性 `.ron`；换 DB 要多维护一份二进制库 + 迁移 + 备份，且**没法 git diff**。
4. agent 用 jq 最顺，SQL 是另一门方言；而世界是**图**（body→city→building、faction→ship），
   硬关系化只会让查询更难写。
5. 破坏「单二进制、零外部依赖、`cargo run` 即复现」的叙事。

**为什么也不该把 schema 裸成纯 dict**：`BTreeMap<String, Value>` 是**绝佳的运行时载荷 /
数据驱动配置表**（资源表、舰级表、组件表已是如此），但**作为 schema** 没有类型系统兜底——
加字段要手写校验、无法自动自省/校验/版本化，恰是「心智负担」的来源。**dict 装数据，
JSON Schema 描述数据，两者分开。**

**无限扩展靠什么载体**：不是换存储，而是「新增机制 = 加一个数据驱动定义」。编译期已知的
字段走 Rust 类型 + `schemars`（加字段 schema 自动跟上）；运行时才想加的实体类型/属性走
**组件包**（内存 dict，非数据库），并可为它生成 JSON Schema 描述（schema 是描述，store
是数据，天然分离）。

---

## 9. 附：三个后续设计决策（struct/dict 判据、RON vs JSON、UI metadata 植入）

### 9.1 struct vs dict：按「schema 一致性」分？——加一条「封闭性」轴

「schema 一致（同构）」这个判据方向对，但**不够**；真正决定 struct 还是 dict 的是
**轴②封闭性**（属性键/取值集合在编译期封闭，还是能靠配置/模组增长）。同构 + 开放（资源表）
→ dict；异构 + 封闭（行为指令）→ enum，不能只看一致性。

| 属性 | 载体 | 例（本仓库） |
|---|---|---|
| 同构 + 封闭 | **struct 字段**（编译器兜底） | `City.population: u32`、`Faction.alignment: f64`、`Body.orbit: Orbit` |
| 同构 + **键集开放** | **`BTreeMap<Key, V>`** | `resources`、`relations`、`ship_progress` |
| 异构 + 封闭 | **enum / tag-union** | `ShipBehavior` |
| 异构 + **开放** | **dict / 动态组件包** | `Ship.components: Vec<String>`、舰级/组件/建筑配置表 |

仓库已大致按此直觉走（资源/关系/进度=`BTreeMap`、人口/意识形态=struct、行为=enum、配置表=
字符串键），只需把「封闭性」显式写进原则，未来不跑偏。

### 9.2 RON 和 JSON 同时存在：**统一 schema，不统一格式**

RON 和 JSON 消费**同一份 `State` 的 serde 派生**——是同一逻辑文档的两种编码，不是两套
schema。真正制造「双轨」的是**手写的平行投影 `AgentState`**。因此：
- **保留 RON** 用于配置+持久化（人类可读写、支持注释、确定性、易 git diff、保留全精度浮点）。
- **保留 JSON** 用于机器交换（agent 用 jq、web API、control diff）。
- **统一 schema**：拆掉 `AgentState`，让 agent JSON 直接从 `State` 用 serde 属性裁剪
  （`skip_serializing_if`/`rename`/`serialize_with`）。JSON 的「四舍五入 2 位」视为**展示层
  丢瓣**，RON 保留全精度为权威。
- **不要**全改 JSON（JSON 不允许注释，`game.ron` 的可读性就废了）；**也不要**全改 RON
  （破坏 jq/web/diff）。

### 9.3 UI metadata（widget tree）如何植入 schema——**夹带进同一份 JSON Schema**

原则：**metadata 生成/附着到同一份 schema，绝不另开手写 `widget_tree` manifest**（否则又
回到第 1 点「第二套 schema 漂移」）。

- **方式 A（便携元数据）**：`schemars` 标注属性 → JSON Schema 标准关键字。
  `#[schemars(description="…")]`→`description`、`#[schemars(range(min=…,max=…))]`→
  `minimum`/`maximum`、`#[schemars(with/example/default=…)]`——WebUI/agent/validator 原生可读。
- **方式 B（UI 专属元数据）**：JSON Schema 的 `x-*` 厂商扩展关键字。用
  `#[schemars(extend="…")]`，或生成后跑一个**装饰 pass**：定义类型化的 `UiMeta` 表
  （字段路径 → `{widget, min, max, step, group, editable, visible_when}`），
  `apply_ui_meta(schema, &ui_meta)` 把 `x-ui-*` 合并进 schema。`model.rs` 保持干净，仍产出
  一份自描述 schema。WebUI 读 `schema.properties.<path>.x-ui` 构建 widget tree；agent 读
  `description`/`range` 看 label 和范围。

**推荐**：便携信息（label/help/单位/范围/枚举）走 A，UI 专属（widget 类型、分组、下拉/滑块/
表格/星图渲染、只读/可编辑、条件显隐）走 B。全程避免「schema 用 A 文件、widget 用 B 文件、
注释放 C 处」。**只要 schema 单一来源，Ron/JSON/codec/UI 元数据都只是它的不同视角。**

---

## 10. 沉淀：「轨迹生成器 + 外部 jq」（Agent 讲故事的批次模型）— **已落地**

> 用户拍板：**外部 jq 为必选依赖；丢 `--strict`；删掉自研 jq 引擎与查询 REPL；但 agent
> 仍可通过 `--apply` diff 影响故事走向。** 本节是落地蓝图（已实现，见 `.agents/ideas.md` §13）。

### 10.1 形象转变：agent 从「玩家」变成「导演 / 说书人」

之前：agent 是**玩家**，每回合 `advance → 读状态 → apply diff → advance`，在**有状态 REPL**
里交替换。现在：agent 是**导演**，重点是**跑一段轨迹、任意时间轴查询、然后讲给用户**——
不再逐回合决策，而是**分段跑**（可随时 `--apply` 定向）+ **批次生成 JSON Lines** + **外部 jq
架空查询**。

### 10.2 要删掉的东西

| 删什么 | 为什么 |
|---|---|
| `src/query.rs`（自研 jq 子集，~1400 行 + 它全部单测） | 只是 CLI 查询口；`web.rs`/`sim.rs`/`agent.rs` 都不用它，爆炸半径仅 CLI。换外部 jq 后整块冗余 |
| `--query` / `q` / `query`（进程内裸 jq 路径） | 交给外部 jq |
| `--strict` + `apply_strict_lines`/`apply_strict_lines`/`strict_query` 辅助 | 真 jq 永远宽容，无法「未知字段报错」 |
| 有状态查询 REPL（`run_agent_repl` 的查询分支）及其 `--script` 脚本循环 | 批次模型用 shell 管道 + `--apply`/`--save` 分段替代 |
| REPL 里的便利查询（`summary`/`delta`/`cities`/`ships`/`city <id>`/`faction <id>`…） | 对轨迹本体直接外部 jq 即可，不必保留为进程内命令 |

### 10.3 保留/新增：agent 仍能影响故事走向

**控制/diff 面是「diff 影响故事」的唯一机制，必须保留**：`web::control_surface` +
`web::apply_patch`。agent 用 `--apply <diff.json>` 在**每段开始时**覆盖控制 diff，从而定向。

**确定性分段**（现已具备，直接用）：`--start <ckpt.ron>` + `--apply` + `--round N` +
`--save <ckpt.ron>`。agent 可「跑一段 → apply 一个 diff → 再跑一段」串成完整轨迹；每段是
**纯函数**（seed/checkpoint 保证可复现），拼接 JSON Lines 即整条时间轴。

**语义产物改为 CLI 标志**（agent 读，或喂给外部 jq / 直接叙事）：
- `--meta`：规则字典（已有）。
- `--schema`：agent 视图的 JSON Schema（新增 CLI 标志；原 REPL `schema` 命令）。
- `--story`：编年史（`chronicle` 数组，叙事弧完整正文，含 round/id/title/body/participants）。
- `--control`：当前可编辑控制面模板，供 agent 写 `--apply` diff。
- `--round N`：每回合 agent 视图 JSON Lines（轨迹主体，含 `.events`；流式、可 `jq -s` 整段收）。
- （可选）`--traj N`：把整段轨迹 + 编年史打包成一个 JSON 文档（story pack），一次拿到。

### 10.4 外部 jq 示例

```bash
# 流式：每 10 回合采一个「城市/舰数/舰队价值」雷达
planet_x --round 240 | jq -c 'select(.round % 10 == 0) | {round, ncity:(.cities|length), nship:(.ships|length)}'

# 整段时间轴聚合（-s 收集）：每回合舰数时序
planet_x --round 240 | jq -s '[ .[] | {round, nship:(.ships|length)} ]'

# 语义聚合：按船主统计各势力舰数
planet_x --round 240 | jq -s '[ .[] | .ships[] ] | group_by(.owner_name) | map({owner:.[0].owner_name, n:length})'
```

### 10.5 `--strict` 的替代

不再做「未知字段报错」。根上靠 **P0**（meta 从 config 结构体派生）+ **P1**（schema 自描述）
防漂移；agent 写任何查询前先 `--schema` 查字段，而不是凭记忆。诚实代价：真 jq 下
「引用了 schema 里不存在的字段」会静默返回 null（且 null 可能是合法值），**无法完美补回**——
这是为「全量 jq + 删 1400 行」付的学费。

### 10.6 代价 / 注意

- **`jq` 成为必选运行时依赖**：生成器本身仍是单二进制（`cargo run` 即复现），但 agent 的
  叙事/查询侧需要宿主装 jq。破坏「零外部依赖」叙事，接受。
- **Web 是面向玩家的，完整保留**（关键分界）：`planet_x_web`（`web.rs`）是给**人类玩家**
  玩的交互前端——它有自己的 `advance`/`apply_patch`/`StateView` 视图，是**有状态、可交互**
  的玩家模型。它**不依赖 `query`**，所以删掉自研 jq 引擎**不影响它**。Web 的
  `control_surface`/`apply_patch` 与命令行 `--apply` 共享同一套控制/diff 模型（同一契约）。
  **结论：删的只是「agent 侧 CLI 的查询 REPL」，Web 玩家界面照常。**
- **`lib.rs` 移除 `pub mod query;`**；`Cargo.toml` 保留 `schemars`（schema 自描述还要用）。

### 10.7 完成后的心智负担对比

| 维度 | 旧（进程内 jq） | 新（外部 jq） |
|---|---|---|
| 查询语言 | 维护一个 1400 行子集 | 用真 jq，零维护 |
| 跨回合/时间轴查询 | 不能（只有当前帧） | 天然（对 JSON Lines 任意 jq） |
| 同一数据多次查 | 每次重投影 | 落盘一次，查任意次 |
| 未知字段防护 | strict（但治标） | 靠 P0/P1 + `--schema`（治本） |
| 单二进制零依赖 | ✅ | ✗（需 jq） |
| agent 定向故事 | 有状态 REPL | `--start`/`--apply`/`--round`/`--save` 分段 |

### 10.8 权威 schema 贯彻（auth-schema worktree）：WYSIWYG key + 拆掉 `AgentState` 镜像

遵循 §3.1 Layer 1 与 §9 的结论，真正「贯彻」了两点（见 `.agents/ideas.md` §14）：

- **资源 key = 可读名（WYSIWYG）**：把 `config/game.ron`、`State`、控制面、world/测试里的
  资源 key 全部从 raw（`water_ice`）改成显示名（`水冰`）。于是「看到什么 key 就是什么 key」，
  `meta.resources` 的 raw→中文 翻译表删除，agent 在状态视图看到的就是它能在 `--apply` 里写的。
- **拆掉 `AgentState` 镜像，agent 视图 = 权威 `Trajectory`**：删除 10 个镜像 struct + `from_state`，
  用一个 `#[derive(Serialize, JsonSchema)] pub struct Trajectory`——字段直接复用权威类型
  `Vec<Body>/Vec<City>/Vec<Faction>/Vec<Ship>` + `Vec<GameEvent>/Vec<ChronicleEntry>`，不含
  控制面。`schema_value()` = `schemars::schema_for!(Trajectory)`，schema 与发射 JSON 同源、不会漂移。
  ——这回答了「agent view 还有必要吗」：**没必要用手写镜像**，因为外部 jq 就是查询层；
  保留的只是一个从权威类型直接复用字段的 `Trajectory`（单一来源）。
- **代价/须知**：不再预计算派生字段（舰 `panel`、城 `armor`/`gov_distance`、`owner_name`/
  body 名、relations 从「按名字」变「按 id」）。agent 用 jq 现场 join/计算（`--meta`/`--schema`
  提供规则与字段）；后续可加 Layer-2 语义视图补省事。
- **守卫**：`meta_value_covers_every_config_section`（meta 覆盖每个 config 段）+ 
  `agent_view_is_self_described_by_schema`（schema 自描述发射的视图）——把「单源 + 自描述」
  用测试钉死，防止未来换生产者时悄悄漂移。
