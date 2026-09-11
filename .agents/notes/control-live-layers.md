# 控制属性 = 活层（三态归属 + 层次化默认值）

> ⚠ **2026-10 用户后续裁决**：本文所述的「删叶 / 恢复出厂值」（`remove: true`、
> `NOTE_APPLY_REMOVED` 控制叶部分）已**整体删除**。出厂默认只是初始值，不是可恢复的目标；
> 控制叶没有 `remove`/`删叶`，旧键会在 API 边界被拒。本文相关段落只作历史记录。

> 状态 `[~]`（指令部分已实现；风格部分与按舰级默认未做） ｜ 索引：[notes.md](../notes.md) ｜
> 关联：`agent-control-long-game.md`（§5 新舰默认归 AI、§6 结构性叶片所有权、§7 幽灵权重）、
> `agent-control-api.md`、`agent-play.md` §3/§4（玩家手册）、`combat-behavior-doctrine.md`
> （doctrine/kiting 的出处）、`governance-loyalty.md`（迁都/忠诚的同形先例）、
> `python-control-authoring.md`（用 Python 统计地写这套控制面）、`ship-blueprint.md`
> （它的对偶：非控制属性 = 出厂快照）

## 0. 一句话

**控制属性是活层**：每一层只表态「谁负责 + 干什么」，缺省不说话（`Inherit`）就上溯到更宽
的那一层；因此「改主意」是改一个更宽的叶片，而点名单舰只剩**例外**一种用途。
它的对偶是 `ship-blueprint.md`：**非控制属性不在这一层**，它们是出厂时印死的快照。

## 1. 模型（三态 + 两轴 + 一条链）

### 1.1 归属：三态

```rust
pub enum ControlMode { Inherit, Auto, Player }   // #[default] Inherit
```

| 值 | 中文 | 含义 |
| --- | --- | --- |
| `Inherit` | 继承 | **这一层没有说话**。不是第三种决策者，只是把决定权让给更宽的一层 |
| `Auto` | 自动 | 系统（`autocontrol`）每回合决定并改写（原来叫 `Ai`） |
| `Player` | 玩家 | 玩家的指令，系统只读不改写 |

**缺省就是 `Inherit`**：`Control.mode`、`ControlScope` 各节点、补丁里省略的 `mode` 全是它。
「叶子不存在」≡「显式写 `Inherit`」；**全链都没有人说话 → `Auto`**。

### 1.2 两轴（这是最容易搞混的地方）

| 轴 | 谁在回答 | 各层 |
| --- | --- | --- |
| **归属**（谁负责） | `ControlMode` | 叶子 → （舰：舰队默认）→ 城市 → 天体 → 势力 scope → 全局 |
| **值**（干什么） | 各层自己的 `value` | 叶子值 / 舰队默认值 / （作用域节点**不带值**） |

作用域节点（`ControlScope`）**只表态归属、装不了值**——这是「只把势力设成 `Player`，新舰
归你了但原地 `Idle`」的根因（`agent-play.md:243` 那条警告）。

### 1.3 取值规则（活层的核心）

> **有效值 = 权威层（归属胜出的那一层）的值。**

* 叶自己有意见（`Player`/`Auto`）→ 取**叶值**（权威就是叶子；`Auto` 时叶值 = 系统写的
  **流水记录**）；
* 叶没有说话（`Inherit`/缺叶）→ 取势力级默认的值，**但只在它自己是 `Player` 时**。若舰队默认
  是 `Auto`，值应当由系统每回合现写，不能去用高层里那个可能早已过期的值；
* 都没有 → `None`（调用方按 `Idle` 兜底）。

> ⚠ **2026-10 修订**（`blueprint-stance.md`）：**指令**已经没有"更高的一层"了——舰队级
> `default_ship_order` 与图上的 `order` **两片叶都已删除**（用户裁决：指令是即时操作），
> 所以指令的有效值**就是那片逐舰叶里的值**，叶不存在 ⇒ `None`。
> 上面这三行现在只描述**长期倾向三轴**（风格 / 风筝姿态 / 角色），而且那三条链多了一层：
> `叶 → 出厂图 → 舰队默认 → 势力 scope → 记录值`。

**推论（必须守住）**：AI 每回合写回的 `Control::inherit(...)` 是**流水，不是指令**
（`autocontrol/tactics.rs:351/402/413/447`）。不认这一条，`scope=Player` 就会把整队冻结在
AI 最后写的行为上——那是「活层」最危险的坑。

## 2. 已实现（`[x]`，全部在 `feature/control-tri-state`，已提交）

* `[x]` **三态化 + 去掉 `Option`**：`model/control.rs`（`ControlMode`、`Control<T>{value,mode}`、
  `ControlScope{global,factions,bodies,cities}` 都是裸三态）、`resolve_chain`、
  `model/state.rs` 的 6 个 `*_control()`（调用点用 `ControlMode::is_player()`，不再 match 三态）。
* `[x]` **线格式与无损迁移**（`SCHEMA_VERSION` 4→5，见 `migrate()` 的 v4→v5 段）：
  `ControlMode` 手写 `Serialize`/`Deserialize`，线格式是「Option 外壳 + 名字载荷」——
  RON 写 `Some(Player)`、JSON 写 `"Player"`；读端同时认旧档的 `None` ≡ `Inherit`、
  `Some(Ai)` ≡ `Auto`。⚠ RON 的 `deserialize_any` **看不见裸标识符的名字**（实测退化成
  无载荷 unit），所以**不要**把线格式改回裸枚举，也**不要**用 `serialize_str` 写载荷
  （RON 里会写成带引号的字符串，派生枚举读不回来：`ExpectedIdentifier`）。
  守卫测试：`control::tests::legacy_mode_spellings_still_load`（含「自己写的必须读回来」）。
  实测：旧档 `play/exp2/ckpt_r12.ron` 加载后归属逐值一致。
* `[x]` **作用域树读面即写面**：`ScopeView` 并入 `ControlScopePatch`，读面只列**有意见**的
  节点 → 「dump → 改 → 回传」不会静默清掉没列出的层。
* `[x]` ~~**势力级舰队默认指令** `ControllableState::default_ship_order`~~ —— **2026-10 删除**
  （`blueprint-stance.md`）：实测它不是"默认值"而是**全舰队接管开关**（写它 ⇒ 全舰队归属变
  `Player` ⇒ 自动控制的 style/freight/contract 闸门全跳过、连自保撤退都不生效，而全舰队被钉死在
  同一条站桩指令上），而指令本身是**即时操作**、不该有"势力级默认"。
  舰队级只剩**长期倾向三片**（`default_doctrine` / `default_kiting` / `default_role`）；
  "新舰出厂是什么"改由**设计图的倾向三轴**回答。
* `[x]` **一次性指令不再脱手归属**：`sim.rs::reset_order_keep_mode`（殖民收尾只换值不换
  `mode`）；此前玩家点名的殖民舰建完城就被静默交还给系统。
* `[x]` **写值即接管**：只写值、不写 `mode` 的补丁 = `mode: Player`，覆盖
  `ship_orders` / `default_doctrine` / `default_kiting` / `default_role` / 两类预算 / 两类权重 /
  `loyalty_budget` / `capital`
  （`control.rs::write_value_leaf` + 各写点）。回执：`ApplyReport.took_over` +
  CLI stderr 的 `NOTE_APPLY_TOOKOVER`。理由：值写进去而归属仍解析成 `Auto`，系统下一回合
  就按自己的逻辑覆盖它，而 stdout/退出码一切正常 = 又一次「失败看起来像成功」。
  ⚠ 副作用：`agent-play.md`「省略 mode 保留当前模式」的旧说法对**值**不再成立（对
  「只写 mode」仍然成立）；显式写 `mode: Inherit` 是「撤回表态」，**不算**接管。
* `[x]` ~~**web**：势力「舰」分组新增「舰队默认指令」一行~~ —— 那一行**2026-10 已删**
  （`blueprint-stance.md`；舰队级只剩三条倾向默认）。原文如下（保留作历史）：编辑器按**有效归属**
  （叶 → 舰队默认 → 势力 → 全局，`app.js::effectiveMode`）开放；改行为/改值即把该叶钉成
  `Player`；不可编辑时给一行说明（`.tnode-hint`）。三态下拉是「继承/自动/玩家」。
  ⚠ **只跑了 crate 测试与 round-trip 守卫，没实机点过**（未按 `scripts/web.ps1` 起服务）。
  → 这一条的实机部分已在**本轮**补上（含两行默认风格 + 逐舰风格叶，见 §7）。
* `[x]` **风格（doctrine / kiting）也是活层**（提交 `70e15e5`，见 §6）：每舰叶片 +
  势力级 `default_doctrine`/`default_kiting` + `Ship` 字段降级为记录值 + AI 5 个读点改走有效值。
  **实测行为中性**（同 seed 的 60 回合完整状态流 sha256 改前=改后；240 回合 `--digest` 逐行相同）。

## 3. 剩余

* `[x]` **doctrine / kiting 也变成活层** —— 已落地（提交 `70e15e5`），实现见 §6。
* `[x]` **读面看不出「有效值」** —— 已落地（同一条提交）：不再是给 `--control` 加字段，
  而是**投影的表**给答案（`ships` 表 `order_leaf_mode`/`order_default_mode`/
  `order_effective_mode`/`order_effective`/`doctrine`/`kiting`）——符合 `engine-data-plane.md`
  的分工：**引擎给答案，Python 只负责筛**。`ShipDoctrineEntry`/`ShipKitingEntry` 也加了
  `mode`（值取有效值、mode 取叶片表态 ⇒ 模板原样回传安全）。
* `[ ]` **舰队默认的粒度**：现在只有「势力级单值」，表达不了「护卫舰守家、巡洋舰殖民」。
  用户已裁决：**按舰级 = 舰船模板的功能**（"还不存在的实体的规则"），并进
  `ship-blueprint.md`；控制面不另立一层。现有的替代做法：kit 侧按 `class` 筛现存舰展开成叶
  （新舰不跟随，这是"现存 vs 未来"那条分界线的必然结果）。
* `[ ]` **`agent-play.md` 要跟着改**：§3/§4.2 那句「注意这会让新造出来的舰默认 Idle」现在
  有了正解（舰队默认），要改写成「设舰队默认 = 新舰自动继承意图」；「省略 mode 保留当前
  模式」要补一句「但写值即接管」；另外要补 `--derived` 与四张派生表。
* `[x]` **web 侧还差两行** —— **已补上**（分支 `feature/web-fleet-defaults-style`，实现与实机证据见 §7）。
  顺带把「逐舰风格编辑」也补了（此前 web 里根本没有绑 `Ship.doctrine`/`Ship.kiting` 的 UI）。
* `[x]` **投影的 `control` tidy 表补上四条风格叶**（本轮补，引擎侧）：`src/projection.rs` 的
  `control` 表原先只发 `ship_order` / `default_ship_order` / 预算 / 权重 / 首都，现在也发
  `ship_doctrine` / `ship_kiting` / `default_doctrine` / `default_kiting`（`value` 列是 `any`：
  doctrine 是 `{temper, lone_wolf}` 对象、kiting 是数字），`kind` 的 schema 描述同步列全。
  **为什么值得单独记一笔**：漏掉它们的后果不是报错，而是 Python 侧**只能**从 `ships` 表的
  `doctrine`/`kiting`（有效值）看结果 —— 于是「这艘舰的风格是它自己钉的，还是跟着舰队默认走的」
  在表里查不出来（web 的 `effectiveMode()` 正是靠这个区分）。
  守卫：`projection::tests::control_table_holds_every_leaf`（四片叶的值与**各自的** mode 逐条断言）。
  端到端：写一片 `default_doctrine`/`default_kiting` + 逐舰两片 → 投影后 `idx/control.jsonl` 出现四行，
  值就是叶自己的值（`{"lone_wolf":0.1,"temper":-0.9}` / `0.5` / `{"lone_wolf":-0.4,"temper":0.3}` / `-0.6`）。
  ⚠ 顺带记一个反直觉的观察：**跑了 20 回合的基线局里这四片叶一片都不存在**（AI 不写风格叶），
  所以"表里没有"是正确的**缺席**、不是漏发 —— 也正因为如此，没有守卫时这个洞极难被发现。

### 3.1 一个被 kit 撞出来的语义坑：**单轴写「还不存在的两轴叶」**

**事实**（`src/control.rs:762-770`）：`default_doctrine` / `ship_doctrine` 是**一片叶装两条轴**
（`temper` + `lone_wolf`）。当这片叶**还不存在**时，引擎用
`Control::inherit(ShipDoctrine::default())` 把它建出来 —— 也就是**你没写的那条轴变成 `0.0`**，
而不是「保留舰上的记录值」。

**为什么它是坑**：`0.0` 是个**正常取值**（"理智/不独"），事后从读面完全看不出问题，而它已经
把全舰队的 `temper` 从出厂值（中国舰队实测 `0.71`）静默改成了 `0`。复现（kit 侧）：

```python
s = ctl.surface(ckpt)
s.set_default_doctrine("中国", lone_wolf=-0.4, take_over=True)   # 只写一条轴
# → 落地后 default_doctrine.value == {"temper": 0.0, "lone_wolf": -0.4}
```

**已做**：`planet_x_ctl` 的 `Surface._require_both_axes` 在**配方期**拒绝「单轴新建叶」
（叶已存在时仍然允许只改一条轴，那时"缺省 = 保留现值"是真的）。这条与
`agent-play-friction.md` 的「宁可在配方期报错」同源。

* ✅ **已裁决（2026-10，用户确认）**：**(ii) + (iv)，不动叶值形状**。**已实现**，见 §11。
  * **逐舰**（`ship_doctrine`）：叶不存在时，缺的那条轴**从这艘舰的记录值 `Ship.doctrine` 种上**
    —— 引擎手里就有这个数，而"只改一条轴"的直觉含义正是"另一条保持它现在的样子"。
    ⚠ **本轮实测修正**：代码里那条路种的是**这艘舰当时在用的那条轴**
    （`let base = state.ship_doctrine(ship)`）——没有舰队默认时**正是记录值**（这条逐字成立），
    舰队默认是玩家时是**默认值**（而界面显示的也正是它，所以这才是对的：只动一条轴不该让
    界面上另一个数凭空跳回出厂值）。真正会变 `0.0` 的只有**势力级**那条建叶分支
    （`Control::inherit(ShipDoctrine::default())`）——即下面那一条。两条路都补了测试。
  * **势力级**（`default_doctrine`）：没有"单一的现有值"可种（舰队里各舰记录值可能不同），
    所以**要求两条轴一起给**；只给一条 ⇒ **响亮拒绝**（`partial_doctrine_leaf`，错误信息里
    给出三条改法：两条轴一起给 / 先只写 `mode` / `remove` 删掉这片叶）。实现见 §11.2。
  * **不选 (iii)**（两轴各自 `Option` = 逐轴继承）：表达力最强，但要动叶值形状、读面、
    `--control` 模板、kit 的 `LEAF_KINDS`/`_VALUE_FIELD`、web 编辑器与 `SCHEMA_VERSION` 迁移，
    而且链要从"按**叶**解析 mode"变成"逐**轴**解析"——而它唯一多出来的能力（舰队级只钉一条轴）
    几乎只有逐舰叶才需要 ⇒ 收益/代价不成比例。真出现需求再回来拆。
  * **不升 `SCHEMA_VERSION`** ✅（叶值形状没动；`remove` 只是**写面**多了一个字段）。

### 3.2 ~~已查实：风格轴今天没有执行者~~ → **已解决**（`Auto` 在风格轴上有真执行者了）

**原文（2026-10 查实，保留为历史）**：写 `Ship.doctrine`/`Ship.kiting` 的生产代码只有两处
（`world.rs` 开局预置舰队、`sim.rs::spawn_ship` 出厂快照），写风格**叶**（`ship_doctrine`/
`ship_kiting`）的生产代码**一处都没有** ⇒ 把一片风格叶设成 `Auto` 的实际效果是**值冻结**，
web /`--control` 给 `Auto` 的措辞只能是假话。

**现状（本轮 `feature/ai-agency` 已落地，实现见 §16）**：`autocontrol::style` 是风格三轴的
真写入者——每回合按**战况**（战争强度 / 净战果 / 被打残程度 / 撤退占比；编队规模；敌我火力比
与自身硬度）重估逐舰 `ship_doctrine`（temper + lone_wolf）与 `ship_kiting`，**概率触发**、
**分布步长**（指数松弛），写回的是 `Control::inherit(...)`（流水，不是指令）。因此：

* 逐舰风格叶的 `Auto` **名副其实**（web 的措辞已改回「自动控制每回合按战况重估，会改写这片叶」）；
* **势力级默认叶的 `Auto` 仍然不是执行者**，但它的含义现在说得清了：默认叶只在**它自己是
  `Player`** 时供值，所以 `Auto` 的默认叶 = AI 的答案是「**不设全舰队默认、逐舰自己说**」
  ——它确实不需要写任何值。web 对这三片默认叶的措辞是「本层不供值：引擎只在它是玩家时才取默认值」
  （旧措辞「本轴暂无重估者：不给值」会让人以为将来会有人来写）。
* 设计图那一层（`Auto` 图的执行者）也已补齐，见 §17（同一轮）。

## 4. 裁决（✅ 用户已确认，且 §4.1/4.2/4.4 已实现）

* ✅ **§4.1 风格降级为记录值** —— 实现：`Ship.doctrine`/`Ship.kiting` 仍是字段（出厂快照 +
  AI 流水），有效值走 `State::ship_doctrine`/`ship_kiting`。
* ✅ **§4.2 风格两片**：`default_doctrine` + `default_kiting`（没有 `default_style` 包装）。
* ✅ **§4.3 舰级默认 = 舰船模板的功能**（并进 `ship-blueprint.md`，控制面不另立一层）。
* ✅ **§4.4 读面的 `effective`** —— 实现方式见 §3 第二条（走投影表 + 两个 entry 的 `mode`）。

## 5. 复现 / 验证

```bash
cargo test --lib                       # 91 通过（含 fleet_default_* / fleet_default_style_* / writing_a_value_without_mode_takes_over）
cargo test --test longhorizon          # 6 通过（8 ignored 是慢诊断）
cargo test --workspace                 # 再加 web crate 的 15 与 tests/projection_derived.rs 的 2
# 旧档无损迁移的实机检查（旧二进制写的 checkpoint 用新二进制读）：
cargo run --bin planet_x -- --start ../planet_x/play/exp2/ckpt_r12.ron --control
#   → 长城/赤霄/北斗 = "Player"、北辰 = "Inherit"（与旧档的 Some(Player)/None 逐值一致）
```

## 6. 风格活层的实现记录（提交 `70e15e5`）

* **数据面**：`ControllableState` 四片新叶（`ship_doctrine` / `ship_kiting` 每舰一片，
  `default_doctrine` / `default_kiting` 势力级各一片），全部 `#[serde(default)]`；
  `SCHEMA_VERSION` 5→6，`migrate()` 的 v5→v6 段写明**零信息损失**（旧档没有这四片 ⇒
  一路继承 ⇒ 兜底到舰上记录值 ⇒ 有效风格逐舰不变）。
* **取值**：`State::ship_doctrine(id) -> ShipDoctrine` / `ship_kiting(id) -> f64`（叶 →
  舰队默认 → **舰上记录值**）。与 `ship_behavior` 的规则**刻意同形**：叶 Inherit 且舰队默认
  是 `Player` ⇒ 取默认值；否则取叶上的值。归属链是 `ship_*_control()`（叶 → 默认 → 势力 → 全局）。
* **AI**：5 个读点全改走有效值（`temper`、两处 `kiting`、`lone_wolf`、撤退阈值）。
  实测**行为中性**：同 seed 的 60 回合完整状态流 sha256 改前=改后（`827be5b2…`），
  seed 42 的 240 回合 `--digest` 逐行相同。这条中性是可预期的——AI 从不写这两条轴，
  而"叶 Inherit / 没有叶"的舰一律兜底到记录值，改前改后取到的是同一个数。
* **写面**：`ship_doctrine` / `ship_kiting` 补丁改为写**叶片**（写值即接管 + 钳制 [-1,1]；
  缺省轴保留**当前有效值** ⇒ 只写一条轴不会把另一条清零）；新增 `default_doctrine` /
  `default_kiting` 补丁（同一条「写值即接管」规则）。
* **读面**：`ShipDoctrineEntry` / `ShipKitingEntry` 加 `mode`——**值取有效值、mode 取叶片表态**。
  这样「模板原样回传」安全（没改过的行写回去仍然没有意见，有效值原样落进一片 Inherit 的叶），
  而 web 的每舰风格编辑器不会因为"没有叶片"就消失。
  ⚠ 一个要知道的语义：**改值请同时把 `mode` 改成 `Player`/`Auto`**——只改值而留着 `Inherit`
  等于说"这一层没有意见"，除非舰队默认也是 `Player`，那个值不会被采用。
* **投影**：`ships` 表加 `doctrine` / `kiting` 两列（引擎解析后的有效风格），
  与 `order_*` 四列同一思路——**Python 不该自己重实现链**。
* **web**：`default_doctrine` / `default_kiting` 两行**还没加**（见 §3 剩余）→ 本轮补上，见 §7。
```

## 7. web 控制面板接上风格活层（本轮 `feature/web-fleet-defaults-style`）

只动前端（`web/static/app.js`、`web/static/style.css`）与 web crate 的测试；**引擎一行没动**。

* **势力级三行默认**：`舰` 分组的第一组子项是**三条长期倾向**——`舰队默认风格`
  （`default_doctrine`：理智↔热血 + 护航↔独狼）/ `舰队默认风筝姿态`（`default_kiting`：风筝↔贴脸）
  / `舰队默认角色`（`default_role`）。~~原来是四行，头一行是「舰队默认指令」~~ —— 那一行
  **2026-10 已删**（`blueprint-stance.md`：指令是即时操作，没有势力级默认）。三行同一套惯例：
  节点 kind 各一种（`fleetdoctrine`/`fleetkiting`/`fleetrole`）、同样的 `scope: 'leaf'` 三态下拉
  （继承/自动/玩家）、同样的「有效归属是玩家才给编辑器、否则一行 `.tnode-hint`」。
  读面里没有这两片叶（开局就是）时前端补一片 `Inherit` 的叶让行出现，与作用域「没列出的层 ≡
  继承」同义；回传等价于「这一层没有意见」（引擎声明的「模板原样回传安全」）。
* **摘要诚实**：势力级默认行的摘要只在**它自己是 `Player`** 时显示那几个数（`Auto` 显示
  「自动（值由系统写）」、`Inherit` 显示「未表态」）——引擎的取值规则就是「默认叶只在自身是
  Player 时供值」，把没表态的存储值显示成"当前风格"会骗人。
* **`effectiveMode()` 按轴选默认叶**：`DEFAULT_LEAF` 映射 `shipdoctrine → default_doctrine`、
  `shipkiting → default_kiting`、`shiprole → default_role`（对应引擎 `ship_doctrine_control` /
  `ship_kiting_control` / `ship_role_control`）。~~`shiporder → default_ship_order`~~ 那一项
  **2026-10 已删**（指令链上没有默认叶了）。
* **逐舰风格叶**：一条舰从「一片叶」变成**三叶容器**（tabs：指令 / 风格 / 风筝姿态）。三片叶的
  归属链各自独立，所以三态下拉跟着子叶走。风格两叶写的是**叶片**
  （`ship_doctrine`/`ship_kiting`），**不碰** `Ship.doctrine`/`Ship.kiting`——那是记录值，
  写它没有任何控制效果（本轮实机确认：改了叶之后 `ship.doctrine` 记录值原地不动，而读面的
  有效风格变了）。逐舰 leaf 的读面值 = **有效风格**（叶 → 舰队默认 → 记录值），所以
  「继承舰队默认」的舰也看得见它现在实际用的数。
* **值一改即接管**：三片风格轴（temper / lone_wolf / kiting）都钳到 [-1,1]，且 `Inherit` 时改值
  就把该叶钉成 `Player`（与 `--apply` 的「写值即接管」同一条规则）。
* **可自动化**：`.tnode` 上加了 `data-key`（如 `fleetdoc中国` / `doc长城`），行可直接选中。
* **验证**（`cargo test --workspace` 全绿：planet_x 96 / longhorizon 6+10ignored /
  projection_derived 3 / planet_x_web 15+2；新增的两条 web 测试见下）+ **实机**：
  `scripts/web.ps1` 起服务（3001，pid 8344，`GET /api/ping` 对得上）、浏览器里给中国设
  `舰队默认风格 = (理智 +0.50 / 独狼 -0.25, 玩家)`、`舰队默认风筝姿态 = -0.60 玩家`、
  长城 `风格 = 理智 -0.90 玩家`（逐舰例外）、北斗 `指令 = 移动(30,20) 玩家`，点「应用到服务器」
  后**刷新页面仍在**，且 `/api/state` 里落的是**叶片**：
  `control["中国"].default_doctrine = {value:{temper:0.5,lone_wolf:-0.25}, mode:"Player"}`、
  `ship_doctrine["长城"] = {value:{temper:-0.9}, mode:"Player"}`，而 `ships[长城].doctrine`
  记录值仍是 `{0,0}`；`ship_doctrine` 读面里 北斗/赤霄/镇岳 的 `mode` 是 `Inherit`、值 = 舰队默认
  （= 归属链按新轴生效）。
* **新增 web 测试**（`web/src/lib.rs`）：① `fleet_default_style_rows_round_trip_through_the_web_surface`
  ——「只写 mode」（前端第一步改归属）合法、再写值读面立刻回显、且叶 Inherit 的舰改用默认；
  ② `posting_the_read_surface_back_keeps_effective_style` —— 把 `/api/state` 的
  `control`/`scope` 两段**原样** POST 回去（前端「点应用」就是这么干的）之后，逐舰有效风格与
  有效归属**一字不变**（守门「读面原样回传安全」，含读面里每舰都有一行的风格叶）。
* **已知副作用（引擎已裁定为安全）**：因为前端回传整份读面，`ship_doctrine`/`ship_kiting`
  里那些「没有叶片」的行会被写成一片 `Inherit` 的叶（值 = 当时的有效值）。语义不变（`Inherit`
  = 这一层没有说话），但 `--control` 模板与 `state.control` 会比从前多出这些叶。
* **剩余 / 需要上层裁决**：
  * `projection.rs` 的 `control` tidy 表没有这四片风格叶（见 §3 剩余最后一条）——Python 侧
    看不到风格叶自己的值与表态。
  * 「改了值就自动钉成 `Player`」是前端跟着引擎的「写值即接管」做的；但**读面给的是有效值**，
    所以「把继承来的值原地再写一遍」也会把这片叶从 `Inherit` 钉成 `Player`（原本跟随舰队默认，
    改完后变成这艘舰自己的特例）。UI 上这是"我碰过这一格"的后果，目前没有提示也没有「恢复继承」
    按钮（把三态下拉调回「继承」也能达到同样效果，但理解成本高）。
  * 舰节点从一片叶变成三叶容器是**界面结构改动**（指令编辑器从舰这一行移到「指令」tab）：
    §1 那条「一条舰 = 一片叶」的直觉需要跟着更新；如果更希望逐舰风格不占界面，改成挂在
    另一个分组也只是 `app.js` 的事。

## 8. 控制面板（web）的裁决（✅ 用户已确认 2026-10）

对应 §7 末尾「剩余 / 需要上层裁决」那三条，逐条定案（**都是 `app.js` 层的改动**）：

1. **逐舰风格用三叶容器（tabs）—— 保留**。理由：三片叶在引擎里本来就是三片独立的叶、各有
   自己的 mode，塞回"一个舰行一个归属下拉"会**制造假象**（一个下拉暗示一种归属）。
   **追加一条**：舰这一行保留一个**便利下拉**「这三片叶一起归谁」= 一次写三条**只带 mode** 的叶
   （引擎支持，且永不接管），批量效率与诚实兼顾。
2. **「碰过就钉 `Player`」要修**，两步：
   * **显式「恢复继承」**（把 mode 写回 `Inherit`、值不动）——**主手段**；
   * **等值不接管**：前端要写回的值若**等于当时的有效值**，就保持原 mode——辅助手段，规则可解释
     （"数值没变 ⇒ 不算表态"）。二者都不引入隐式魔法。
   * 那一格还要**显示"当前跟随：舰队默认 / 出厂值"**，把心智模型补齐。
3. **「应用」只回传差异**：前端手里就有载入时的读面快照，diff 出用户真正改过的叶再 POST。
   理由不只是足迹：回传整份会把「没有叶 ⇒ 兜底到记录值」变成「叶钉住这个数」，将来记录值语义
   一变（例如蓝图接管出厂风格），这些舰**不会跟随**——而现象是"改图/改配置对这艘舰没用"，
   极难归因。改完「应用」还应是**幂等**的。
4. **措辞保留「舰队默认风筝姿态」**（`kiting ∈ [-1,1]` 是姿态不是距离）。
5. **`Auto` 的标签要诚实**（依据 §3.2）：风格轴上不能写「值由系统写」。

## 9. 下一步顺序（✅ 用户已确认）

| # | 做什么 | 验收判据 |
| --- | --- | --- |
| 1 | ~~**web 三条**（§8 的 1/2/3/5；纯前端）~~ → `[x]` **已完成**（`feature/web-control-panel-ux`，提交 `f673bac`，见 §10） | ✅ 全测绿 + 实机点通：改一格→应用→刷新仍在；**把值改回原数不钉 `Player`**；只回传差异 ⇒ 别的势力的叶一个都没多 |
| 2 | ~~**两轴叶**（§3.1 的裁决；引擎十来行 + 守卫）~~ → `[x]` **已完成**（`feature/leaf-existence`，与下面的删叶一起做，见 §11.2） | ✅ 守卫齐了：势力级单轴新建 ⇒ `partial_doctrine_leaf`、叶已存在 ⇒ 允许、逐舰单轴 ⇒ 种"当时在用的那条"（无默认时正是记录值）；行为中性（同 seed `--digest` 逐字不变） |
| 2b | **删叶（方案 A，§10.4）** → `[x]` **已完成**（同一分支，见 §11.1） | ✅ `remove: true` 十条叶全支持 + `NOTE_APPLY_REMOVED` 回执 + kit 的 `remove_*` + web 的「恢复出厂值」；实机删叶后逐舰风格真的回到出厂快照 |
| 3 | **蓝图**（`ship-blueprint-spec.md` §8.0 的全部裁决 + 附 A 改动地图），**连同 `spawned_round` 一次升 `SCHEMA_VERSION`**（⚠ 现在是 **7 → 8**：`feature/freight-collection` 已把 v7 用掉了，见 §10.5） | spec §7 的测试计划全绿 + 旧档迁移**零信息损失**说明 + 老开局逐字节中性（`--digest` 对照）+ 长局 harness |

> 顺序的理由：1 是纯前端且当天可验（还顺手把 §3.2 的假话改掉）；2 是十来行但**现在就能从 web 踩到**
> （静默改数）；3 要升档 + 迁移，值得等 1/2 落地、读面稳定之后再动。
>
> ⚠ ① 落地后**没有**关掉 §3.1 那个坑：势力级两轴叶「单轴新建 ⇒ 另一条轴变 0」当时仍然在引擎里
> （① 只是让界面不去踩它：壳被碰过就整片发）。**② 已把它关掉**（引擎响亮拒绝），
> 且 ① 的补丁形状（完整的两轴新建）仍然合法。

## 10. 本轮：控制面板三条落地（`feature/web-control-panel-ux`，提交 `f673bac`）

只动 `web/static/app.js` / `web/static/style.css` + web crate 的测试；**引擎一行没动**
（同 seed `--digest` 逐字节不变，见 §10.3）。

### 10.1 交付物（逐条对应 §8 的裁决）

| §8 | 裁决 | 落地 |
| --- | --- | --- |
| 1 | 三叶 tabs **保留** + 舰行加便利下拉 | tabs 没动；`ship` 节点多了 `bulkOwnership`：一个下拉一次写**三条只带 mode** 的叶（`{ship, mode}` × 指令/风格/风筝），**永不写值**。三片表态不一致时多一个「三片叶：各自不同」占位项（`selTab` 的 0 值 = 不写）——不假装它们一样 |
| 2 | 「碰过就钉 `Player`」要修 | ① **「恢复继承」按钮**（`data-role="restore"`，只在叶自己有表态时出现）：`mode → Inherit`、值一个字节不动；② **等值不接管**：`wroteValue()` 对比「载入时那个数」，值没变就把**界面自己钉的** `Player` 撤回原来的表态（下拉里显式选过的模式不会被值的变化推翻）；③ 每行多一行 `当前跟随：…`（见下） |
| 2 | 显示「当前跟随」 | `styleFollowHint()`：叶自己没表态时给出**这个数从哪来**——`舰队默认（…）` / `出厂快照（…）` / `本舰叶片里的数（没表态 ≠ 没值）`。第三支是 §10.4 那个发现的可视化 |
| 3 | 「应用」只回传差异 | `buildCommandDiff()`（见 §10.2）；状态行直说「已应用 N 片叶的改动」/「没有改动要应用（只回传差异）」 |
| 4 | 措辞保留 | 「舰队默认风筝姿态」没改（`kiting ∈ [-1,1]` 是姿态不是距离） |
| 5 | `Auto` 的标签要诚实 | 风格轴：摘要 `自动（本轴暂无重估者：值冻结 / 不给值）`，下拉里 `option[value=Auto]` 的 `title` 直说"风格轴目前没有 AI 执行者：选『自动』不会有人来重估这个值"；指令/预算轴照旧「值由系统写」（那里真的有执行者） |

### 10.2 「只回传差异」的实现（一张显式的叶形状表 + 「壳」）

* `LEAF_SPEC`：每片叶的**身份键**（`ship`/`resource`/`city`+`building`）与**值字段**
  （`behavior`/`temper`+`lone_wolf`/`kiting`/`value`）。不能靠 `for (k in leaf)` 猜：
  读面条目还带着 `kind`/`structure`/`ship_type` 这类**实体属性**（属于 state，不属于控制面）。
* `pairOrigins()`：`buildEdits()` 里把编辑面每一片叶与**读面**（`baseControl`/`baseScope`）
  里同一片配成对，原值记进 `leafOrigin`；`edScope` 走 `buildScopeDiff()`（只报变过的键）。
* `diffLeaf()`：只报「现在 vs 原值」不同的字段。于是
  「改一格风格」发出去的是 `{"ship":"长城","temper":0.7,"mode":"Player"}`——**没有 `lone_wolf`**，
  缺省的那条轴由引擎保留现值（实测：那一轴取到了刚写进舰队默认的 -0.40，而不是 0）。
* **「壳」**：读面里没有那片叶时界面仍要显示一行（势力级三条默认、新下的舰），
  `shellLeaf()` 把壳的模板登记成它的原值。壳的两条规则：
  1. **没被动过 ⇒ 不进 diff**（否则"打开面板再点应用"就会给全势力造出叶）；
  2. **被动过 ⇒ 整片发**（两条轴都给）。第 2 条是 §3.1 的接口：势力级两轴叶单轴新建会被
     引擎拒，而壳里显示的数就是发出去的数（实测：`舰队默认风格 → 玩家` 发出的是
     `{"temper":0,"lone_wolf":0,"mode":"Player"}` 不是 `{"mode":"Player"}`）。
* 「恢复继承」/等值不接管都只改 `mode`/不改值，所以它们与第 3 条的 diff 天然相容
  （实测：撤销后那条 entry 从 diff 里消失得干干净净）。

### 10.3 验证

* `cargo test --workspace` 全绿：`planet_x` lib 100 通过 +1 ignored、`longhorizon` 6+10 ignored、
  `projection_derived` 4、`planet_x_web` lib **16**（新增
  `minimal_leaf_diffs_touch_only_what_changed`：空 diff = no-op、逐轴提交只动那一条轴与那一片叶、
  「恢复继承」只撤表态不动值）+ 2 条集成。
* **行为中性**：`--seed 42 --round 240 --digest 20` 的 sha256 = `70D5A34E…D901`，
  与 `main`（`4117214`）上同一条命令**逐字节相同**（19 行）。引擎没动，这是应然——但按约定实测。
* **实机**（`scripts/web.ps1`，3001，pid 35212，构建于 1s 前；`PLANET_X_WEB_CLOSE_EXIT=0` 便于 headless）：
  1. 打开面板选「中国」→ 只把 `长城 风格` 的归属改成玩家 ⇒ 发出的补丁只有
     `{"ship":"长城","mode":"Player"}`（**只写 mode 合法**，空 diff 时为 `{control: []}`）。
  2. 在**继承舰队默认**的叶上把值敲成与显示相同的 `0.00` ⇒ 补丁**不长出**这条 entry，
     且叶的 mode 仍是 `Inherit`（= §8 第 2 条的"把值改回原数不钉 Player"）。
     敲成 `0.7` ⇒ entry 出现且带着 `mode:"Player"`，归属下拉**就地**翻到「玩家」；
     再敲回 `0.00` ⇒ entry 消失、归属翻回「继承」、`恢复继承` 按钮消失。
  3. 一次「应用」（真按钮点击）：势力级默认风格 `{temper:0, lone_wolf:-0.4, mode:Player}` +
     `长城 temper 0.7` + `赤霄` 三片叶批量归属 ⇒ 状态行「已应用 5 片叶的改动」；
     `GET /api/state` 里 `world.control` 的**其他 8 个势力与 scope 逐字节未变**
     （`othersUntouched: true`、`scopeUntouched: true`），这就是"只回传差异 ⇒ 不新增别人家的叶"。
     叶落地的样子：`ship_doctrine["长城"] = {mode:Player, value:{temper:0.7, lone_wolf:-0.4}}`
     （缺省轴 = 当前有效值，**不是 0**）、`ship_doctrine["赤霄"] = {mode:Player, value:{0,-0.4}}`
     （批量只写 mode，值取的是当时有效值）、而 `Ship.doctrine` 记录值**仍是 {0,0}**。
  4. 刷新页面 ⇒ 全部仍在；`buildCommandDiff()` 回到 `{control: []}`（**幂等**）。
  5. 行上的 `当前跟随`：`北斗`（无叶）= `出场快照（… +0.00）`；`赤霄`（叶在但没表态）
     = 舰队默认 / 之后变成 `本舰叶片里的数（没表态 ≠ 没值）`——见 §10.4。

### 10.4 ⚠ 新查实：「恢复继承」**撤不掉叶里的值**（今天没有任何接口能删一片叶）

**事实**（`src/model/state.rs:291` / `:311`）：逐舰风格取值的最后一步是
`leaf.map(|l| l.value).unwrap_or(record)`——**只要叶存在就用叶里的值，与叶的 `mode` 无关**；
对照组是势力级默认叶：它有一道 `if d.mode.is_player()` 才供值。也就是说「叶 Inherit」与
「根本没有叶」在**归属**上等价（链上没人说话），在**取值**上**不等价**。

实机演示（同一个势力、两艘舰、两片叶的 mode 都是 `Inherit`）：

| 舰 | 叶 | 有效风格 |
| --- | --- | --- |
| 赤霄 | `{mode:Inherit, value:{temper:0, lone_wolf:-0.4}}` | **-0.40**（叶里的数） |
| 北斗 | 没有叶 | **+0.00**（出厂快照） |

**后果**：
* 「恢复继承」的实际效果 = 只交还**归属**，不交还**数值**。一旦某艘舰的风格叶被写过值
  （包括"只写 mode"时引擎顺手建出来的那一片），它就不再跟随出厂快照——除非舰队默认自己
  变成 `Player`（那会用默认的值盖住它）。§8 第 2 条裁决里「值不动」是我当时写的，这条规则
  在**指令轴**上是自洽的（默认叶 Player 才供值），到**逐舰风格轴**上就变成"钉住"。
* 这也正是 §8 第 3 条（只回传差异）为什么重要：以前那次"整体应用"会给**全舰队每艘舰**
  都造出这样的叶，而现场只表现为"舰队默认改了但船不跟"。
* 附带发现：`src/control.rs:33-36` 的读面文档写着「只改值而留着 `Inherit` ……那个值**不会**被
  采用」——与上面那段**代码相反**（那条描述对势力级默认叶成立，对逐舰叶不成立）。文档与代码
  必须先对齐一个，否则下一个读这段的人还会踩。

**候选（✅ 用户已选 A，本轮已实现 —— 见 §11）**：

| | 做什么 | 代价 |
| --- | --- | --- |
| **A** ✅ | 补丁加**删叶**（`{"ship":"X","remove":true}`）：`恢复出厂值` 才真正做得到；kit 的 `Surface` 与 web 各加一个动作 | 引擎 + kit + web 三处；要给 `remove` 定语义（舰队级/逐舰、与 `mode` 并存时谁优先）→ 三条规则见 §11.1 |
| **B** | 把逐舰叶的取值对齐文档（叶 `Inherit` ⇒ 一律走默认/记录值，只有叶有表态时才用叶值） | 会推翻 §7 的"逐舰叶读面给有效值 + 只改一条轴保留现值"那套；`--digest` 从"行为中性"变成"要重标"；但"没表态就没用"与「写值即接管」更自洽 |
| **C** | 什么都不做：把两行 hint 说清楚（§10 已做）+ 把"删叶"留在话题里 | 「我碰过这格、现在想还给它」只能靠舰队默认或重新开局 |

> 用户选了 **A**。B 仍留在桌上：`remove` 让"没表态就没用"这件事有了出口，但**取值规则**本身
> （叶存在就用叶值）没变——`src/control.rs` 那段读面文档（§10.4 附带发现）本轮改成了实话，
> 代码没动。

### 10.5 顺带修的小东西 / 记下的边角

* **布局 bug**（实机撞到）：底部读面（全宽）盖住左侧面板的「应用到服务器」，两条 bar 同时
  展开时按钮**点不动**（Playwright 直接报 "subtree intercepts pointer events"）→ `#side` 提到
  `z-index: 16`（读面是只读的，让它让位）。
* `styleField` 的输入框加 `data-axis`、三个下拉加 `data-role`：与 `data-key` 同一条思路
  （人和脚本都能一步点到那一格，实机验证就是靠它）。
* `wroteValue()` + `syncOwnership()`：值一改就把**那一行**的归属下拉与「恢复继承」就地跟上去。
  不重画整棵子树——那会换掉正在编辑的输入框（沿用既有取舍），但"我敲了数、归属还显示继承"
  是会骗人的。
* **`SCHEMA_VERSION` 已经是 7**（`feature/freight-collection` 的产地货栈用掉了 v7，`migrate`
  的臂是 `0..=6`）→ §9 第 3 步的蓝图升档是 **7 → 8**，`ship-blueprint-spec.md` 里的版本号与
  行号引用本轮一并修正了。
* **一个读面缺口**（不是本轮引入的）：`ship_orders` 的读面**只列有叶的舰**（`control_view` 遍历
  `c.ship_orders`，而逐舰风格那两条遍历的是 `state.ships`），所以**从没被点名过的舰在控制树里
  根本不出现**——它悄悄地跟着那一层默认走，而你没法在界面上给它单独设归属。要补就是让
  `ship_orders` 也"每舰一行"（与风格两叶同形），但那会改变 `--control` 模板的形状 ⇒ 留给 ③
  （蓝图那一步本来就要把 `order_source` 摆到读面上）。
* **一个与本题无关的编译警告**（`main` 上就有，来自 MOND 概率化那次合并）：
  `src/sim.rs:2371 pub(crate) fn mond_arrival_chance` 从未被使用 ⇒ `cargo build` 一直有一条
  `dead_code`。没在本分支动它（不想在纯前端分支里改引擎），但它该被清掉。

## 11. 本轮：两轴叶（②）+ 删叶（方案 A）（`feature/leaf-existence`）

引擎（`src/control.rs`、`src/main.rs`、`src/agent.rs` 一处夹具）+ kit（`play/planet_x_ctl`）
+ web 前端（`web/static/app.js` / `style.css`、`web/src/lib.rs` 的测试）。
**不升 `SCHEMA_VERSION`**（叶值形状没动，`remove` 只是写面多一个字段）；同 seed `--digest`
逐字节不变（见 §11.4）。

### 11.1 删叶的三条规则（A 的实现）

`remove: bool` 加在**十个控制叶补丁**上：`DefaultShipOrder` / `DefaultDoctrine` / `DefaultKiting`、
`ShipOrderPatch` / `ShipDoctrinePatch` / `ShipKitingPatch`、`BudgetPatch`、`InvestWeightPatch` /
`BuildWeightPatch`、`LoyaltyBudgetPatch`、`CapitalPatch`（`BuildingPatch` 早就有同名的 `remove`，
但那是**结构性**的——删一座建筑；语义不同、名字刻意相同）。

1. 删的是**控制面里那片叶**，**不要求实体还在**（舰战沉 / 城易主 / 建筑没了 / 资源 key 已删
   都能删）⇒ 顺带是清理陈叶的路；
2. 叶本来就不存在 ⇒ **幂等成功**（`applied += 1`、**不进** `removed`、不报 `skip`）——
   "目标状态达成了"与"改一个值"是两种成功；
3. `remove` 与任何值 / `mode` 字段同时出现 ⇒ **拒绝**（新码 `remove_conflicts_with_value`）：
   一条同时说着"删掉它"和"把它设成 0.5"的补丁没有正确答案，静默优先级 = 又一次
   "失败看起来像成功"。

回执：`ApplyReport.removed`（只记**真的**删掉的）+ CLI stderr `NOTE_APPLY_REMOVED`（与
`NOTE_APPLY_TOOKOVER` 对称）；web 的 `POST /api/command` 照旧忽略。

实现上把每片叶的"删 / 写"抽成了十个 `apply_*` 助手，`apply_diff` 只剩调用。理由不是行数：
`remove` 的冲突检查与幂等语义必须在**每一片**叶上完全一致，而原先那十个内联块已经开始互相漂移。

> ⚠ **一个被 kit 的 demo 当场抓出来的线格式坑**：读面复用 `DefaultShipOrder` / `DefaultDoctrine` /
> `DefaultKiting` 来**回显**叶片，于是 `remove` 会以 `"remove": false` 出现在 `--control` 里；
> 而 kit 的 `verify` 是按字段比对读面的 ⇒ 每一步都会多出一列"假变动"。
> 修法：`#[serde(default, skip_serializing_if = "is_false")]`——只在真的要删叶时才出现在线格式里。
> 又一次"契约有两端"：**emitter 一改，consumer 立刻报出来**（这次是好事）。

### 11.2 两轴叶（② 的实现）

* **势力级** `default_doctrine`：叶**不存在** + 只给一条轴 ⇒ `partial_doctrine_leaf` 拒绝，
  理由里给三条改法（两条轴一起给 / 先只写 `mode` / `remove` 删掉这片叶）；叶已存在时单轴写
  照旧合法（缺省轴保留现值）。
* **逐舰**：代码本来就是对的（缺省轴取"当时在用的那条"，见 §3.1 的实测修正），本轮**补测试**把
  两种情形钉住：没有舰队默认 ⇒ 种**出厂记录值**；舰队默认是玩家 ⇒ 种**默认值**（界面上显示的
  那个数）。**绝不是一个凭空来的 `0.0`。**
* kit 的 `_require_both_axes` 保留（配方期报错比 apply 期报错早），文档改成"引擎也会拒绝"。

### 11.3 三端接口（A 落地后）

| 端 | 动作 |
| --- | --- |
| 引擎 | 写面 `remove: true`；拒绝码 `partial_doctrine_leaf` / `remove_conflicts_with_value`；回执 `removed` → `NOTE_APPLY_REMOVED` |
| kit | `Surface.remove(faction, kind, key)` + `remove_doctrine` / `remove_kiting` / `remove_default_doctrine` / `remove_default_kiting` / `remove_default_ship_order`；`Report.removed` / `removed_leafs`；`_leaf_fields` 给**势力级**单片叶多一个 `exists` 字段（读面 `null` 就是"没有这片叶"）；`verify` 把删叶请求按**回执**判落地（读面永远没有 `remove` 字段） |
| web | 行上第二个动作「**恢复出厂值**」（与「恢复继承」并存，各自写在按钮上）：标记 → 发 `{"…","remove":true}`；只在 `state.control` 里**真的**有这片叶时出现；标记后隐藏编辑器 + 一行说明；再点一次取消。§10.4 那个"叶在、mode 是 Inherit"的状态下它照样在——那正是它存在的理由 |

### 11.4 验证

* `cargo test --workspace` 全绿：`planet_x` lib **104**+1ignored（新增 4 条：删叶回源、
  删叶的三个边角（势力级 / 舰已不在的陈叶 / 预算与迁都）、两轴叶新建守卫、逐舰单轴种子）、
  `longhorizon` 6+10ignored、`projection_derived` 4、`planet_x_web` **17**（新增删叶往返）。
* **行为中性**：`--seed 42 --round 240 --digest 20` = `70D5A34E…D901`，与 `main`（`d452481`）
  上同一条命令**逐字节相同**。应然：`remove` 不请求就不发生，② 那两条路只接管以前会**静默改数**
  的补丁。
* **kit**：`demo.py` 全部断言通过（并顺手修了它对 C 的断言：现在多一条 `exists` 翻转，
  那是**真的**"这片叶从无到有"）；另跑了一次端到端脚本（`scratch/kit_remove_check.py`，未入库）：
  写叶 → 删叶（回执里出现它）→ 读面回到出厂 `{0,0}` → **幂等**再删 → 单轴新建被 kit 与引擎
  **各拒一次** → 删势力级默认叶，全部通过。
* **实机 web**（`scripts/web.ps1`，3001，pid 47888）：舰队默认风格设玩家 + 长城风格 `0.7` → 应用；
  点「恢复出厂值」发出的正是 `{"ship_doctrine":[{"ship":"长城","remove":true}]}` → 应用后
  `state.control.中国.ship_doctrine == {}`、那一行改口成「当前跟随：舰队默认（… -0.40）」；
  再把舰队默认也删掉 → `default_doctrine: null`、那一行改口成「当前跟随：出厂快照（+0.00）」；
  全程另外 8 个势力**一个字节没动**（`world.control` 别的势力段逐字节相同）。

### 11.5 顺带

* 修了一个**偶发失败**的测试（`web/src/lib.rs::bind_auto_keeps_the_base_port_when_it_is_free`）：
  它"问内核要一个空闲端口 → 放手 → 重绑"，而两步之间那个端口会被并行测试或一次出向连接抢走
  （实测 `cargo test --workspace` 里偶发，单独跑 5/5 通过）。改成重试 8 次，失败信息里说清
  「返回了**别的**端口」与「实现没用起始端口」的区别。
* `src/control.rs` 那段读面文档（§10.4 的附带发现）改成实话：**逐舰**叶"存在就用叶里的值"，
  与 `mode` 无关；只有**势力级**默认叶才是"自身 `Player` 才供值"。
  （方案 B 没做：取值规则本身没变。）

### 11.6 下一步

③ **蓝图**（`ship-blueprint-spec.md` §8.0 十条裁决 + 附 A 改动地图），连同 `spawned_round`
一次升 `SCHEMA_VERSION`（**9 → 10**，见下条）。⚠ 蓝图那一步要注意：`order_source` 必须把
「叶不存在」与「叶写着 `Inherit`」分开报——本轮已经证明这两者在**取值**上不等价。

## 12. 本轮：**角色轴（第三条风格轴）补齐 + remove**（`feature/role-axis-parity`）

这一轮的起点是一次**合并**，不是新功能：`main`（`a1c6359`）把 `feature/freight-collection`
（运输：货舱 + Haul + 按积压抽签派单，schema v9）合到了 §11 之上，而两条特性**此前从没碰过面**
——freight 给 `control.rs` 加了**第三条风格轴**（角色：运输舰↔战舰，`default_freighter` +
`ship_freighter`），那 113 行是纯新增、与 §11 的 `remove` 无冲突，但它**没有经过 ②/A 那套规矩**。

### 12.1 合并暴露的缺口（为什么必须补）

1. **新轴没有 `remove`**：11 片控制叶都能删，唯独这条轴不能 ⇒ 直接违反已裁决的
   「所有控制叶都可删」。而且它那两处写入**内联在 `apply_diff` 里**，没走抽出来的 `apply_*`
   助手（抽助手的原因正是"十条规则必须逐片一致"）。
2. **web 面板不认识它**：`LEAF_SPEC` / `KIND` / `RAW_LEAF` / `DEFAULT_LEAF` 只有指令 + 风格两轴
   ⇒ 玩家**既看不见 AI 每回合写下的定编结论，也钉不住**（`Player`）这艘舰。
3. **kit 也不认识它**：`LEAF_KINDS` / `_VALUE_FIELD` 里没有这两片叶 ⇒ 读面有、`surface()` 看不见
   （`engine-data-plane.md` §8.1 那条老教训：契约是发射端 **+** 消费端两处）。

### 12.2 轴间不对称（本轮查实，写进三端文档）

| | 逐舰风格两轴（doctrine/kiting） | 角色轴（freighter） |
| --- | --- | --- |
| 谁写这片叶 | **只有玩家/agent** | **自动控制每回合也写**（`autocontrol::freight` 按积压定编，写 `Control::inherit(role)`） |
| `Auto` 的含义 | 空头承诺：没人会来重估，值冻着 | **名副其实**：AI 会重估并改写（`Player` 才是闸门） |
| **删叶**的含义 | 回到出厂快照，**从此冻结** | **放手**：交回自动定编（AI 下回合可能立刻又写下结论） |
| 势力级默认叶 | 玩家 `Player` 时才供值（§3.1 修正后的规则） | 同规则（`State::ship_freighter` 实测一致）；⚠ 但 AI **不写**这片默认叶，所以它的 `Auto` 仍属空头承诺 |

值规则**没有**新东西：`leaf.map(|l| l.value).unwrap_or(record)`，舰队默认只在自身 `Player` 时压过
叶片值（`ship_style_chain` 的 `StyleAxis::Freighter` 臂）——我核对过，与另两轴逐字同形。

### 12.3 三端接口（补齐后）

| 端 | 动作 |
| --- | --- |
| 引擎 | `DefaultFreighter.remove` / `ShipFreighterPatch.remove`（`skip_serializing_if = "is_false"`，读面不带）；新助手 `apply_default_freighter` / `apply_ship_freighter`（**删叶 → 实体校验 → 写**，与另十条一致：删叶不要求舰还在）；`apply_diff` 里那两段内联代码删掉 |
| kit | `LEAF_KINDS` + `_VALUE_FIELD` 收下两片叶；`set_freighter` / `set_default_freighter`（值必须 `bool`，写值必须明说归属）/ `remove_freighter` / `remove_default_freighter`；`_check_bool` 把 `1`/`0` 挡在配方期 |
| web | 逐舰第 4 片叶「角色」（下拉：运输舰／战舰）+ 势力级「舰队默认角色」一行；`RAW_LEAF` / `DEFAULT_LEAF` / `LEAF_SPEC` / `LEAF_OPTIONS` 全部登记；「恢复出厂值」自动跟着出现；舰行的批量下拉从"三片叶"改成按实际片数说话 |

### 12.4 验证

* `cargo test --workspace` 全绿：`planet_x` lib **118**+1ignored（新增 2 条：角色轴的删叶三规则、
  删叶=交回自动定编）、`longhorizon` 6+10ignored、`projection_derived` 4、`planet_x_web` **18**（新增
  `the_role_axis_round_trips_through_the_web_surface`：读面一行/写值接管/删叶回源/势力级默认叶）。
* **行为中性**：合并后的新基线 `--seed 42 --round 240 --digest 20` =
  `293725C43A0E26DC516977C04A5BD9C99977252B8C08D683EC2EA2749ADEDBC4`（12 行；取法：只取 `^\{` 行、
  `\n` 连接、UTF-8 无 BOM），本轮改完**逐字节相同**，且连跑两次相同（确定性）。
  旧基线 `70D5A34E…` 随 v9 运输落地作废——那是**行为的**改变，不是噪声。
* **kit**：`demo.py` 新增「[4b] 角色轴」一节（10 条断言）后**全部断言通过**：写值即接管 →
  真实 `--apply --save` → 删叶（回执里出现它）→ 幂等再删 → 势力级默认叶建成 → 删它在回执里
  （`exists` 由 True 翻回 False）；另加一条配方期拒绝（喂 `1` 当角色）。`README` 补
  「角色轴删叶 = 放手」与 API 两行。
* **实机 web**（`scripts/web.ps1`，3001，pid 19620 / 截图 `scratch/role-axis-panel.png`）：
  长城「角色」行 → 改归属为玩家 → 下拉选「运输舰」→ 发出的载荷是
  `{"ship_freighter":[{"ship":"长城","freighter":true,"mode":"Player"}]}` → 应用后
  `state.control.中国.ship_freighter == {长城:{mode:"Player",value:true}}`；点「恢复出厂值」→
  载荷 `{"ship":"长城","remove":true}` → 应用后回到 `{}`、按钮自己消失、那一行改口
  「当前跟随：出厂快照（战舰）——这一层还没有叶，自动控制随时可以给这艘舰定编」。
  再把**舰队默认角色**设成玩家 + 运输舰 → `default_freighter == {mode:"Player",value:true}`，
  长城的行跟着改口「当前跟随：舰队默认（运输舰）——它是玩家钉的 ⇒ 自动控制的逐舰定编不碰这艘舰」
  （正确的**例外**：势力级默认是玩家时，逐舰行也归玩家 ⇒ 编辑器开放）。另外 8 个势力的
  control 段全程一个字节没动。

### 12.5 顺带：两条"文档也会过期"的修正 + 一条断言的教训

* `ship-blueprint-spec.md` 里的版本号**已经过期**（它写 7→8，而合并后 `SCHEMA_VERSION = 9`）。
  已按现状改成 **9 → 10**（§1 改动地图、§6 迁移、§8 动手顺序、§9 附 A 四处 + 顶部状态行），
  并写明 v7→v8（产地货栈）、v8→v9（货舱 + Haul）**都已被 freight 用掉**。
  ⚠ 动手前先 `grep SCHEMA_VERSION src/model/state.rs`——版本号是最容易被并行会话吃掉的东西。
* `web/static/app.js` 里那句「风格两轴上的 `Auto` 是空头承诺」加了限定：**逐舰角色叶是例外**
  （AI 真的每回合写它）。势力级默认角色叶**不**是例外（没人写它），UI 提示因此是两句不同的话
  ——最初照抄逐舰那句「由自动控制定编」，实机一看是假话，当场改掉（note §3.2 的措辞纪律）。
* **`verify` 是只读演习**这条又咬了一次：新写的 demo 里有两条断言（"再删一次是幂等的"、"删势力级
  默认叶在回执里"）**默认了第一次删叶/建叶已经落地**，而它们其实只被 `verify` 演习过 ⇒ 两条假失败。
  修法是先 `--apply --save` 再对**新 checkpoint** 断言，并把这条坑写进 demo 的注释里（§11 那轮
  也踩过同一个坑——一个坑踩两次，说明它该出现在被复制的地方，而不只是笔记里）。

### 12.6 下一步

③ ~~**蓝图**~~ → `[x]` **已完成**（`feature/ship-blueprint`，`SCHEMA_VERSION` **9 → 10**；
`Ship.spawned_round` 一并落地并**进了投影 ships 表**——编制表的 tie-break 终于能说「取最老的」）。
实现记录见 [`ship-blueprint.md`](ship-blueprint.md) §6。这一步把本篇的活层模型往「还不存在的舰」
那一侧推了一格：设计图是**图（舰级层）**，链变成 `叶 → 图 → 舰队默认 → 势力 → 全局`，
但**图的意图轴默认沉默**（建图 ≠ 表态）——正是 §3.2 那条「Auto 必须是真执行者」的延伸。
⚠ **2026-10 修订**（`blueprint-stance.md`）：图**不再携带指令**（`order` 删除），它携带的是
**倾向三轴**（风格 / 风筝姿态 / 角色），链也改成 `叶 → 出厂图 → 舰队默认 → 势力 → 记录值`；
指令链则缩成 `叶 → 势力 → 全局`。

再往后是排队项（方案 B「逐舰取值规则与文档对齐」、风格轴要不要真的 AI 执行者、
`ship_orders` 读面列出每一艘舰、kit 的 `_approx` 列换成引擎的 `effective`/`order_source`）。

> ⚠ 蓝图的 intent-source 一节（`ship_behavior_source`）把本篇 §10.4 那条查实的**取值规则**
> 落成了读面：`order_source` 会把「**叶不存在**」与「叶写着 `Inherit`」分开报（前者才可能落到
> 图/舰队默认，后者诚实地报 `leaf`）。这正是「没表态 ≠ 没值」第一次进入正式读面契约——
> 方案 B 若哪天要做，改的就是这里。

## 13. 本轮：`ship_orders` 读面每舰一行（A）+ kit 的 `_approx` 换成引擎的答案（B）

分支 `feature/read-face-parity`（从 `main` = `994455d`，`SCHEMA_VERSION = 10` 起）。
**不升 `SCHEMA_VERSION`**：叶值与状态形状都没动，改的是**读面/投影的消费者**。同一 seed 的
`--digest` **逐字节不变**（见 §13.4）。

排队项两件事一次做完，因为它们是同一个毛病的两端：**读面只给"我这边有的"，而不是"这件事的
真相"**——一端是引擎少列了每艘舰，另一端是 Python 自己重算了一遍链。

### 13.1 A：缺口与改法（`ship_orders` 读面「每舰一行」）

**缺口**（§10.5 记下的那条，本轮把它做实）：`control_view` 里 `ship_orders` 遍历的是
`c.ship_orders`（**只有存在叶的舰**），而三条风格轴（`ship_doctrine` / `ship_kiting` /
`ship_freighter`）遍历 `state.ships`（**每舰一行**）。后果实测（一个跑到第 20 回合的真实档）：

```bash
# 删掉「长城」的指令叶（= kit 的删叶 / web 的「恢复出厂值」）
planet_x --start c20.ron --apply rm.json --round 0 --save c20r.ron
planet_x --start c20r.ron --control | jq '.control[] | select(.faction_id=="中国") | .ship_orders | length'
#   6   ← 中国的舰有 7 艘（`ship_doctrine` 那三行是 7 行），长城**整行不见了**
```

⚠ 比"少一行数据"更糟：web 的那棵树是**从指令行长出来的**（`app.js::buildTree` 遍历
`fc.ship_orders`，一艘舰的风格 / 角色两行是它的**子节点**）⇒ 叶一被删，这艘舰的**风格与角色
两行也一起消失**，玩家再也没法在界面上单独给它设归属。

**改法**（与风格轴逐字同形）：

| | 改前 | 改后 |
| --- | --- | --- |
| 遍历 | `c.ship_orders`（有叶的舰） | `state.ships` 过滤本势力（**每舰一行**，顺序与 `state.ships` 一致） |
| `behavior` | 叶里的值（`ShipBehavior`，非空） | **有效值** `State::ship_behavior` ⇒ `Option<ShipBehavior>`：`null` = **链上没有任何一层说话**（调用方按 `Idle` 兜底） |
| `mode` | 叶的 mode | 叶的 mode（**没有叶 = `Inherit`**，与改前一致） |

写面**没有**改形状：`ShipOrderPatch.behavior` 本来就是 `Option`（`null` = 不动），所以
「`--control` 模板原样回传」照旧安全；`--control-schema` 由 `CommandReq` 派生，**自动跟随**。

**⚠ 顺带一条必须补的写面规则（否则"不动点"当场破）**：读面每舰一行之后，模板里会出现
`{"ship":X,"behavior":null,"mode":"Inherit"}`。而 `apply_ship_order` 以前**无条件**
`or_insert_with(|| Control { value: behavior.unwrap_or(Idle), mode: implied })` ⇒ 回传模板会给
每艘"叶被删过"的舰**重新建出一片 `value = Idle` 的叶**，把「链上没人说话」（`null`）静默变成
「叶里记着 `Idle`」（`Some(Idle)`）：一次**没人要求**的写操作，而且第二次 `--control` 与第一次
不再逐字节相同。

规则（与 §11.1 规则 2「删一片本来就不存在的叶 = 幂等成功」同源）：

> **既没写值（`behavior` 缺席/`null`）、表态又是 `Inherit` ⇒ 不建叶**（`report.applied += 1`，
> 不进 `skipped`）。叶**已经存在**时照旧只改表态、不动值；写值或写 `Auto`/`Player` **照旧建叶**
> ——那才是"给这艘舰设归属"的入口。

### 13.2 A：三端接口

| 端 | 动作 |
| --- | --- |
| 引擎 | `ShipOrderEntry.behavior: Option<ShipBehavior>`（读面 `null` = 没人说话）；`control_view` 改为遍历 `state.ships`；`apply_ship_order` 加「不建空叶」规则（`src/control.rs`） |
| web | `web/src/lib.rs` 新增守卫 `the_order_read_face_lists_ships_without_a_leaf`（每舰一行 / 删叶后仍在 / 只写 mode 能把叶建回来）；`app.js` 把 `behavior === null` 显示成「**无人表态（按待命兜底）**」而不是「待命」（`behaviorType` 新增 `unset`，编辑器里那一项是 disabled 的只读显示——想真下达待命就选「待命」，那才是一次写值=接管）。「恢复出厂值」按钮仍按 **`st.control` 里的原始叶**判定（读面看不出来） |
| kit | ⚠ **不是"无需改动即兼容"**——本分支第一版就是这么判的，于是踩了 **§13.6** 那个洞（`order_leaf` 恒 `True`、`order_behavior` 静默变成有效值）。现在 `ships()` 的「本舰那片叶」四列（`order_leaf`/`order_mode`/`order_value`/`order_behavior`）改从投影的 **`derived.control`** 表读（只列真实存在的叶），有效三列继续读引擎列 |
| 文档 | `agent-play.md` §4.2 补一段「`ship_orders` 每舰一行 + `behavior: null` 的含义」 |

### 13.3 B：缺口与改法（kit 的 `_approx` 列换成引擎的答案）

**缺口**：`planet_x_ctl.ships()` 在 Python 里**重算了整条链**
（`effective_order_mode_approx` / `effective_order_value_approx` / `effective_authority_approx`），
而那条链是**设计图之前**写的（`叶 → 舰队默认 → 势力 scope → 全局`），**没有蓝图层**
（`叶 → **出厂图** → 舰队默认 → …`）⇒ 对任何"按图造的舰"给出的是**错的**答案，而列名看着像
权威答案（`README` 第 8 条自己记着这个洞）。同时 `effective_authority_approx` 与
`effective_order_mode_approx` 是**两列答同一个问题**，且都不是引擎的答案。

**改法**（引擎列在就绝不自算；不在就诚实降级）：

| 帧 | 列 | `effective_order_from_engine` |
| --- | --- | --- |
| 引擎列在 | `effective_order_mode` / `effective_order_value` / `order_source` | `True` |
| 引擎列缺席（旧 index 目录） | `effective_order_mode_approx` / `effective_order_value_approx` / `effective_authority_approx` | `False` |

* `effective_order_mode` ← `order_effective_mode`（`State::ship_control`）、
  `effective_order_value` ← `behavior_str(order_effective)`（`State::ship_behavior`）、
  `order_source` 就是引擎自己那一列（**原样穿过、不改名**，`State::ship_behavior_source`）。
  ⚠ 这三列不是一回事：`order_source` 答「这条**值**是谁供的」（`leaf` / `blueprint:<图名>` /
  `fleet_default`），`effective_order_mode` 答「这艘舰**归谁**」（叶→图→默认→势力→全局）。
* `APPROX_COLUMNS` 从"本地重算"降级为**仅旧 index 目录的兜底**，名字带 `_approx`；
  `ENGINE_EFFECTIVE_COLUMNS` / `EFFECTIVE_PROVENANCE_COLUMN` 两个常量导出给配方用
  （两种来源**列名不同**，所以读面一眼看得出是谁算的；布尔列让 `df.query` 也能问同一句话）。
* 顺带把 `behavior_str()` 那条**只读**的撒谎修了：引擎新增的 `Haul{from,to}`（运输）不在 kit 的
  六种行为表里，旧实现渲染成 `"Haul:"`——看着像被截断的值，还丢了 `from`/`to`；现在原样 JSON。
* 检查过但**没改**：`df["kiting"] = kit` 曾覆盖引擎的 `kiting` 列。核实后是**非问题**（读面那些
  逐舰风格行给的**也是有效值**，与引擎同源同值），但改成了**只在引擎列缺席时才补**——
  与 `effective_order_*` 同一条纪律：**引擎给答案，Python 只负责筛**（`engine-data-plane.md` §0）。

### 13.4 验证

* `cargo test --workspace` 全绿：`planet_x` lib **140**+1ignored（新增 2 条：
  `the_order_read_face_lists_every_ship_and_is_a_fixed_point`、
  `a_null_behavior_row_never_invents_a_leaf`）、`longhorizon` 6+10ignored、
  `projection_derived` 4、**新增集成测试 `tests/control_read_face.rs` 1**（真实 CLI 端到端）、
  `planet_x_web` **20**（新增 1）+2。
* **行为中性**：`--seed 42 --round 240 --digest 20` 只取 `^{` 行（12 行）的 sha256 =
  `293725C43A0E26DC516977C04A5BD9C99977252B8C08D683EC2EA2749ADEDBC4`，与 `main` 基线**逐字节
  相同**，连跑两次相同。应然：本轮只动读面与**写面的建叶条件**，而模拟进程不消费 `control_view`。
* **A 的端到端（真实 CLI，两条独立证据）**：
  1. `tests/control_read_face.rs`：造一个真实世界（seed 42 / 12 回合）→ 删掉一艘舰的指令叶
     （回执 `NOTE_APPLY_REMOVED`）→ `--control`：该势力**行数 == 舰数**且顺序与 `state.ships`
     一致，被删那艘是 `{"behavior": null, "mode": "Inherit"}`、其余**非空** →
     模板**原样** `--apply --round 0 --save` → 再 `--control`：**逐字节相同**，且回传后那行仍是
     `null`（叶没被重建）。
  2. 手工在 20 回合的真实档上跑同一条链：`430D2F8A7828B50B3FAED0AFE9E670A03B99F1C7EEB28AC7FC75ADC76E18B541`
     改前=改后（`T0 == T1`），checkpoint 里逐字节确认**长城那片叶没有被建回来**（其余 6 艘的叶原样）。
* **B 的端到端（`play/planet_x_ctl`）**：`uv run python demo.py` **全部断言通过**，其中 4 条 +
  新增的 §[4d] 十条（10 条）是本轮新增/改写的：
  * 「有效指令列来自**引擎**（含设计图层），不是本地重算」——13 行与
    `order_effective_mode` / `behavior_str(order_effective)` 逐值相同、来源布尔列全 `True`；
  * 「引擎列在时**没有** `_approx` 列」；
  * 「旧索引目录 ⇒ 退回本地近似，且读面说得出『这一帧是谁算的』」——把当年那几列从
    `idx/ships.jsonl` 里删掉，真的造一份旧引擎目录跑一遍（`demo.py::_old_engine_index`）；
  * 「降级路径与引擎在**这一帧**给出同样的结论」（差别只在蓝图层，而这一帧没有按图造的舰）。
  * ⚠ 顺带修了 demo 一条**被本轮暴露**的断言 bug：`C` 步骤原本用 `{(c.leaf, c.field, c.after)}`
    做集合比较，而读面改给有效值之后，`incidental` 里第一次出现了 `behavior` 字段，它的值是
    **dict** ⇒ `TypeError: unhashable type: 'dict'`。现在拆成两条断言，并且**按配方自己的意图**
    断言那 4 处变动（「跟着舰队默认走的舰，有效指令真的换成了那一句」）——那是**真事**：旧的
    近似读面根本看不见"舰队默认一落地，这些舰的有效指令当场就换了"。
  * ⚠ 另有一处**排版 bug**（不是本轮引入）：`demo.py` 第 5 节的 `print("\n[5] 确定性…")` 被并进了
    上面那行注释里（那一节的标题从没打印过）。修了。
* **§13.6 那个洞的证据（审查方探针 `scratch/probe_leaf.py`，修前/修后各跑一次）**：见 §13.6。

### 13.6 ⚠ 一个被审查方钉住的真洞：读面改形状，**kit 里那三列的语义被静默换掉了**

**这是「契约有两端」那条老教训的第 N 次复发**（`engine-data-plane.md` §8.1/§8.7），而且这次
症状最阴：**列名没变，语义变了**。

**怎么来的**：§13.1 把 `--control` 的 `ship_orders` 改成「每舰一行、`behavior` 给**有效值**」，
而 kit 的 `Surface._index` 正是从 `--control` 的 `fac[kind]` 列表建的 ⇒ `ships()` 里那三列
（`order_leaf` / `order_value` / `order_behavior`）跟着变了意思：

| 列 | 它**应该**说 | 改形状之后它说成了 |
| --- | --- | --- |
| `order_leaf` | 这艘舰**有没有自己的叶** | **恒 `True`**（读面对每艘舰都有一行） |
| `order_value` / `order_behavior` | 那片叶**自己记着的值** | **有效值**（叶 → 出厂图 → 舰队默认） |

**审查方的探针证据**（`scratch/probe_leaf.py`：把中国的舰队默认钉成 `Player` + `Dock 月球`，
于是"叶里的记录值"与"有效值"必然分道扬镳；中国 7 艘舰**本来都有** AI 写的 `Inherit` 叶）：

```
修前（假话）：order_leaf 恒 True；order_behavior = Dock:月球（= 有效值）
              真叶里记着的却是 {"DockCity":{"city":"泰坦采矿城"}} / {"Haul":{...}}
修后（实话）：
舰    order_leaf order_mode order_behavior(kit)                 有效值(引擎)  order_source   真叶(control 表)
长城   True      Inherit   DockCity:泰坦采矿城                  Dock:月球     fleet_default {"DockCity":{"city":"泰坦采矿城"}}
北斗   True      Inherit   {"Haul": {"from": "金星", "to": "地球"}} Dock:月球  fleet_default {"Haul":{"from":"金星","to":"地球"}}
中国：6 艘（删掉北斗的叶之后）order_leaf = False，order_behavior 空，有效值照旧 Dock:月球
```

**修法**：「本舰那片叶」这件事只能问**只列真实存在叶**的那个读面 —— 投影的
**`derived.control`** 表（`idx/control.jsonl`；引擎遍历 `c.ship_orders` 生成，
`kind == "ship_order"`）。于是 `ships()`：

* `order_leaf` = 这张表里有没有该舰的 `ship_order` 行；
* `order_mode` / `order_value` / `order_behavior` = 那行里的 `mode` / `value`（叶自己的记录值）；
* `effective_order_*` / `order_source` **照旧**读引擎列（§13.3）；
* 投影太老、没有 `derived` 段 ⇒ `q.derived()` 响亮报错（**不猜**：猜就是又一次"名字没变、
  语义变了"）。

**这件事顺带带来一个正收益**：「**叶里的记录值 ≠ 有效值**」第一次在 kit 侧可读（今天只有引擎列
回答得了一半）。`demo.py` 新增 §[4d] 把它钉死（10 条断言）：叶在、叶说 `Inherit`、舰队默认是
`Player` ⇒ `order_behavior` 是叶里那句旧的、`effective_order_value` 是默认那句、
`order_source == "fleet_default"`；再删掉一艘舰的叶 ⇒ `order_leaf` 翻成 `False` 而有效值照旧。

> **教训（写进措辞纪律）**：读面**改形状**时，凡是"从读面推出来"的消费者列都要重新问一遍
> 「这条信息的**权威来源**是哪个读面？」——`--control` 是**写面模板**（值 = 有效值 + 表态），
> 而「叶存在吗」是**状态**问题，它的家在 `derived.control`。两条读面各答各的，
> **别让一个形状变化把另一个问题的答案换掉而不同时改列名或改名**。

### 13.5 未做 / 已知边界

* `[ ]` **没在 demo 里现场演"两套链分道扬镳"那一格**：只有**按图造的舰**能让引擎与本地近似给出
  不同答案，而造出这样一艘舰需要真实船坞下水——玩家归属的图**买不起就不下水**（Q4(b) 是刻意的），
  指针挂在建造区上而 12 回合的 fixture 里城市倒戈/夷平很频繁（实测两次探测：一次指针随城被
  重建、一次城易主），便宜的护卫舰也要几十回合攒进度。demo 因此只断言**契约**（引擎优先 /
  降级标注 / 无图舰上两套一致），把这一格记在这里而不是写一条会飘的断言。
* `[ ]` **读面的 `ship_orders` 只列「活着的舰」**：叶还在、舰已不存在的**陈叶**不再出现在
  `--control` 里（改前会列）。实测这条路径今天不可达（`sweep_dead_ships` 每回合
  `retain(alive)`，`apply_diff` 也拒绝给不存在的舰建叶），真要删陈叶用 `remove: true` 即可
  （它不要求实体还在，§11.1 规则 1）。
* `[ ]` **`--control` 的 `behavior: null` 行在 web 上仍是"可编辑的"**：把类型下拉改成真行为
  就等于写值即接管（`Player`），这是既有语义；只是那一格现在多了一个 disabled 的
  「（无人表态 · 按待命兜底）」显示项。
* `[ ]` **web 那半只跑了 crate 测试，没实机点过**（措辞纪律：别把"代码写了"说成"验证过了"）。
  `planet_x_web` 的守卫（每舰一行 / 删叶后仍在 / 只写 mode 能把叶建回来）是**引擎侧**证据；
  `app.js` 的「无人表态（按待命兜底）」是**代码级**改动（`behaviorType` 新增 `unset` +
  编辑器里一个 disabled 的只读显示项），本轮没在浏览器里点过：第一次尝试走 `scripts/web.ps1`
  时撞上的其实是**别的 worktree** 的实例（`/api/ping` 的 `exe` 是 `C:\resource\planet_x\target\…
  且 `owner_pid: null` 的孤儿），而自己那个实例随启动 job 被收走（租约生效）⇒ 没拿到实机证据。
  要补就三步：起服务（`scripts/web.ps1`，用 `/api/ping` 核对 exe 是本 worktree 的）→ 点掉一艘舰
  指令行的「恢复出厂值」→ 应用后看那一行是否改口成「无人表态（按待命兜底）」。
* `[ ]` **方案 B（逐舰取值规则与文档对齐）仍留在桌上**——本轮把「叶不存在」与「叶 Inherit」
  的差别**读面化**（`order_source` + A 的 `null`），取值规则本身一个字没动。
* `[ ]` **风格/角色三轴没有跟上 §13.6 那条修法**：它们的 `exists`（kit 的 `Surface.leaf`）仍然是
  "读面列了这一行"（逐舰行永远在），删叶落地只能看 `NOTE_APPLY_REMOVED` 回执。**指令轴现在有了
  权威答案**（`derived.control`，`ships()["order_leaf"]`），同理可以给风格三轴各加一列
  （`control` 表里已经有 `ship_doctrine`/`ship_kiting`/`ship_freighter` 的行，改的是 kit 侧）；
  本轮没做，因为它牵扯"叶存在性"这条读面契约的**整体**说法，值得与方案 B 一起定。

## 16. 本轮：**风格三轴的真执行者**（`feature/ai-agency`，`autocontrol::style`）

> 这一节与 §17 是同一轮的两个 `Auto` 执行者。**`SCHEMA_VERSION` 不动**（没动叶值形状，
> 只多了两个"记录用"的判定种类），但**同 seed 的 digest 变了**——这是**有意的行为改变**
> （给空头承诺补执行者，见 §3.2），新基线见 §18.1。

**一句话**：`Auto` 在风格轴上不再是"值冻结"——`autocontrol::style::regulate_styles` 每回合按
**战况**重估归 AI 管的逐舰风格叶（`ship_doctrine` 的 temper+lone_wolf、`ship_kiting`），
写回 `Control::inherit(...)`（流水）。

### 16.1 位置与节奏（两条都有理由）

* **调用点**：`sim::step_military` 的逐舰循环**之后、护甲再生之前**。此时本回合的接战/撤退/战沉
  都已发生（`state.events` 与本回合的 `decisions` 都是现成的），而护甲还没回满（"被打残"这个
  信号还新鲜）。写下的值**从下一回合起生效** ⇒ 与舰的处理顺序无关（那个顺序是按 rng 打乱的），
  也不改变本回合任何判定。
* **概率触发 + 分布步长**（仓规：不用硬阈值/贪心）：每回合每舰每轴先抽一枚
  `P(重估) = autocontrol.style_chance`（默认 0.35），中了再走**指数松弛**一步
  `新值 = 当前值 + (战况目标 − 当前值) × roll × style_step_max`（默认 0.55）。
  风格是**慢变量**（它表达"这支部队在打什么仗"，不是"这一发打谁"）；指数松弛离目标远时改得多、
  近了改得少 ⇒ **天然收敛、不会来回抖**。变化小于 `style_epsilon`（0.01）就不写叶——控制面 diff
  是给人读的。
* **骰子**：只走 `sim::derived_roll`（`(势力, 舰名, 回合, 用途)` 派生；temper 与 lone_wolf 用
  不同 salt ⇒ 同一片叶上两条轴不共用一枚骰子），**绝不消费主 `Prng` 流**。每艘舰独立抽签
  ⇒ 舰队不会整体同步抖动。

### 16.2 驱动力（只用引擎已有的可读状态，不新造隐藏状态）

| 轴 | 目标值 |
| --- | --- |
| `temper` | 战况：`sim::war_strength`（在打，+）× `temper_war` + 本回合净战果（击毁敌舰 − 战沉，+）× `temper_win` − 舰队被打残程度 `1−Σhull/Σhull_max` × `temper_damage` − 本回合自保撤退舰数占比 × `temper_withdraw`。四项全 0（和平）⇒ 目标 0 ⇒ 自己松弛回基线。⚠ 只算 `DeathCause::Combat` 的战沉：维护费欠缴的锈蚀报废不是战果 |
| `lone_wolf` | **编队规模**：本舰护航半径（`lone_wolf_radius`，0 = 用 `combat.escort_range`）内的友舰数 `n` ⇒ `1 − 2·n/(n+lone_wolf_ref)`（孤舰 +1、成群 → −1）。平滑、无阈值 |
| `kiting` | 敌我火力比（`sim::deterrence` 对数比，强则贴脸）+ 硬度对比（船体+护盾的和差比）+ 挨打程度（`1−hull/hull_max`，残则拉开）。**附近没有敌舰 ⇒ 目标 0**（这条轴只在接战时才有意义，与 `kiting_dest` 同一把尺子：`tactics::nearest_enemy_ship`，感知半径 = 武器射程 + 0.5） |

每次真的改了就往 `view.decisions.styles` 追加一行（`StyleDecision`：轴 / 旧值 / 新值 /
目标 / 当时读到的驱动输入）——投影 `idx/decisions.jsonl` 的 `kind = "style_retune"`。

### 16.3 闸门（宁严勿宽）与"两轴一片叶"

* 闸门三道：① 舰本身归 AI（`ship_control == Auto`）；② 该轴的归属链（叶 → 舰队默认 → 势力 →
  全局）不是 `Player`；③ **势力级默认叶是 `Player` 就一律不写**——比链更严一档，理由是用户
  裁决："那条默认就是玩家的答案，AI 的流水不该盖它"。**闸门逐轴判**：只钉了 `default_doctrine`
  时 `kiting` 仍归 AI（两条轴各有各的默认叶，测试里钉住了这个不对称）。
* `temper` 与 `lone_wolf` **同处一片叶**（`Control<ShipDoctrine>`）⇒ 两条轴的目标分别算，但
  **只写一次**，且没被重估的那条带着它**当时在用的值**（= `control-live-layers.md` §3.1 那条
  「只改一条轴不许把另一条清零」的落点）。
* 写的是 `Control::inherit(...)`（**流水，不是指令**）：玩家把更宽的那层钉成 `Player` 时，
  AI 的流水不许遮住它——与指令轴/角色轴同一条规矩。

## 17. 本轮：**`Auto` 设计图的执行者**（`autocontrol::blueprints`）

**一句话**：AI 势力现在会**自己建图 / 重估选装 / 去重复用 / 回收**，并把它挂到自己建造区的
指针上。此前那一层只有半个执行者（`retool_shipyards` 只会重估**已有图**的舰级）。

### 17.1 形状（三个决定）

1. **一张图 = 一个 `(舰级, 设计主题)`**。主题（`config/game.ron` 的 `autocontrol.blueprint_themes`：
   强袭/堡垒/远洋/平价）决定选装评分里各模块**分类的权重**与**造价惩罚**；主题按**抽签**选
   （权重 = `weight + 战争强度 × war_weight`），主题换掉 = 换一张图（旧图没人指向就被回收）。
   图名就是它的身份：`自动{主题}·{舰级}` ⇒ **图库是 `O(主题数 × 舰级数)` 而不是 `O(回合数)`**
   ——这就是 `ship-blueprint.md` §5「设计图爆炸」的答案；另有**按 `(舰级, 选装签名)` 归并复用**
   （同签名的自建图直接指过去，不再新造）。
2. **建图 ≠ 表态**（Q1(c)）：AI 写的图 `order: None`，叶片一律 `Control::inherit(...)`。
3. **选装是"当时的快照"**：由 `choose_loadout_themed` 生成（与出厂现算**同一套**可得性检查），
   库存变了就**按概率重估**（`blueprint_chance`，默认 0.25/回合/舰级）；**已下水的舰不受影响**
   （`Ship.components` 是出厂快照，有测试钉住）。

### 17.2 闸门与回收

* 玩家钉的图（`blueprint_control == Player`）⇒ 那个建造区**整个跳过**（不改图、不改指针）；
* **悬空指针**（图被删）⇒ 跳过：删图是玩家/agent 的动作，那个区停产本身就是可见后果；
* 想用的名字被玩家的图占了 ⇒ 这一轮对这个舰级什么都不做（不改名、不抢名字）；
* 回收：**没人指向 + 名字带 `自动` 前缀 + 不是玩家钉的**三者齐备才删叶。第三个条件顺带解释了一个
  安全属性：玩家/agent 用 `--apply` 写的图会被「写值即接管」钉成 `Player`，而 AI 自己写的是
  `Inherit` ⇒ **回收只会碰 AI 自己造的东西**（测试里钉住）。已下水的舰仍带着自己的快照，只是
  出处指针可能指向一张后来被回收的图（读面原样输出）。

### 17.3 ⚠ 两处语义改动 + 一条**平衡证据**（本轮实测，留给上层裁决）

1. **`resolve_loadout` 改掉"`Auto` 图的选装被静默忽略"**：新语义是
   **图 = 出厂规格（装什么），`mode` = 谁可以改这张图**。因此 `components` 非空时**任何**归属
   都按图装配；空数组仍 = 交给生成器。不这么改，"AI 建图"就只是空头支票（AI 画的图不会影响
   任何一艘船）。
2. **新增 `autocontrol.blueprint_stock_margin`（默认 `0.0`）**：图在**回合步进里**画，钱在
   **下一回合出厂那一刻**付；不留余量时 `commit_spend` 的钳零会白送模块，而**白送的模块照样付
   维护费**。余量 = "画得出的图，钱要留一倍"。**默认故意是 0（= 与出厂现算同一条线）**，因为
   实测 **1.0 会在部分种子上把舰队推过「养不起→锈蚀→整队拆解」的悬崖**：
   `--seed {1,7,123}` / 300 回合，余量 1.0 时 `min_ships` 打到 **0**（世界停在 0 舰、无法复垦），
   而余量 0 时八个种子 `min_ships` = 2–20、末态 23–49 艘（与 main 同量级）。要开它就是一行，
   开了请重标长局。
   * 也试过**另一条路**并否掉了：把「买不起就不下水」（今天只对 `Player` 图生效）扩到**所有**
     带选装的图。结论是**它会把造船业掐死**——同 seed 300 回合只有 **5 次出厂**（main 同期
     88 次/200 回合），世界冻结在 4 艘舰、21 座城 ⇒ 那是"死棋盘"，比白送模块更糟。
   * 同一轮扫出的另一条观察（未结论）：主题的**分类权重**（如 `强袭` 武器 ×1.8）在当前配置下
     会让部分种子的长局更早收敛；本轮已把 `game.ron` 里四个主题的分类权重与造价惩罚取成
     **保守档**（全 1.0 / 0.0 = 不改变选装内容，只保留"主题"这个身份与记录），**机制照旧可调**。
3. **没做**（明确留给下一步）：把"买不起不下水"扩到 AI 图（理由见上）、按 `Player` 图的选装
   复用（AI 的建造区不采用玩家设计的图）、设计图主题的时代门控（`eras-technology.md` 的活）。

## 18. 本轮的验证与剩余

### 18.1 digest 与基线

`--seed 42 --round 240 --digest 20`（只取 `^{` 行、`\n` 连接、UTF-8 无 BOM）：
**`D693E83816FE5F5F9F4D067AC10B80B99FDB98D552FFD08A314A60ED09B19273`**（12 行），
连跑两次**逐字节相同**。旧基线 `293725C4…DBC4` **作废**——那是**有意的行为改变**（两个 `Auto`
执行者第一次真的动手），不是噪声。行为中性的替身证明：把 `style_chance` 设 0、`blueprint_themes`
清空 ⇒ 同 seed 240 回合的 digest 与 main **逐字相同**（两个执行者关掉就等于旧行为）。

### 18.2 测试与其余种子

* `cargo test --workspace` 全绿：`planet_x` lib **151** +1 ignored、`longhorizon` **6**+10 ignored、
  `projection_derived` 4、`planet_x_web` **19 + 2**。
  新增用例：`autocontrol::style` 5 条（真的在写叶 / 打残更保守 / 两轴一片叶 / 三道玩家闸门 +
  逐轴判 / 抽签与步长）、`autocontrol::blueprints` 7 条（建图与口径 A / 按签名去重且幂等 /
  玩家的图与建造区不碰 / 悬空指针原样留着 / 只回收自建且没人指向的图 / 改图不碰已下水的舰 /
  舰级漂移会重新对齐）、`sim` 1 条（**图的选装是出厂规格**，含"空选装仍走生成器"的对照）。
* 其余种子（本分支，保守档配置）：`seed 1/7/42/123/99/777/2024/31337` 各 300 回合——无 panic、
  城数 3–21、`min_ships` 2–20（没有一个种子掉到 0 舰）。

### 18.3 两处测试判据的改动（如实记下，不是"改测试让它过"）

1. `longhorizon::coalition_mechanism_is_alive`：判据从"**必须**送出 `CoalitionFormed` 事件"
   放宽成"事件**或**回合末 `view.coalition_members ≥ min_members`"。理由是那条事件报的是
   **跃迁**，触发取决于"关系先冷"还是"霸权先出现"这个与机制无关的先后顺序（成员在霸权出现前
   就已疏远 ⇒ `before_active` 一上来就是 true ⇒ 永不报事件），而卫兵要问的是**机制是否活着**。
   **事件本身的这个盲区是候选修复项**（要给它加"按霸权存续"的记忆），不在本轮范围。
2. `sim::tests::a_city_razed_this_round_is_not_refounded_by_its_own_loser_this_round`：非空
   样本从 `seed 7` 单个种子改成 `[1, 7, 42]` 三种子合计（不变式对每个种子每一回合照旧检查）。
   原因：seed 7 这条轨迹在新行为下安静了（400 回合 10 次拆平 / 4 艘舰，同种子上基线是 57 次 /
   33 艘）——**非空这条要求不该只押在一个种子上**。

## 19. 本轮：**控制行压成一行 + UI 描述文字整批删掉**（2026-10，用户裁决）

### 19.1 用户原话（两条，连着说）

> 「UI 中条目的 自动控制/玩家 选项单独一行很占地方，弄到同一行，但必须确保横向的 UI 不可以
> 被挤出屏幕，只会挤到换行」
>
> 「怎么这么多 help，太占空间了，删掉」（澄清：*「tooltip 又不占空间，我说的是 UI 上的各种
> 描述文字」*）

### 19.2 病根：一片叶最多吃 **3 行**

`renderLeafNode`（`web/static/app.js`）以前的结构是
`head = [标签][归属下拉][恢复继承]` → `编辑器` → `跟随说明`。而左栏详情层（`layout: layers`
的内层 = `.sv-sheet-row` 的 118px 标签列 + 1fr 值列）里标签**住在左列**，于是 `head` 那一行
只剩一个 54px 的下拉 ⇒ 一整行只放一个下拉；下面还有一句 `ctlFollowHint` 的说明。
实测一个资源预算行 **96px**，一屏放不下几行。

### 19.3 改法（一条规矩，五处落点）

**一片叶 = 一行**：标签 / 值编辑器 / 归属下拉 / 旁路按钮全住同一个新容器 `.tnode-line`。

* `web/static/app.js::renderLeafNode`：`head` → `line`；归属与「恢复继承」「删图」由闭包
  `ownBits()` 在**编辑器之后**追加（顺序：标签 → 值 → 归属 → 旁路）。
* `web/static/app.js::syncOwnership`：就地跟进的那个查询从 `:scope > .tnode-head` 改成
  `:scope > .tnode-line`（不然「刚敲了数，归属还写着继承」会回来）。
* `web/static/app.js::weightRow`（建造区那两片权重叶）：同样的顺序（标签 → 值 → 归属）。
* `web/static/style.css`：新增 `.tnode-line { display:flex; align-items:center; gap:6px;
  **flex-wrap: wrap**; }`。**`flex-wrap` 是硬要求**——面板只有 ~430px，装不下必须**换行**，
  不许把元素挤出屏幕；`.leaf-val` 也补上 `flex-wrap`（它以前是 `nowrap`，是同一类隐患）。
  死的 `.tnode-head` 规则整条删掉（没有别处再造它）。
* 角色下拉的选项文字从「战舰（找仗打（接战 / 轰炸 / 殖民））」缩成「战舰」，
  解释移进 `option.title`（**下拉宽度由最宽的选项决定**，这是它会把归属挤下去的原因）。

实测：资源预算行 **96 → 28px**（约 3.4×）；扫过所有 `.tnode-line`，没有一个子元素越过容器右缘
（`scrollWidth == clientWidth`）。

### 19.4 删掉的描述文字（**保留** ⚠ 告警与写回执）

| 删 | 位置 |
| --- | --- |
| 「这片叶没表态（继承）⇒ 现在按上层…改一个值就归你」 | `ctlFollowHint`（整函数） |
| 「由系统自动决定（要自己指挥就把左边的归属改成「玩家」）」×3 + 角色那两句 | `renderLeafNode` 的 else 分支 |
| 「没有叶（这一层没表态）——写一个数就是新建这片叶」 | `controls.js::leafNodeBox` 的 `hint` |
| 「N 项：N 项已有叶…写值即接管」 | `controls.js::leafRow` 多键叶表头 |
| 「新建的叶先在**编辑面**里…单独一片壳不进 diff」 | `controls.js::newKeyForm` |
| 「这张图自己没有表态（Inherit）⇒ 往上看…」 | `blueprintOwnershipHint`（整函数） |
| 「图上写了某条轴 ⇒ 之后…出厂就带这条倾向」 | `blueprintEditor` 末尾 |
| 「（factions：中国 现在 = Inherit）」（与下拉同义） | `controls.js::ownerRow` |
| 「这一条的全部字段都在这里：声明过的按人工顺序在前…」 | `specview.js::renderLayers` 的 `sv-note` |
| **页顶那 7 句**（`pages[].hint`）+ `spec.hint` 的渲染 | `views.json` / `app.js` / `specview.js` |

**故意留着两条**（不是漏删）：
1. `specview.js::omitLine`（「本视图声明不看 N 项：…」）——`g4_spec.py` §静态纪律要求
   每条 `omit` 都写 `why`，其判据文本明说*「界面要把省略说出来」*；删了它，判据就变成假话。
2. `controls.js::readOnlyNote`（「引擎现算（写面不写回）：…」）——那是**值**不是描述，
   前缀只说明"这列不写回"。

### 19.5 验证

* `bash scripts/check-js.sh` → `check-js: OK`（25/25 着色器 + 全部脚本 `node --check`）。
* `cargo nextest run -p planet_x_web` → **25 passed**（前端的改动不经它，但它是写面的边界）。
* `uv run --project play/planet_xq python play/tests/run.py`（快组）→ 组 1 绿；组 4 **36/39**，
  3 条红**全是** `events.攻击方`（「16 行里 0 格非空」）。**这 3 条与本次改动无关**：
  把 `views.json` 换回 `HEAD` 的版本重跑，红的还是同样 3 条（实测记录在案）。
* 实机（`scripts/web.ps1`，端口 3000）逐页看过：势力详情（舰队默认角色 = `[战舰▾][继承▾]` 一行）、
  城市详情（`福利权重 [7][玩家▾][恢复继承]` 一行）、舰队详情（`指令 [待命▾][继承▾]`）、
  建造区那一行（`结构/舰型/设计图/建设权重` 各一行）。**写值即接管**实测有效：
  在输入框里敲 7 ⇒ 同一个 `.tnode-line` 里的归属就地翻成「玩家」、并长出「恢复继承」按钮。

### 19.6 没做（留给下一轮）

* `.ctl-grid` 里每片叶外面那圈 `.tnode` 边框 + `margin: 4px 0` 还在（一行 28px 里占 8px）。
  要再挤，就在 `.ctl-grid > .tnode` 上去掉边框/外边距——但那会改变"每片叶是一个盒子"的观感，
  没做之前先问。
