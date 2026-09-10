# 控制属性 = 活层（三态归属 + 层次化默认值）

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
* 叶没有说话（`Inherit`/缺叶）→ 取势力级 `default_ship_order` 的值，**但只在它自己是
  `Player` 时**。若舰队默认是 `Auto`，值应当由系统每回合现写，不能去用高层里那个可能早已
  过期的值；
* 都没有 → `None`（调用方按 `Idle` 兜底）。

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
* `[x]` **势力级舰队默认指令** `ControllableState::default_ship_order: Option<Control<ShipBehavior>>`
  （读面即写面：`DefaultShipOrder{behavior, mode}`）。归属链变成
  `叶 → 舰队默认 → 势力 scope → 全局`；值规则见 §1.3。**这就是「新舰出厂就有意图、
  一次性指令收尾有去处」的答案**（`agent-control-long-game.md` §5）。
* `[x]` **一次性指令不再脱手归属**：`sim.rs::reset_order_keep_mode`（殖民收尾只换值不换
  `mode`）；此前玩家点名的殖民舰建完城就被静默交还给系统。
* `[x]` **写值即接管**：只写值、不写 `mode` 的补丁 = `mode: Player`，覆盖
  `ship_orders` / `default_ship_order` / 两类预算 / 两类权重 / `loyalty_budget` / `capital`
  （`control.rs::write_value_leaf` + 各写点）。回执：`ApplyReport.took_over` +
  CLI stderr 的 `NOTE_APPLY_TOOKOVER`。理由：值写进去而归属仍解析成 `Auto`，系统下一回合
  就按自己的逻辑覆盖它，而 stdout/退出码一切正常 = 又一次「失败看起来像成功」。
  ⚠ 副作用：`agent-play.md`「省略 mode 保留当前模式」的旧说法对**值**不再成立（对
  「只写 mode」仍然成立）；显式写 `mode: Inherit` 是「撤回表态」，**不算**接管。
* `[x]` **web**：势力「舰」分组新增「舰队默认指令」一行（同形编辑器）；编辑器按**有效归属**
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
* `[ ]` **投影的 `control` tidy 表还缺四条风格叶**（引擎侧，与 web 无关）：
  `src/projection.rs` 的 `control` 表只发 `ship_order` / `default_ship_order` / 预算 / 权重 / 首都，
  缺 `ship_doctrine` / `ship_kiting` / `default_doctrine` / `default_kiting` —— Python 侧现在只能从
  `ships` 表的 `doctrine`/`kiting`（**有效值**）看结果，看不到这四片叶**自己的值与自己表的态度**；
  `kind` 的 schema 描述也还只列着老的七种。（→ 上面那个子 agent 只做了 web，按分工没自己糊引擎读面；
  这一条我来补。）

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

* `[ ]` **待裁决（引擎侧）**：要不要让引擎自己把缺的轴**初始化成"出厂记录值"**，或者把叶值
  改成两轴各自 `Option`？两条路的代价：前者要求引擎在建叶时能拿到该势力/该舰的现有风格
  （势力级默认更麻烦——舰队里各舰记录值可能不同），后者动到叶值的形状、读面与迁移。
  在裁决之前，**kit 的拒绝就是当前的正确答案**（响亮失败 > 静默改数值）。

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

* **势力级三行默认**：`舰` 分组的第一组子项从「舰队默认指令」一行变成三行——`舰队默认指令`
  （`default_ship_order`）/ `舰队默认风格`（`default_doctrine`：理智↔热血 + 护航↔独狼）/
  `舰队默认风筝姿态`（`default_kiting`：风筝↔贴脸）。三行同一套惯例：节点 kind 各一种
  （`fleetorder`/`fleetdoctrine`/`fleetkiting`）、同样的 `scope: 'leaf'` 三态下拉
  （继承/自动/玩家）、同样的「有效归属是玩家才给编辑器、否则一行 `.tnode-hint`」。
  读面里没有这两片叶（开局就是）时前端补一片 `Inherit` 的叶让行出现，与作用域「没列出的层 ≡
  继承」同义；回传等价于「这一层没有意见」（引擎声明的「模板原样回传安全」）。
* **摘要诚实**：势力级默认行的摘要只在**它自己是 `Player`** 时显示那几个数（`Auto` 显示
  「自动（值由系统写）」、`Inherit` 显示「未表态」）——引擎的取值规则就是「默认叶只在自身是
  Player 时供值」，把没表态的存储值显示成"当前风格"会骗人。
* **`effectiveMode()` 按轴选默认叶**：新增 `DEFAULT_LEAF` 映射
  `shiporder → default_ship_order`、`shipdoctrine → default_doctrine`、`shipkiting →
  default_kiting`（对应引擎 `ship_control` / `ship_doctrine_control` / `ship_kiting_control`）。
  以前只认 `default_ship_order`。
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
