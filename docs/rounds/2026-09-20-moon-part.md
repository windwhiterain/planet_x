# 30 · 第 7 轮：场景配方的 `kind = "moon"`（补齐能力）+ 卫星凌日（V12 完成）

目标：`23` 的两张表各挑一条。视觉挑 **V12（卫星凌日）**，系统侧那条**不是**从表里挑的，而是做 V12
时撞出来的：**场景编译器根本写不出"天上还有一颗小球"**（`kind` 白名单只有
planet / clouds / atmosphere）——于是这轮的系统项就是**把这个能力补上**。

---

## 一、系统：`kind = "moon"`（新 part 种类）

改的三处（都在 `px-scene`）：

| 改在哪 | 内容 |
|---|---|
| `vocab.rs` | 新结构键表 `MOON_KEYS = ["radius", "subdivisions", "position", "spin"]`（其余一律交给本 part 那份 shader 的契约判，与 planet 同口径） |
| `recipe.rs` | `compile()` 里的白名单抽成 **`check_kind()`**（报错要点名它认哪些）；新分支按 `kind == "moon"` 逐个装配物体：内建球 + 本 part 材质 + 带 `position` 的变换 |
| `recipe.rs` | 几何/变换装配抽成 **`moon_body()`** —— 抽出来是为了**能单独判**（场景级判据要烘图，代价大得多） |

**两条定点判据**（`px-scene` 43 passed，其中新增 2 条）：

* `a_moon_part_carries_its_position_and_radius_into_the_object`：`position` 进 `translation`、
  `radius` 进内建球、`subdivisions` 有默认值。
* `an_unknown_part_kind_is_named_against_the_list_it_knows`：`moon` 放行；`asteroid` 被拒时
  报错必须**同时点名**"写错的那个"与"它认哪些"。

⚠ **第二条测试当场抓到我一个错**：我把 `radius`/`subdivisions` 写成了 `number_or()`（它给 **f32**）
再塞回 `Value::Num(f64)` —— 于是配方里的 `0.093` 会变成 `0.09300000220537186`。
那是**静默改产物字节**（几何参数在产物里就是 f64）。改成 f64 读法之后 43 passed。

## 二、视觉：卫星凌日（V12）

`art/scene/orbit-gasgiant.toml` 加一个 `[[parts]] kind = "moon"`：

```toml
[[parts]]
id = "satellite"
kind = "moon"
shader = "gasgiant"
members = { shader = "shaders::gasgiant" }
params = { radius = 0.093, subdivisions = 48, position = [-0.62, 0.30, 1.32], spin = 0.0, … }
```

⚠ 它用的是**气态巨行星那份材质**（次表面散射），但**没给条带图** ⇒ 渲染器的兜底空白条带图让
`band` 恒为一个常数 ⇒ 出来是一颗**均匀的奶黄球**，正是参考图里那颗卫星的样子
（"生成物只为消费者烘"这条在这里反过来用了一次：**没有消费者就不烘**）。
⚠ 它 `cast_shadow: true` ⇒ 太阳 → 卫星 → 行星，球面上会落下**卫星的影**（实测图里那道斜影）。

出图 `target/shot-gasgiant.png`（960×640，510568 B，sha256 前 16 `9CEE0C0866C87F0E`）。

## 三、判据

| 改动 | 判据 | 读数 |
|---|---|---|
| `px-scene`（场景编译器） | `cargo test -p px-scene` | **43 passed**（含新增 2 条） |
| 动了编译器会不会动冻产物 | `art/anchor/hashes.txt` §三 六格重烘对账 | **6/6 逐字节相同** |
| 卫星这条新路 | `px run scene orbit-gasgiant` + 同机位出图 | `[satellite]` 那一行出现在审计里（图元 icosphere、shader gasgiant@8895bbb8ea03、贴图 无） |

⚠ J1 六张的上游没动，未重跑。

## 四、下一轮

* **V14（新记）**：卫星影与环影在 shadow map 分辨率下偏糊（图里那道斜影是一块 smudge）——
  要"更利落的影"就得调 shadow map 的尺寸/柔度（那是渲染器侧的一条）。
* V9（可喂章，用户已选 B）、V13（球面还偏絮状）、V3（极区）、V6（月球地球反照）、
  V7（天王星配色档）都还在表里。
