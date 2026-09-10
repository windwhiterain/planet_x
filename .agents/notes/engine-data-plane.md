# 引擎=数据平面，Python kit=策略平面

> 状态 `[~]`（分界线已裁决、读面缺表已开工） ｜ 索引：[notes.md](../notes.md) ｜ 关联：
> `python-control-authoring.md`（客户端篇：kit 的职责与落地）、`control-live-layers.md`（控制面的形状：
> 三态 + 链 + 写值即接管）、`ship-blueprint.md`（**"还不存在的实体"的规则住那里**）、
> `lazy-index-pandas.md`（投影 schema 契约）、`agent-play-friction.md` §45（`--control-plan` 的
> 成本→收益预览：也是"引擎算、Python 只读"的既成例子）

> ⚠ **本笔记里的读面名字大多已换代**（`feature/pre-post-unify`）：`flow` + `metrics` 两段已合并成
> **一份视图** `RoundView`（`pre`/`post` 同形，每个数只有一个位置）；`RoundFlow` 退成**引擎内部的
> 写入口袋** `RoundSink`（**不出现在读面上**）；`Derived` → `RoundView`、`derived_from_state` →
> `view_from_state`、`round_metrics` → `observe`；表名 `idx/flow.jsonl`/`idx/city_flow.jsonl` →
> `idx/faction_process.jsonl`/`idx/city_process.jsonl`，`main.jsonl` 每行的 `metrics` 键 → `view`。
> 下文**保留当时的原话**（它是落地记录，不改写历史），凡遇到旧名请按这一条读；
> 见 [`pre-post-unify.md`](pre-post-unify.md)。

## 0. 一句话

**引擎只负责两件事：产出某种标准的统计学数据格式（tidy 表，按名字 join），以及接受同一形状的
diff。** 通配、编制表、统计筛选、配方、verify——全部是 Python kit 的事，引擎不为它们加任何功能。

这条分界线**和项目里已有的裁决同源**：`play/planet_xq/README.md` 的「Semantic views」一节写的就是
「Python 视图只打包模拟**已经算好并写进 `view`/懒表**的量（纯检索，永不与规则漂移）；任何需要
游戏公式的判断都留在 Rust」。本 note 是它的**写侧对偶**，加上一条更硬的分界（见 §1）。

## 1. 分界线（用户裁决）

| 层 | 管什么 | 为什么 |
| --- | --- | --- |
| **引擎（链 + 模板）** | **还不存在的实体**的规则：`叶 → 舰队默认 → 势力 → 全局` 的归属解析、新舰继承什么意图、`写值即接管` 的隐含规则、以及舰船模板（出厂快照，见 `ship-blueprint.md`） | kit 只能对**现在存在**的舰说话；"新下水的舰自动继承意图"只有在链上才成立 |
| **Python kit** | **现存**舰/城/建筑的批量与统计操作：通配、名字表、筛选、聚合、配方、`verify` | 这是 pandas 的活儿；引擎不该长出一堆一次性命令 |

### 1.1 已否决（别再提一遍）

* ❌ **引擎侧通配 `{"ship":"*"}`** —— 不需要。写面已经够表达：`{"ship":"长城","mode":"Auto"}` 合法，
  且 `behavior` 缺省时**只改 mode、不动值**（`src/control.rs:742-767`）。"一键把中国的舰全设成自动"
  = kit 展开成 N 条 `{ship, mode}`，与手写 N 条叶是同一个东西，只是不用手抄。
* ❌ **`clear_ship_orders`（清叶动词）** —— 同上，`mode:"Inherit"` 就是"这片叶不再说话"。
* ❌ **`NOTE_APPLY_EXPANDED`（引擎报"展开了几艘"）** —— `fanned_out` 是 kit 打印给自己的，不是回执。
* ❌ **引擎侧通配铺到建筑权重** —— 顺带记一笔坑：`InvestKey = BuildKey = (CityId, BuildingId)`，
  而 `BuildingId = u32` 是**城内下标**（`src/model/control.rs:47`）。跨回合拼下标必错；kit 必须
  **同回合变换**（读 ckpt → 出 diff → 应用到同一个 ckpt），这样下标才自洽。

### 1.2 按舰级默认落在哪

* 要**新造的**护卫舰自动守家 → 引擎（链上加一层 / 舰船模板）——用户裁决：**这就是舰船模板的功能**，
  所以并进 `ship-blueprint.md`，本 note 不另立概念。
* 只要求**现存**护卫舰 → kit 一句 `ships.query("class=='护卫舰'")` 展开成叶（代价：diff O(N)、新舰不跟随）。

## 2. 引擎欠的表（读面）——这就是"标准的统计学数据格式"

投影已经是项目选定的标准：`idx/*.jsonl` 一行一个 `(round, 实体)`、列名固定、**按名字 join**
（`FactionId`/`CityId`/`ShipId` 全是 `String`）。本节写下"当时以为缺什么"，**实际的修正见 §7.2/§7.4**
（写下来是为了让下一个 agent 看到"动手前先核实"这条教训本身）：

| 欠的东西 | 数据在哪 | 当时的状态（⚠ 见 §7.2 的修正） |
| --- | --- | --- |
| **`flow` 表**（`idx/flow.jsonl` 回合×势力：各资源产出 / `upkeep` / `governance.total` / `coverage`；`idx/city_flow.jsonl` 回合×城×资源产出） | `RoundFlow`（`src/model/metrics.rs`，文档原话："各 step 计算并应用、**不落到持久状态、原本不对外暴露的量**"） | 作为**表**确实一张都没发；但**数值**早已被 `round_metrics` 抄进 `metrics`（内联在 main 行里）——所以真实收益是**形状**不是数据，见 §7.2 |
| **`pre` 段**（当时以为 = "本回合依赖 rng 的随机决策"） | `RoundState.pre: Derived`（当时的类型名；`src/model/state.rs`） | ❌ 这个描述**是错的**：`pre` 只是"推进前的观测"（`flow` 恒空）。真正的"AI 掷了什么"任何地方都没记录，要新捕获，见 §7.4 |
| **控制面 tidy 表 + 每实体 `effective`** | `State.control` / `State.scope` | 这条**是真的**：`--control` 有（读面即写面）但**不是投影里的表**，Python 要 join 得再起一次进程；`effective` 没有 → Python 只能自己重实现 `resolve_chain`（`src/model/control.rs`），**那是漂移源**。✅ 本轮已做 |
| **`--derived` 单点导出**（一个 ckpt → `{pre, post}`） | 同上 | 没有；不想为一次 join 跑整个 `--index`。✅ 本轮已做 |

**硬约束（写成测试）**：`post` 是 `state` 的函数（同一 state 恒定），所以 **`--derived` 与 `--index`
里的同一个数必须完全相同**。两个读面各说各话，比缺数据更坏。→ `tests/projection_derived.rs`
（跨进程：`--index --save` 一份 ckpt，再 `--derived` 读它，逐值比对）。

投影 schema 是发射端与 Python kit 共享的契约（`projection_schema()`，有测试断言 `LAZY` 与 schema
一一对应：`src/projection.rs:480`）——**每加一张表，两处都要动，别漏 schema**。

## 3. 引擎接受的形状（写面）：不变

`{control:[…], scope:{…}}` 的 presence-aware 多级补丁，读面即写面（`--control` 的模板原样回传安全），
`--control-schema` 给机器可读定义，回执在 stderr（`WARN_APPLY_SKIPPED` / `NOTE_APPLY_TOOKOVER`）。

**核实结论：写面没有"表达不出来"的东西**（含"交回上层" = `mode:"Inherit"`）。这是这套设计少见的
干净处——所以本轮**不动写面**。

## 4. 一个必须写进 kit 文档的取值坑

「交回上层」（`mode:"Inherit"`）**只在舰队默认是 `Player` 时才是干净的**。`ship_behavior`
（`src/model/state.rs:186-193`）：

```rust
if leaf.mode == Inherit {
    if let Some(d) = &c.default_role { if d.mode.is_player() { return d.value } }   // ← 现在是**倾向**的层；指令那片舰队默认叶 2026-10 已删
}
leaf.value      // ← 否则落到叶子上那个可能已经过期的记录值
```

* 归属是 `Auto` 时**自愈**：系统下一回合按自己的逻辑重写这片叶，陈旧值只是短暂的。
* 归属是 `Player` 而舰队默认不是 `Player` 时**会长期显示旧值**。
* kit 的处理：凡"释放到上层"，通常**同时**把该轴的舰队默认写成 `Player`（一片叶），语义闭合。
  ⚠ **指令没有舰队默认叶了**（2026-10，见 `blueprint-stance.md`）：释放指令 = 叶里的旧值继续算数，
  想让它干活就**写**（`Idle` 也是一个值），不要靠删叶。

## 5. 落地状态（用户授权自行决定顺序）

1. `[x]` **读面补表**（本轮做完，见 §7）：`flow` / `city_flow` / `control` / `scope` / `decisions`
   五张派生表 +
   ships 表的 `order_*` 列（引擎解析的归属/有效指令）+ `--derived` 单点导出 + schema 同步 +
   跨进程一致性测试（`tests/projection_derived.rs`）。
2. `[x]` **"AI 到底掷了什么"要单独捕获**（见 §7.4/§7.5）：`pre` **不是**这个（它只是回合前的观测），
   真数据在 `sim`/`autocontrol` 的决策点做了一次**结构化捕获**（`RoundDecisions` → `idx/decisions.jsonl`），
   并证明了行为中性（`--digest` 与 `main.jsonl` 逐字节不变）。
3. `[x]` **风格活层**（`control-live-layers.md` §4.1+4.2+4.4）：`default_doctrine`/`default_kiting`
   + `Ship.doctrine/kiting` 降级为记录值 + ships 表的 `doctrine`/`kiting` 有效值列。
   提交 `70e15e5`，含**行为中性**验证（60 回合状态流 sha256 改前=改后）。
4. `[ ]` **舰船模板**（`ship-blueprint.md`，含按舰级默认）。
5. 引擎侧**不加**通配/清叶动词（§1.1）。

## 6. 复现 / 验证

```bash
cargo test --workspace
cargo run --bin planet_x -- --seed 7 --round 6 --index out/          # 看 idx/{faction_process,city_process,control,scope,decisions}.jsonl
cargo run --bin planet_x -- --seed 7 --round 6 --index out/ --save ckpt.ron
cargo run --bin planet_x -- --start ckpt.ron --derived               # 与上面同一回合，值必须完全相同
# 行为中性（改捕获前后各跑一次，逐字节比对）：
cargo run --bin planet_x -- --seed 42 --round 240 --digest 20 | shasum -a 256
```

## 7. 落地记录（2026-10，`feature/control-tri-state`）

### 7.1 做了什么

* `idx/flow.jsonl`（回合 × 势力：`production` / `upkeep` / `governance_total` / `governance_coverage`）、
  `idx/city_flow.jsonl`（回合 × 城，含 razed 空城）、`idx/control.jsonl`（每个叶片一行：
  `kind` / `key` / `sub` / `value` / `mode`）、`idx/scope.jsonl`（显式作用域节点）。
  四张表由 `projection.rs` 的 `DERIVED` 声明驱动，schema 里有对应的 `derived` 段（测试断言
  声明／写出／schema 三处一致）。
* ships 表加四列：`order_leaf_mode` / `order_default_mode` / `order_effective_mode` / `order_effective`
  ——**引擎解析的答案**，Python 不该自己重实现链。
* `--derived`：打印这一回合存下来的 `{round, source, pre, post}`；有档就是**档里那一对**，
  没档（或叠加了 `--apply`）就按当前状态重算并附 `note`（否则空的过程量会被误读成"本回合零产出"）。
* 顺手修掉两个"静默不实"：
  * `--index --save` 存出来的 checkpoint 里 `pre`/`post` 都是**丢了流量**的重算值
    （`write_index` 现在返回 `IndexOutcome{pre, post}`），于是同一回合的两个读面会对不上；
  * `serde_json` 默认的**浮点解析不精确**（实测：文件写 `1.4000000000000001`，读回来是 `1.4`）——
    换成 `features = ["float_roundtrip"]`。这个坑不止影响测试：`--apply` 的 diff 里的数也走同一条解析。

### 7.2 修正一个我先前的说法（重要）

`RoundFlow` 的**数值**并不是"从未暴露"：`round_metrics` 已经把 flow 抄进了 `metrics`
（`FactionMetrics::{production, production_value, upkeep, governance_cost, governance_coverage}`、
`CityMetrics::{production, production_value, …}`），而 `metrics` **内联在每行 main.jsonl 里**。
所以本轮新表的真实收益是**形状**（可 join 的平铺行、按 `faction_id`/`city_id` 键、列类型稳定、
razed 城也在），**不是**"拿到了以前拿不到的数"。教训照旧：动手前先把"我以为没有"核实一遍。

### 7.3 `{}` vs `null` 的契约

投影表**永远给对象**（没产出就是 `{}`，不是 null），这样 Python 侧列类型稳定；`RoundView` 里没有
这个势力的键时是 `null`。集成测试当时用一层 `res_map()` 翻译这两者（语义相同，形状不同）——
⚠ **那个翻译层已经删了**：两个读面读同一份视图之后不再需要它，见
[`pre-post-unify.md`](pre-post-unify.md) §4。

### 7.4 `pre` 的真相（下一步的依据）

`RoundState.pre` 现在是 `view_from_state(推进前状态)`：**过程量恒为 0/空**、观测部分是推进前的
世界。它**不是**"本回合 rng 掷出的随机决策"——那个东西**任何地方都没有被结构化记录**（AI 造了什么舰、
舰队怎么重组、谁接战了，只散在事件与状态差里）。想要"AI 在想什么"的读面，得在
`sim`/`autocontrol` 的决策点补一次捕获（新的 `Decisions`），并且必须**行为中性**——
判据：同 seed/同回合的 `--digest` 输出与捕获前**逐字相同**，长局 harness 全绿。

→ **§7.5 就是这件事的落地记录。**

### 7.5 `decisions`：把「AI 的判定过程」也做成中间量（本轮做的）

**为什么必须是新捕获**：指令叶（`ship_orders`）只留下**结果**——「它现在在 `Follow` 某艘舰」；
事件（`ShipDestroyed`/`Withdraw`/`ShipSpawned`）只记**发生了什么**。中间那层「AI 为什么这么选」
（是接战索敌还是自保撤退？当时的血量比与撤退阈值差多少？这回合它是不是**根本没被派活**？）
全部发生在 `autocontrol` 内部，用完就丢。

**形状**（`src/model/decisions.rs` → 挂在 `RoundSink.decisions` → `RoundView.decisions` →
`idx/decisions.jsonl`）：

* `kind="ship_order"`：逐舰判定，`verdict` ∈ `withdraw` / `engage` / `colonize` / `bombard` /
  `move` / **`hold`**；`detail` 带判定**输入**——`hull_ratio` / `retreat_hull` / `kiting` /
  `enemy_in_range` / `after_move` / `destination` / `order`。
  * **`hold` 是空白里的信息**：这一回合 AI 没给这艘舰派活（叶上那条值可能是很久以前的）。
  * **一艘舰一回合最多两行**：先机动（`move`），移动到位后再判一次（`after_move: true`）——
    所以聚合前要看 `after_move`，否则会把「先去了哪、到了之后改了什么主意」算成两件事。
* `kind="retool"`：船坞改装（`actor`=城名、`target`=新舰级、`detail.from/building`）——这是
  **少数几个不留事件的 AI 决策之一**，此前只能从 `ship_type` 的变化反推、且不知何时改的。

**行为中性已实测**（同一次改动前后）：`--seed 42 --round 240 --digest 20` 的输出 sha256
**完全相同**；`--seed 7 --round 60 --index` 的 `main.jsonl` sha256 **完全相同**；
只有 checkpoint 字节变了——因为它多存了 `decisions` 这一栏（预期，且是有用的那一栏）。
实测规模：seed 7 跑 60 回合 → 1083 行（engage 444 / hold 464 / move 144 / bombard 14 /
retool 13 / colonize 3 / withdraw 1）。

**没做的两件事（记成候选，别当成做完了）**：

* `[ ]` **玩家名下的舰**：它们的指令是你下的（没有 AI 判定），但**自动接战/轰炸**（`auto_combat`）
  对所有舰一视同仁——那一层今天不记录，所以「AI 拿你的舰去开火」仍然只能从事件反推。
* `[ ]` **火力分配（`build_fire_plan` 的逐武器索敌）**：那是"每一发打谁"的判定，比行为判定更细。
  没做是因为它一行一武器会爆行数（一场大战几百发），值得先想清楚它的表形状。

## 8. 落地**之后**发现的缺口（同轮处理 / 记成候选）

写侧套件 `play/planet_x_ctl`（并行做的那一半）在真实使用中撞出六条，逐条记录处理方式——
**"加了表就完事"是这一节最想纠正的错觉**：

1. `[x]` **只写了表、没让消费者看见**（本轮修复）：四张派生表在 `schema.json` 的**新 `derived` 段**
   （不是 `lazy` 段——它们不是靠 main 的 id 数组索引，而是靠 `join_on` 指向 main 已有的列），
   而 `planet_xq.load()` 只读 `schema["lazy"]` ⇒ Python 侧**看不见**它们。
   修复：`planet_xq` 现在也读 `schema["derived"]`，并给 `q.derived(name, round)` +
   几个便利读法（当时叫 `q.flow()/q.city_flow()/q.control()/q.scope()`；两张过程量表现已改名
   `faction_process`/`city_process`，缺表时报出"旧版投影没有这一段"）。
   **教训**：契约是"发射端 + 消费者"两处，光在 Rust 侧加 schema 段不算完。
2. `[x]` **投影一份 checkpoint 时起点回合的流量是 0**（本轮修复）：`--start ckpt --round 0 --index`
   走的是 `view_from_state`（过程量恒空），于是 agent 看到"全世界零产出/零维护/零治理"——
   数字自洽、语义骗人。修复：`projection::write_index_seeded(..., start: Option<RoundView>)`，
   `--start` 时把档里存的 `post` 交给投影当**起点回合**的行（那一行的 state 就是那一回合的结果）；
   守卫：`tests/projection_derived.rs::projecting_a_checkpoint_keeps_that_rounds_flow`
   （含"至少一个势力维护费非零"的防空转断言）。全新开局仍然没有流量（初始世界没有"上一回合"）。
3. `[x]` **`--control` 的叶值被舍入到 2 位小数**（本轮修复）：`round_view` 为了 token 噪声把
   预算/权重四舍五入，而 checkpoint 存的是全精度——于是"dump → 改 → 回传"会**静默量化到 0.01**。
   **修法：直接删掉读面的舍入**（选它而不是"加 `--control-raw`"，理由是分界线本身：**引擎是数据
   平面，输出即真值；好不好看是策略平面的事**）。而且先量过代价：一份跑到 120 回合的真实控制面里
   **470 个数值没有一个会被 2 位舍入改变**——这点噪声在当前世界里根本不存在，舍入只带来风险、
   没带来收益。守卫：`src/control.rs::the_control_template_never_rounds_a_leaf_value`
   （三个"舍入会改变它"的值落在三种叶上：势力级默认风格 / 逐舰风格 / 资源预算）；
   端到端证据：写 `0.7131` → `--control` 读回 `0.7131`（旧行为是 `0.71`）。
   kit 侧同步收紧：`_values_match` 从"半格容差"改成**精确比较**（只留 1e-9 的浮点表示余量），
   因为读面已经无损——"落在容差里"不再能当"落地了"的证据。
4. `[x]` **风格轴曾经没有 `mode`** —— 已由 `70e15e5` 解掉（`ship_doctrine`/`ship_kiting` 现在
   都是三态叶片 + 写值即接管 + 势力级默认）。写侧套件是在那次提交**之前**做的，所以它的
   报告里把这条列为"做不到"；现在 kit 可以（也应该）要求显式 `mode`。
5. `[ ]` **ships 表没有 `spawned_round`**（候选）：编制表的确定性 tie-break（"旗舰 = hull_max
   最大的巡洋舰，同分取**最老的**"）因此表达不出来，kit 只能用名字序当代理（舰名带世代后缀，
   近似但对不齐）。要做得给 `Ship` 加一个出厂回合字段 + `SCHEMA_VERSION` 升档（旧档缺 ⇒ 未知）。
6. `[x]` **`--control` 需要 CWD 里有 `config/game.ron`，且二进制与 config 是 worktree 局部配对**
   （kit 侧的坑，已按其办法解决）：从 checkpoint 往上找 config 会找到**另一个 worktree 的**配置，
   于是报 `missing field 'sanction_trade_mult'`。kit 现在优先按**二进制自己**的位置推 CWD，
   checkpoint 次之。
7. `[x]` **「契约有两端」这条教训又犯了一次**（本轮修复）：引擎这轮给控制面加了两片**势力级
   默认风格**（`default_doctrine`/`default_kiting`）并让风格轴有了 `mode`，而 kit 的
   `LEAF_KINDS` 里没有这两个 kind —— **它不报错，只是静静地把这两片叶从 `surface()` 里藏起来**
   （读面有、Python 看不见；demo 的叶计数 237 → 修复后 255，差的 18 就是它们）。
   修复：`LEAF_KINDS`、`_VALUE_FIELD`、`_leaf_value`（两轴叶）、两个新 setter
   （`set_default_doctrine`/`set_default_kiting`），并让逐舰的 `set_kiting`/`set_doctrine`
   也**要求显式归属**（`mode=` 或 `take_over=True`）——引擎那边它们早就是三态叶了，
   kit 的文档还停在"engine gap"。
   教训（与 §8.1 同源，值得单独再记一次）：**引擎改了读面，必须顺手 grep 一遍消费者里
   写死的字段清单**；"没报错"在这里恰恰是最坏的症状。
   顺带撞出 `control-live-layers.md` §3.1 那个两轴叶的坑，kit 已按"配方期拒绝"处理。
