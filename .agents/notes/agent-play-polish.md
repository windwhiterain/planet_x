# Agent 游玩体验打磨（真玩一局 → 修掉「静默失败」与文档腐烂）

> 状态 `[x]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §26

> 动机：`agent-play-friction.md` 把「agent 玩起来难受在哪」列成了清单，但那一轮是**读代码猜**的。这一轮的做法是
> **真的以 agent 身份玩一局 `--seed 7`（读 → 决策 → `--apply` → 观察 → 复现）**，把摩擦点
> 按「实际踩到的顺序」记下来，只修其中**会让人（agent）做出错误决定**的那些。
> 结论先行：**最贵的不是「功能不够」，而是「失败看起来像成功」。**

### 26.1 实测踩到的摩擦点（按严重度）

- `[x]` **① 手册教的控制面早已被移除（最贵的一条）**：`agent-play.md` §4.2 的招牌范例写
  `{"type":"target_ship","ship":"华盛顿","attack":true}`，而 `combat-behavior-doctrine.md` 的**行为重构**早已删掉
  `TargetShip`/`TargetSettlement`（攻击是自动的）。照手册抄 = 直接失败，而且报错是 serde 的
  `invalid value: map, expected map with a single key`——**不点名任何字段，也不给替代写法**。
  **一个 agent 手册里最该能抄的那段代码是坏的，这是本轮最贵的摩擦。**
- `[x]` **② 引用不存在的实体 → 静默 no-op**：`{"ship":"不存在的舰"}` 退出码 `0`、stdout 正常、
  什么都没改。舰名**会换代**（`长城` → `长城2`）、城会被夷平易主，所以这在长局里**必然**发生；
  agent 会带着「命令已下达」的错觉继续玩，再把后果归因到别处（这比崩溃危险得多）。
- `[x]` **③ 一个错别字造出幽灵势力**：`faction_id` 写错时 `control.entry().or_default()` 会
  **凭空建一个势力条目**，它随后出现在 `--control` 模板与 web 控制面里，看起来像一个真的
  势力（只是永远不动）。实测 `--control` 从 9 个势力变成 10 个，多出来那个叫「不存在势力」。
- `[x]` **④ 打错字段名 → 整条意图蒸发**：`ship_order`（少个 s）被 serde 静默忽略，退出码 0。
- `[x]` **⑤ `buildings[]` 是整面里静默失败点最密的地方**：未知 kind / 未知 structure /
  不是自己的城 / 下标对不上 / 对非建造区设 `ship_type` / 对非开采区设 `resource`——六条路径
  **全是裸 `return`**。
- `[ ]` **⑥ 模板不是合法 diff（低危，但会误导）**：`--control` 的 `invest_weights[]` 条目带
  `kind`/`resource`/`ship_type`/`structure`、`build_weights[]` 带 `ship_type`，而这些字段
  **不在 `InvestWeightPatch`/`BuildWeightPatch` 里**——模板能回传只是因为未知字段被容忍。
  这也是「给这两个类型加 `deny_unknown_fields`」会被自己的模板卡住的原因。

### 26.2 已落地的修法（全部行为中性，见 26.3）

- `[x]` **`apply_diff` 返回 `ApplyReport{applied, skipped:[{path,value,code,reason}]}`**
  （`src/control.rs`）：**每一条** `continue`/`return` 都换成了带路径与原因的记账
  （`no_such_ship`/`not_your_ship`/`no_such_faction`/`no_such_city`/`not_your_city`/
  `no_such_building`/`no_such_resource`/`not_a_shipyard`/`not_a_mining_building`/
  `no_such_kind`/`no_such_structure`/`no_such_body`/`missing_city`）。
  理由写进了类型文档：**丢弃本身不是错误**（「只触碰 diff 里出现的叶片」允许引用已死实体），
  但它是 agent 唯一能发现「命令没下达」的渠道。
- `[x]` **CLI 在 stderr 上回执**（`src/main.rs`）：有丢弃 →
  `{"code":"WARN_APPLY_SKIPPED","applied":N,"skipped":[…]}`（**退出码仍 0**——部分落地是合法的，
  web 的 `POST /api/command` 整面回传时更是预期行为，所以刻意不硬失败）；**全部落地则一声不吭**
  （守住 stdout/stderr 的零噪声原则）。stdout 永远只是状态流。
- `[x]` **`normalize_behavior` 未知 tag → 错误（不再 pass-through）**，错误里带
  **diff 路径**（`control[0].ship_orders[0] (ship 「长城」)`）、**合法 tag 全表**，并对已移除的
  `target_ship`/`target_settlement`/`guard` **直接给出替代写法**。合法 tag 列表收成
  `BEHAVIOR_TAGS` 一个常量，紧挨解析器——**错误信息不可能再和解析器漂移**。
- `[x]` **`CommandReq` + `FactionControlPatch` 加 `#[serde(deny_unknown_fields)]`**：字段名打错
  当场报错并列出合法字段。**安全性有守卫**（见 26.3 的 round-trip 测试）：读面
  `FactionControlView` 的键集是写面的子集，所以「编辑模板再回传」（web 的做法）仍然合法。
- `[x]` **幽灵势力修掉**：势力不存在时整条补丁被丢弃并报 `no_such_faction`，不再污染
  `state.control`。
- `[x]` **文档重写**（`.agents/agent-play.md`）：§0 三条坑 → 四条（补「`building` 是城内 u32 下标」
  与「**一定要读 stderr**」）；新增 §0 的「攻击/轰炸不是指令」提示；§3 换成真实的六种行为 +
  `ship_doctrine`/`ship_kiting`；§4.2 招牌范例改成能跑的 `follow`/`dock`；§4.3 修正 tagged 形式；
  §4.4 修正 `building` 例外；**新增 §4.5「`--apply` 的回执」**（stderr 契约 + `skipped[].code` 全表）；
  §5 加入「读回执」这一步；§8 补 5 条真实坑（改名换代、`view_*` 返回类型不统一、`events` 的类型列
  叫 `type` 而 `q.history` 的第一个参数才叫 `kind`…）。
- `[x]` **§6.1「红线是量出来的」**（把 `agent-play-friction.md` §18.6 的「量化阈值」补上，用本轮**真实游玩数据**）：
  `upkeep/production_value > 0.8` 该收手、`power_share ≳ 0.5` 必招合纵，并给出同一 seed 下
  「被动基线」与「被引导」两条轨迹的对照。

### 26.3 验证（这一轮的证据是「真玩 + 行为中性」）

- `cargo test --lib` **75 → 82 passed**（+7 条新守卫：合法 diff 必须 `is_clean` 且计数正确、
  已战沉舰必须报 `no_such_ship`、别人的舰报 `not_your_ship`、错别字势力**不许造出幽灵势力且不许
  泄漏进模板**、跨城 `building` 下标必须报出**该城真实下标列表**、拼错字段名必须报错并列出合法字段、
  **模板必须能整面回传**）；`cargo test --workspace` 全绿（含 longhorizon 6 passed/8 ignored）。
- `[x]` **行为中性（golden 对比）**：`--seed 7 --round 60 --index` 的
  `main.jsonl`/`meta.json`/`schema.json`/`idx/*.jsonl` **逐字节一致**；带真实 diff 的
  r60 干预局（`apply steer_defend.json`）也**逐字节一致**。改动只加可观测性，不碰模拟。
- `[x]` **端到端复玩**（同一 seed，分段 `--start`/`--save`/`--index` 续玩）：
  - **被动基线**：`中国` r20–30 舰队全灭 → r29–34 四城被夷平 → r35 起只剩 1 城，
    `upkeep` 一路涨到 53。世界在 俄罗斯 → 欧盟 → 美国 之间轮换霸权。
  - **被引导**：`dock 地球` + `ship_kiting:-1.0`（风筝，别拿护卫舰撞战列舰）→ **r30 时 6 舰满血**
    （基线 0 舰）、4 城全在、**世界仍无霸权**；一路赢到 r60 变成 **11 城 / 份额 0.51** →
    招来全网合纵 + 制裁 → r75 `upkeep 97.9 > 产出 83.1` → **r90 崩回 1 城**。
    → **实测复现了 §6 的设计护栏**：`中国` 的两种死法是同一种过度（先「打光」，后「赢到招恨 +
    造到养不起」）。这条轨迹也第一次证明：**`--control-plan` 的 `bleeding` 不是理论警告**。

### 26.4 未做（留给以后，各自带理由）

- `[ ]` **`agent-play-friction.md` §18.1 的语义指令助手**（`--fight`/`--build`/`--hold`/`--colonize`/`--loyalty`）——
  本轮**刻意不做**：先把「写错了会告诉你」这条路修通，再谈「不用写」。现在的
  `WARN_APPLY_SKIPPED` + 点名到叶的路径已经让手写 diff 可以自纠；而高层命令会引入
  「意图 → 叶片」的第二套真相，得先想清楚它和 `--control-plan` 的成本预览怎么合。
- `[ ]` **模板瘦身 / `--control <faction>` 收敛**（`agent-play-friction.md` §18.3 的 `--focus`）：`--control` 整面
  17 KB（≈9 个势力），agent 每次只想看一个。低风险但会动读面契约，单独一轮做。
- `[ ]` **`skipped` 的机器可读原因码该有个 schema**：现在 `code` 是文档里的字面量表（见手册
  §4.5）。若哪天要程序化消费，应像 `GameEvent::kind()` 一样做成**穷尽 match 的单一权威**，
  而不是散在 `report.skip(...)` 调用点。
- `[ ]` **`_golden_compare.py` 不在仓库里**：`sparse-history.md` 提到的
  `python play/_golden_compare.py <baseline> <after>` 在本 worktree 找不到（本轮用 `cmp` 逐文件
  手做）。要么把它补进仓库，要么把手册里对它的引用去掉。
- `[ ]` **`view_*` 返回类型该统一**（`agent-play-friction.md` 遗留）：`view_sitrep` 返回 dict 但**内嵌 DataFrame**
  （`json.dumps` 直接炸），`view_frontier` 返回 DataFrame。要么统一成 DataFrame，要么让
  `view_sitrep` 彻底 JSON 化。
- `[ ]` **（游戏侧，非体验侧）平衡观察**：官方 AI 的 `中国` 会**反复**走「舰队送死 → 造养不起的
  舰 → 被夷平」这条路（两种 seed 都一样），且 `sparse-history.md` 遗留的 `upkeep_shortfall` 报废与这里同源。
  这看起来是 `autocontrol` 的舰队使用/造舰节奏问题，**不是 agent 体验问题**，但值得单独一轮。
