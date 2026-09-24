# 调参面板：窗口里改图参数 → 烘 → 画面变

> 2026-09-28（`feature/viewer-gui`）。用户口径逐字是：
>
> 1. **「给 viewer 添加 GUI，让 user 能够动态编辑图参数然后 cook」**；
> 2. 岔路裁决：cook 走 **子进程 `px run`**｜面板用 **egui + egui-wgpu + egui-winit**
>    （不是自写覆盖层）｜编辑默认写 **`target` 下的会话副本**，另有 Save 写回 `art/`。

## §1 形状

```text
窗口（px_render --view [--edit <场景配方名>]）
 ├─ egui 面板（交换链上的第二个 pass）
 │    ├─ 控件表：art/scene/<配方>.toml 引用到的每张图、每个节点、每个标量叶子
 │    ├─ 编辑 → target/pcg/edit/<图>/<节点>.toml（**会话副本**，唯一的编辑落点）
 │    ├─ Save → 改过的那些文件写回 art/（作品的源码；没改的一个字节都不动）
 │    └─ Cook → 后台线程起 `px run`
 │         px run <图> --store target/pcg/edit     （每张图一次，被引用的在前）
 │         px run scene <配方>                      （最后一步；**不带 --store**，见 §4）
 └─ 烘完 → 重读 target/pcg/scene/manifest.json → 换窗口显示的那一份产物
```

**编辑哪份配方**：`--edit` 给了就用它；没给就**问清单**"窗口正在显示的那份产物是哪份配方
烘的"（`target/pcg/scene/manifest.json` 里 `node` + `key` → `cas_path` → 与手上那份逐字比）。
⚠⚠ **不能拿产物路径的文件名当配方名**：CAS 那份叫 `<内容键>.pxart`，与配方名一个字符都不相干
—— 实测踩到过（推出来一串十六进制，然后面板报"读不到那个 .toml"，一句**指错方向**的话）。
⚠ 比较要走 `canonicalize`：盘上那一条可以由调用方写成相对/绝对、`\` 或 `/`，
而清单拼出来的是另一种写法；按字符串比会在"看着像同一个文件"的地方判不中
（第二次踩到：`--show` 推过来的那一条就对不上）。清单不在时按配方自己的 `name` 栏扫；
两条都不中 ⇒ 面板说"给 `--edit` 哪个名"，**不猜**。

面板画在**交换链上**，在 `present.upload` 之后、`frame.present()` 之前。这一条是判据的
一部分，不是实现细节：

```text
render::Session::draw → 回读字节 ─┬→ Present::upload → 交换链 → 屏幕
                                 └→ shot::write_png（--shot）
```

⇒ 面板**不可能**进 `--shot` 那张图，`--image-hash` 那把尺子也一个字节不动。
**S7 那条判据（"窗口 `--shot` 与离线同文档同机位逐字节相同"）因此一个字都不用改。**

## §2 为什么是子进程，不是进程内

`px_render` **只读产物**（`art.rs` 顶上那条）：算节点的是图程序，而"哪条命令能烘这张图"
已经有一个答案（`px run`）。在进程内做就等于把整套 `px_*` 算图栈拖进渲染宿主，
并且**只对 Rust 图程序可行**（图程序将来可能是别的形状）——那是把宿主绑死在今天的实现上。

代价明说：**viewer 必须先编出来 `px` 与目标图 exe**（同一个 `target/<profile>/`，
与它自己并肩）。找不到时面板**当场报"该编什么"**，不静默什么都不做。

## §3 参数目录换指（图侧那一半）

`px_graph::driver` 新增两样，都**不进键**：

* `param_root()`：参数根默认 `art/`，`PX_ART` 可以换指；
* `apply_store_args()` / `args_without_store()`：图程序在 `main` 第一行把 `--store <目录>`
  落成 `PX_ART`，**并把它从命令行上摘掉**。

⚠⚠ **摘掉那一半是必须的**：`scene` 按**位置**读配方名、`passes` 按位置读三个参数
⇒ 不摘的话 `scene --store X orbit` 会去烘一份**叫 `--store` 的配方**（不是报错，是读错东西）。
实测踩到过：`scene -- orbit-bare` 报"不认识的参数 `orbit-bare`"（`px run` 从前只认 `--` 之后）。

`px run` 那一侧同时改了两处：

* `--store <目录>` / `--store=<目录>` 两种写法都收，原样转交给图 exe；
* **不带 `-` 的位置参数也算图参数**（`px run scene orbit-bare`）。这**不放松任何一条**：
  px 自己的开关全部以 `-` 开头 ⇒ "不带 `-` 的东西"不可能是 px 的。

## §4 会话副本与"哪些字节进键"

* 面板开的时候把 `art/<图>/*.toml` **复制**到 `target/pcg/edit/<图>/`；
  已经存在的副本**不覆盖**（一个窗口重开时，上一轮还没保存的编辑还在那儿——那正是"会话"）。
* 编辑口是 **`toml_edit`**（保格式）：`art/**/*.toml` 里的注释是这批作品的说明书，
  `toml::to_string` 会把它们整片抹掉。改一个数只动那一行。
* 场景那一步**不带 `--store`**：`scene` 那张图的参数住在 `art/scene/`，它不在副本里
  ⇒ 带了就会去副本找一份不存在的 `art/scene/`，而 `node_params` 缺文件是**静默走 Default**
  的（症状是"改完图参数之后场景参数全变默认值"）。

### ⚠ 这一轮抓到的真缺陷（判据钉住的）

`Item::is_value()` **对数组也回真**（数组是一种 `Value`）⇒ 先问它的话，
`set_leaf(doc, "light[1]", …)` 会把**整个数组**换成一个标量（实测 `light = 0.5`），
**而没有任何一行报错**。修法：**数组先问**。判据
`array_elements_are_individual_fields`（改之前是红的）。

## §5 判据（本单元实跑）

| 判据 | 读数 |
|---|---|
| 渲染这条**一位没动** | `--offline --scene orbit-bare.pxart --out x.png --width 960 --height 640` ⇒ **`63184151909371A5`、300012 B**（与 `art/anchor/hashes.txt` §一登记值逐字节相同） |
| 改一个参数 ⇒ 只重算该节点与下游 | 副本里 `continents.frequency 0.55 → 0.9` ⇒ `共 6 个节点：**命中 2、重算 4**` |
| 新场景产物的键变了 | `f9029752d997…` → `f9a783115620…` |
| 画面真的变了，而且**变得对** | 改后图 `F0E316C4AE6EE5A5`；`--diff`：差异 200908/614400（32.7%），**剪影外 0 像素**，剪影内亮度相关 **0.969**、最大通道差 149 |
| **目录不进键**（§3 那条断言的判据） | 副本复制回原值 ⇒ 不带 `--store` 与带 `--store` 两趟**都能命中**，场景键回到 **`f9029752d997…`**（逐位相同） |
| `art/` 没被面板改脏 | 编辑只落在副本；`art/planet/continents.toml` 全程 `frequency = 0.55` |
| 测试 | `px_render` 84 passed（含 edit 那 6 条）、`px_graph`/`px_cook`/`px_graphs` 全绿 |

⚠ **没验到的一格（诚实记账）**：本会话**没有可交互桌面** ⇒ 那个 winit 窗口起来之后
很快自己退了（`租约没了 ⇒ 预览窗口退出`），所以**"人眼看着面板长什么样、点起来顺不顺手"
这一格没量**。面板的代码路径（`paint` → `egui_wgpu::Renderer`）只到了编译级 +
它下游那一条（编辑→烘→换画面）的端到端判据。**待人在有桌面的机器上看一眼。**

## §6 顺手碰到的、与本单元无关的一处**陈旧**

`art/nebula/density_volume.toml` 里还是旧字段 **`res_ratio`**，而 `DensityParams` 现在只认
`res` / `layers` / `inner` / `outer` / `reach`（`43-params-not-canvas.md` §2 那条改名的落点）
⇒ **`px run nebula` 在当前 HEAD 上本来就烘不过**（与本次改动无关；`origin/v2` 也一样）。
判据是实测的：`unknown field res_ratio, expected one of res, layers, inner, outer, reach`。

⇒ 面板把这一条**原样显示**在日志区（这正是"cook 一路的输出都要看得见"那条设计的用处），
而这一格**没有替用户改**：改作品的参数不在本单元的授权里。

## §7 用法

```powershell
# 先把面板要烘的东西编出来（同一个 target/<profile>/：px、目标图 exe）
cargo build --release -p px_graphs --bin px --bin scene --bin planet

px_render --view --scene <SCENE.pxart> [--edit orbit-bare] --pcg-root target\pcg
#   Tab / F1 收起/展开面板｜改控件 → Cook｜Save → art/｜重读｜复位（从 art/ 读回）
```

⚠ `--edit` **只对 `--view` 有效**（面板住在窗口里）⇒ 单独给会当场拒
（拒词说的是"这条路不适用"，不是"不认识的参数"）。
⚠ 面板开在哪份配方上是**窗口起来时定的**：之后用 `--show` 推**别的**场景进去，
面板**不跟着换**（"会话"的意思）；换一份编辑面就重开窗口。

## §8 还没做的

* **算子源码改了**（`px build` 那一档）：面板只重跑图，**不跑 stage 1**
  ⇒ 缺实例库时面板显示的是图 exe 的报错（"该跑 `px build`"）。接上来是纯增量：
  在 `cook.rs` 的步骤表前面加一步 `px build`（但那会把"运行只读"那条纪律的边界挪一格，
  得先想清楚谁授权编）。
* **`shaders` 那张图**：面板编辑的是参数（`.toml`），`.wgsl` 走的是既有的 shader 热重载
  （S7 后半，窗口里已经在跑）。
* **面板的持久化**：改的控件状态、面板宽度这些没存盘（每次开窗口都是新的）。
