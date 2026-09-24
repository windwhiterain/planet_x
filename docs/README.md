# Planet X 文档

> **这是文档的唯一入口。** 一份文档只有一个家（`docs/` 之下）—— 旧位置的
> `.agents/notes/` 与 `art-framework.md` 已随这次重排删除，不保留跳转副本：
> 两份真相会漂开，而本仓的第一条铁律就是**单一真源**。

## 这个项目是什么

**一个基于图的程序化内容生成（PCG）+ 渲染引擎，面向 agent 的内容创作。**
不是游戏、不是美术资产库 —— 是那套让 agent 能写出图脚本、烘出星球、再把星球渲出来的机器。

形状是流水线，不是框架：

```text
图脚本（普通 Rust，类型检查最大化的调用链）
   ↓  px_cook::cached —— 图里唯一的那个函数（读缓存，脏了才调算子）
算子（dylib，运行期按**内容身份**装载）
   ↓  产物落内容寻址的 CAS
.pxart 产物
   ↓
渲染器（裸 wgpu 宿主）→ PNG
```

两条性质决定了其余一切：

1. **身份 = 内容。** 算子库、生成物、缓存键全部按内容算。改一行实现 ⇒ 只重编、只重烘那一条。
2. **接口在编译期，实现在运行期。** 图程序编译期就固定算子签名；实现是运行期按 key 装载的
   dylib ⇒ "改一个算子不重编图程序"（R1）。

配套的**探针**（`px_probe`）与**判据目标**（`art/anchor/`）不是附属品 —— 它们是"这个工程
还能不能被验证"的载体。

## 怎么找东西

| 你在做什么 | 去哪 |
|---|---|
| 刚进门、想知道今天盘上成立什么 | [`current.md`](current.md) |
| 要改任何东西之前 | [`invariants.md`](invariants.md) —— 每条都付过代价 |
| 想知道某个决定**为什么**这样 | [`decisions.md`](decisions.md)（含被否掉的路线及理由） |
| 要写一个新的算子库 | [`guides/writing-an-operator-library.md`](guides/writing-an-operator-library.md) |
| 要改 PCG / 代码生成那条流水线 | [`system/`](system/) |
| 要改渲染器 / 探针 / 出图 | [`render/`](render/) |
| 要改视觉（星球、云、天空……） | [`art/`](art/) |
| 想知道某一轮**当时**做了什么、量到什么 | [`rounds/`](rounds/) |
| 想挑一条待办 | [`backlog.md`](backlog.md) |
| 读到了明确过期的记录 | [`archive/`](archive/) |

## 目录地图

```text
docs/
  README.md           ← 你在这里
  current.md          现状：今天盘上成立的口径 + 判据清单
  invariants.md       不变式：改任何东西之前先读
  decisions.md        路线裁决与被否掉的理由
  backlog.md          待办（观感 + 系统两张表，滚动登记）

  system/             PCG 与代码生成流水线
    pcg.md                算子表、图程序、参数、CAS、缓存键、版本失效
    assets.md             产物与协议：`.pxart` 里有什么、四种域、载荷指纹、`diff`
    cached.md             图里只有一个函数：`cached`
    graphs.md             图库按领域拆分：schema / op / 动态链接
    typed-scripts.md      图脚本走类型化算子契约（op_id / 字节边界退场）
    operators.md          算子回到实现库、按身份装载
    generic-instances.md  泛型算子的自动单态化：内容寻址的**代码**缓存
    build-graph.md        build graph：用图组织代码生成，与数据图两阶段顺序执行
    codegen-types.md      stage 2 用的类型 = stage 1 生成出来的（图侧零宏）
    nurbs.md              NURBS 算子库（曲线 / 曲面）
    params.md             "一切皆参数"：删掉画布，相机归场景
    elementwise.md        一个泛型 element-wise 算子：`px_elem`

  render/             渲染器
    renderer.md           宿主怎么跑：离屏 / 常驻 / viewer 的实体分法 + 材质契约
    instruments.md        改完怎么验：热重载、review 回路、单帧时间、探针
    pass-table.md         数据驱动的 pass 执行器（`px_pass`）
    viewer-panel.md       预览窗口里的调参面板（改参数 → 子进程 cook → 画面重载）
    graph-research.md     动态 schema 与动态 render graph 的调研（§67–§78）
    schema-limits.md      改 schema 今天能走到哪儿的实测（§79）

  art/                视觉
    geometry.md   sky.md   clouds.md   gradient.md

  rounds/             开发史，**按日期**命名（22→40 那一批）
    2026-09-20-*.md     一连串观感 / 系统迭代
    2026-09-23-virtual-shadow.md

  archive/            明确过期、只当历史读
    bevy-exit.md        彻底剥离 Bevy 的调研（代码未动）
    render-wgpu.md      裸 wgpu 宿主的开工序（3,371 行，当时那一轮的工单）
    handoff.md          更早的交接（"本轮之前的世界"）

  guides/             面向使用者的操作清单（"怎么写"，不解释"为什么"）
    writing-an-operator-library.md
```

## 命名与引用约定

- **文件名不带序号。** 序号会因为增删一篇而全体重编，而"引用"散在 37 份**参与源码身份
  指纹**的源码里 —— 一次重编号等于把全仓缓存键掀一遍。文件名是**冻住的**：要改名，
  先想清楚它值不值这个代价。
- 开发史按**日期**命名（`rounds/YYYY-MM-DD-<主题>.md`），日期是历史事实、永不重编。
- **正文里的 `§NNN` 是历史锚点，不重编号、不复用。** 它们是《某某节》的引用语，不是文件名；
  换一篇文档不等于换一个节号。遇到裸 § 号，直接在 `docs/` 里搜索。
- 引用一律写**仓库根起的全路径**（`docs/system/operators.md`），不写裸文件名 ——
  裸名在别的目录里指不准。

## 三条命令

```powershell
.\tools\px.ps1 -Task test         # 快速测试链（默认 members，不碰宿主与探针）
.\tools\px.ps1 -Task device       # 最便宜的 GPU 门（约 10 秒）
.\tools\px.ps1 -Task field_dual   # 梯度对错的唯一判据
```
