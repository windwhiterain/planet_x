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

## 3. 剩余

* `[ ]` **doctrine / kiting 也变成活层**（本轮只做了「指令」）。
  现状：它们还是 `Ship` 上的**裸字段**（`ship.doctrine` / `ship.kiting`），出厂时从
  `ShipSpec.default_doctrine`/`default_kiting` 拷一份（`sim.rs::spawn_ship`），`--apply` 的
  `ship_doctrine`/`ship_kiting` 补丁**直接改字段**——没有三态、没有活层、也没有舰队级默认。
  改造面**很小**（已数过）：
  * AI 读点 5 处：`autocontrol/tactics.rs:90`（temper）、`:246`/`:264`（kiting）、
    `:338`（lone_wolf）、`:389`（kiting）；
  * AI **从不写** doctrine/kiting（`tactics.rs` 里那两处写点在 `mod tests` 内）→
    不存在「AI 覆盖玩家风格」的既成问题；
  * 读面/补丁面已有（`control_view` 的 `ship_doctrine`/`ship_kiting` + 两个 patch 结构）。
  改法：加势力级 `default_doctrine`/`default_kiting`（与 `default_ship_order` 同形同链），
  读点改走新加的 `State::ship_doctrine(id)` / `State::ship_kiting(id)` 有效值解析，
  `Ship` 上的字段降级成**记录值**。收益：「全舰队风筝 −1、战列舰贴脸 +1」= 两片叶子。
* `[ ]` **舰队默认的粒度**：现在只有「势力级单值」，表达不了「护卫舰守家、巡洋舰殖民、
  战列舰自由接战」。按舰级的 map（`BTreeMap<舰级, Control<ShipBehavior>>`，舰级取
  `Ship.class`，链 `叶 → 舰级 → 势力 → scope → 全局`）能表达。**待拍板 §4.2**。
* `[ ]` **读面看不出「有效值」**：`--control` 给的 `ship_orders[].behavior` 是**叶上的值**
  （`Auto` 时是系统流水），而某舰的**有效指令**可能来自舰队默认。web 已经自己算
  `effectiveMode`，但 agent 侧读不到。可选：给 `ShipOrderEntry` 加一个**只读**字段
  `effective`——注意 `FactionControlPatch` 是 `deny_unknown_fields`，加字段必须同时让
  patch 接受它（否则「编辑模板再回传」会当场报错）。
* `[ ]` **`agent-play.md` 要跟着改**：§3/§4.2 那句「注意这会让新造出来的舰默认 Idle」现在
  有了正解（舰队默认），要改写成「设舰队默认 = 新舰自动继承意图」；「省略 mode 保留当前
  模式」要补一句「但写值即接管」。

## 4. 待拍板（用户尚未裁决）

* **§4.1 快照 vs 活层**：方向已确认「控制属性=活层」；但 `Ship` 上的 `doctrine`/`kiting`
  字段保留为「记录值」这一点请再确认一次（它决定 §3 第一条的实现形状）。
  *推荐：保留为记录值*，与 `ship_orders` 的叶值同构。
* **§4.2 势力级默认风格：两片还是一个**：`default_doctrine` + `default_kiting`
  （与现有两个补丁面一一对应）vs 单个 `default_style{doctrine,kiting}`。
  *推荐：两片*（读面/补丁面不用包装，链与指令同形）。
* **§4.3 舰级默认要不要现在做**：若要做，JSON 形状建议现在一次到位，免得下一轮改 schema：
  `default_ship_order: {"*": {…}, "护卫舰": {…}}`（`*` = 势力级兜底）。
  *推荐：先只做风格活层（§4.1/4.2），舰级默认连同 `ship-blueprint.md` 一起设计*——
  因为「按舰级」离「设计图」只有一步，别把两者做成分叉的两套概念。
* **§4.4 读面加不加 `effective`**：见 §3 第三条。*推荐：加，但同轮改 patch 白名单*。

## 5. 复现 / 验证

```bash
cargo test --lib                       # 87 通过（含 fleet_default_* / writing_a_value_without_mode_takes_over）
cargo test --test longhorizon          # 6 通过（8 ignored 是慢诊断）
cargo test --workspace                 # 再加 web crate 的 15
# 旧档无损迁移的实机检查（旧二进制写的 checkpoint 用新二进制读）：
cargo run --bin planet_x -- --start ../planet_x/play/exp2/ckpt_r12.ron --control
#   → 长城/赤霄/北斗 = "Player"、北辰 = "Inherit"（与旧档的 Some(Player)/None 逐值一致）
```
