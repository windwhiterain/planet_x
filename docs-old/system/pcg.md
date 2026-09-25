# PCG 层

> ⚠ 后续（算子改回实现库、运行期按身份装载）：见 `docs/system/operators.md`。

拓扑是 Rust 程序、参数是 TOML 数据、缓存键是手动版本号加内容寻址。

## §14–§16 三版修订里仍然生效的设计
- **分工**（§16.1/§16.2）：拓扑写在 Rust（`px_graphs/src/bin/<图名>.rs`，手写、入库），算子写在 `px_ops`（有类型的纯函数 + `const VERSION` + `cook`/CAS），参数落在 `art/<图名>/<节点名>.toml`。**美术迭代的热路径是参数、不是拓扑**：改参数 **0** 编译，改拓扑 / 加节点 ~0.5 s 重编，加算子 ~0.5 s 重编。`px_ops` 与 `px_graphs` 分开（算子稳定、图多变）；**加一张图 = 多一个文件，不动 `Cargo.toml`**（`src/bin/*.rs` 由 cargo 自动发现），所有 bin 共享 workspace 的 target 目录（增量缓存全部复用）。
- **不引入通用脚本语言**（§14.1/§15.4）：图是数据、算子是 Rust、参数是数据。「参数表达式」（`ch("../ridge/scale")*2` 那类）只需要一个**极小的带类型 AST 求值器**（约 200 行），**不是一门语言**；逐元素片段先用算子组合覆盖（map / remap / blur / scatter / warp）。⚠️ 真要片段，**必须是纯函数**（不碰 IO、时间、全局），否则缓存与确定性会崩。
- **结构灵活性靠参数驱动的循环**（§16.4）：`for layer in 0..params.layers { … }` ⇒ 参数文件里改一个整数就改「拓扑」，零重编。真正需要改 Rust 的是出现了代码里没有的新算子组合 —— 那是结构设计，本来就该付一次 0.5 s。
  > ⚠ **§159 取代（2026-09-19，`docs/system/graphs.md`）**：cdylib **做了**，理由不是「不重启图程序」——
  > 是**改一个算子不必重编图程序**，以及用户点名的另一条：**场函数过不去类型擦除的边界**
  > （要拿场函数当参数的算子只能在图脚本那一侧单态化）。形态是每域一个 `px_*_op` dylib
  > + 描述符表 + `extern "Rust"` 入口，**入口名按库名派生**（避开 `LNK2005`，也让图自建的
  > op crate 自动被接上）。读数是 46/46 份产物逐字节不变、六份冻产物逐格不变。
  > ⚠ 就地更正（2026-09-20）：这条注记里的 **"描述符表"已在 `19`/`20` 那几轮删掉**
  > （今天入口是 `PxOp::LIB` / `PxOp::SYMBOL` 两个编译期常量，`17` §162；泛型实例那一档见
  > `docs/system/generic-instances.md` + `docs/system/build-graph.md`）。"入口名按库名派生"仍然成立。
- **子图去重是免费的**（§16.5）：两个节点只要 `op_id + 参数 + 输入` 相同就**共享同一份产物**，公共子表达式自动只算一次，不需要 CSE 优化器 —— 这是键设计的副产品。
- **放弃了什么（要认）**（§16.6）：**图不再是可读数据** —— 没有图编辑器、没有「列出所有节点」、没有 DAG 可视化，工具只能 grep Rust；**「美术自己拖节点」不在路线图上**。结构的复现性 = 二进制 + 参数 + 输入 ⇒ 产物里要记 **git rev + 协议指纹**。
- **缓存骨架与语言无关，全部保留**（§14.2）：① 币种就是协议里的 `ArtBundle`（节点输出 = `AssetKind` + params + `Blob`，**缓存里存的就是渲染器要吃的东西**，中间不需要转换层）；② key 是纯函数，**不含路径 / 时间 / pid / 主机名**，文件只贡献内容哈希、不贡献路径（挪文件不失效）；③ **「脏」不是一个状态，而是「key 不在 CAS 里」**（没有要维护的脏标记、不怕重启、缓存可共享）；④ 两遍走：先 key 后 cook，算 key 不需要求值 ⇒ 改一个叶子参数 = O(深度) 次哈希、上游命中是自动的；⑤ 磁盘 CAS 分片 `target/pcg/<key 前两位>/<key>.pxart`；⑥ 确定性：单线程 cook 起步，将来并行必须保证「并行不改变结果」（纯算子 + 确定性归约），不引时间戳。
- **算子静态链、后端按算子声明**（§15.2/§15.6）：算子默认**静态链进 `px_ops`**，cdylib 留接口不做（它唯一买到的是「不重启图程序」，而图程序本来就是一次 ~0.5 s 的 CLI 运行，换了还多出加载期失败模式）；⚠️ 真启用时 `px_ops` 的函数签名不变、只是按路径加载，而 **dll 不在 exe 里 ⇒ 键要把 dll 一并纳入哈希**。**算子只在一个进程内被调用，跨进程流动的是产物（`ArtBundle`）不是算子。** CPU/GPU **不搞全局统一，按算子声明后端**（同一概念可以有两个实现：`field.noise.cpu` / `field.noise.gpu`），只有两边都被用到时才需要一致性检查，否则明确标注哪边权威；烘焙路径（贵、可缓存）与实时路径（参数要能拖）分开，对应 §3 的「CPU bake 权威、GPU bake 只做缓存」。

## §17 PCG 定稿：一节点一文件，CAS 放 `target/pcg/`
- 三条裁决：CAS 放 `target/pcg/`；**一节点一文件**；缓存的哈希包含二进制的信息（**第三条被 §19 取代**）。能用机制保证的，就不靠自觉。

### §17.1 键 = 内容
```

> ⚠ **2026 更正（原型期的大刀阔斧，见 `docs/guides/writing-an-operator-library.md` §3.7）**：
> 本节以及下面 §「node_key = …」几处的键口径**都过期了**。现在：
> ```text
> node_key = blake3("px_pcg/v2" ‖ op_id ‖ 接口哈希 ‖ 规范参数 ‖ [各输入的 key])
> ```
> * `op_version` → **接口形状哈希**（从 `Params`/`Inputs`/`Payload` 三个类型名推，不用人升）
> * `graph_version` **删掉** —— 它是图的属性；改图脚本里别处一行代码不该掀掉某个节点的产物
> * **画布按域**进键（`Payload::RESOLUTION_IS_CANVAS`：场 true、体积/网格 false），
>   不再是无条件掺
> * 投影不进键（编解码口径由 `Payload` 承担）
> * 那条「源码变了但版本没升」的告警**删掉**：源码指纹（`build.rs` 逐源码树算）直接进键，
>   那种陈旧命中不可能发生
> * `GRAPH_VERSION` / `SOURCE_HASH` 两个常量已从所有图脚本删除

node_key = blake3("px_pcg/v1" ‖ op_id ‖ op_version ‖ graph_version
                  ‖ canvas.(W,H) ‖ projection.name ‖ 规范化参数 JSON ‖ [各输入的 key])
```
- **凡是影响产物内容的都必须在键里**，否则 CAS 会出现「同一个键、不同内容」—— `px_ops/src/lib.rs` 里 `node_key` 的注释钉的就是这条。已知进键的：画布尺寸（`docs/system/assets.md` §25.2）、投影 `GraphSpec.projection`、相机表（`key_with_cameras`，§48）、`graph_version`。
- **shader 这一档多一维：include 闭包**（`docs/render/renderer.md` §52.3 的那条，2026-09 修的）：
  ```
  shader_key = blake3("px_shader/v2" ‖ SHADER_VERSION ‖ 闭包指纹 ‖ WGSL 字节)
  闭包指纹  = FNV(sorted[(模块名, 模块源码)] + sorted[外部符号子句])        // px_shader
  ```
  病根：入口 `art/shaders/*.wgsl` 里的 `#import planet_x::*` 由 **naga_oil 在运行期**组装，模块真本住
  `px_render/assets/shaders/*.wgsl` ⇒ 只哈希入口文本时，**改一个 include 什么都不动**：键不动、清单不动、
  场景键不动、槽版本不动，而画出来的东西变了（同一个键、不同内容）。而且 `write_shader` 是无条件覆写，
  `PX_PCG_FRESH=1` 也救不了。
  规则只有一份实现（叶子 crate **`px_shader`**，烘图侧/运行期/门都用它）：只收**可达**模块（改
  `surface.wgsl` 不该连带重烘 `clouds`），外部符号（`bevy_pbr::…`）**只记名字** —— 它们的实现由 Bevy /
  naga_oil 的版本决定，归 `SHADER_VERSION` 手动那一档（§19.1 的口径）。
- **引擎指纹不是「运行时哈希正在跑的 exe」**（那条口径已由 §19 作废）：任何一次重链接都会让整个缓存失效。现在是每算子 `const SOURCE_HASH: u64 = fnv1a_sources(&[include_str!("<算子>.rs"), include_str!("../field.rs"), include_str!("../noise.rs")])` + 手动 `op_version`（依赖也进哈希，见 §28.2）。
- ⚠️ 已知盲区：**若将来启用 cdylib，dll 不在 exe 里**，那时要把 dll 文件一并纳入哈希。
  > ⚠ **§159.4 取代（2026-09-19）**：cdylib 启用了，但 dll 指纹**不进键**（用户裁决）——
  > 整包进键就是 §19 当年否掉的「一动全废」（改任一算子 ⇒ 所有图的全部节点换键）。
  > 落点改成：**每个算子的 `SOURCE_HASH`（编译期常量，随 dll 过来）+ `VERSION` + dll 文件
  > blake3 前 8 字节**一起进 `index.json`，命中时三条对账、不一致就按 §19.1 那套喊。
  > 实测：拆分 + 全量重算后 46/46 份产物**逐字节不变**、六份冻产物文件字节逐格不变
  > ⇒ 键一个都没动，anchor 不用重登记。

### §17.2 参数：一节点一文件
```
art/<图名>/<节点名>.toml        ← 节点 id = 文件名词干
```
- 图程序按键取参数：`let p = px_ops::params::<MixParams>("mix")?;`（文件缺失 ⇒ 用默认值）。**每个节点的参数独立成文件** ⇒ 改一个节点的参数只动一个小文件，diff 干净、可并行改。
- **参数哈希用的是「规范化后的值」，不是文件原文** ⇒ 改注释、调格式**不会**让缓存失效（serde → 键序固定的 JSON → 哈希，仓库里已有这个模式）。**节点名不进 key**（只进清单）⇒ 两个节点 `op_id + 参数 + 输入` 相同就共享产物。

### §17.3 清单（给 agent 看的）
- 每次 cook 写 `target/pcg/manifest.json`：节点名 → key、命中 / 未命中、耗时、产物大小。**这是「图上发生了什么」的唯一可读出口**（图不再是可读数据），也是 agent 判断「这次改动到底重算了哪几个节点」的依据。

### §17.4 分阶段（定稿）与 §17.5 待裁决的落点
- P2a：`px_ops`（真算子 + blake3 CAS）+ `px_graphs/src/bin/<图>.rs` + `art/<图>/<节点>.toml` + manifest 打印；P2b 参数驱动的循环（§16.4）；P2c 节点产物 → 常驻渲染服务出预览图；P2d 算子后端声明（CPU / GPU，§15.6）。P2a 已落地（§20）、P2c 已落地（§21）。（P2a 里「`build.rs` 算 `engine_hash`」一条被 §19 取代：改为算子自带 `VERSION` + `SOURCE_HASH` 兜底。）
- 落点：参数文件格式定为 **TOML**（一节点一文件 `art/<图名>/<节点名>.toml`），`.pxg` 不再承载图；「`.pxp` 还是 RON / TOML」之争到此结束。

## §19 缓存失效：手动版本号（§17.1 的二进制哈希方案作废）
```
node_key = blake3( op_id ‖ op_version ‖ 规范化参数值 ‖ [各输入的 key] ‖ graph_version )
```
- 为什么比「哈希二进制」好（记下来免得将来有人改回去）：**失效范围更细**（改算子 A 不该让用算子 B 的节点重烘；二进制哈希是「一动全废」）；**经得起重构**（改注释、换文件名、纯重构不影响语义，本来就不该失效）；**语义由人声明**（版本号说的是「这个算子的输出语义变了」，不是「这个文件被碰过」）。
- 代价（必须配兜底，否则等于把静默错误请回来）：忘了升版本 ⇒ 旧缓存被命中 ⇒ **画面悄悄不对，而且没有任何报错**。

### §19.1 兜底：源码变而版本没变，就在命中时喊出来
每个算子声明三样：
```rust
pub const ID: &str = "field.fbm";
pub const VERSION: u32 = 3;
pub const SOURCE_HASH: u64 = fnv1a(include_str!("fbm.rs"));   // 编译期算好，自动跟着文件走
```
- ⚠️ **`SOURCE_HASH` 不进键**（进键就等于自动失效，手动版本号就没意义了）—— 它只进**产物元数据**，用于命中时比对。命中时若版本一致而 `SOURCE_HASH` 不一致：
```
⚠ field.fbm 的源码变了但 VERSION 仍是 3；若输出语义变了请升版本并 PX_PCG_FRESH=1 重烘
```
- ⇒ **不需要 CI、不需要锁文件、不需要提交时的纪律**；告警恰好出现在「即将发生陈旧命中」的那一刻。确认是纯注释改动就忽略；`PX_PCG_FRESH=1` 永远保留作逃生门。（`fnv1a` 写成 `const fn` 即可在编译期算；用 FNV 而不是 blake3 是因为它只做变更检测、不做安全。）

### §19.2 图程序自己也要版本号
- 不再哈希二进制 ⇒ 图程序里写死的常量（`field::mix(&a, &b, 0.35)` 的 `0.35`）**重新变成缓存盲区**。所以 `const GRAPH_VERSION: u32 = 7;` 写在图程序源码里、进键，同样受告警保护。⇒ 规则收敛成一句：**每个算子一个版本号、每张图一个版本号；源码变了而版本没变，会被喊出来。**

### §19.3 待确认
- ⏳ **告警要不要升级成门**（把「源码变了但版本没升」做成 `check-pcg-versions` 失败）：倾向**先只告警** —— 门在纯重构时会误报，而误报的门最后会被人绕过去。

## §20 P2a 落地：PCG 层的最终形状
> ⚠ **§159 取代（2026-09-19，`docs/system/graphs.md`）**：`px_ops` / `px_mc` 两个 crate **已解散**，
> 按领域拆成 `px_graph_schema`（契约与装载）、`px_graph`（库本体：驱动/CAS/参数/清单/cameras/generate、
> **静态**）、`px_{field,volume,mesh}_schema`（各域数据）+ `px_{field,volume,mesh}_op`（各域算子，**dylib**）。
> 图脚本从 `node::<ops::fbm::Fbm>("clusters", &[])`（泛型、每个算子在图程序里单态化一份）
> 改成 `node("field.fbm", "clusters", &[])`（按 **id** 取，不按类型取）。下面这一段是**当时的形状**，读数一字不删。
- `px_ops`：`FieldOp` trait / `Field` / 值噪声 + fbm + ridged / `node::<Op>(名字, 输入)` / blake3 CAS / `index.json` + `manifest.json` / 版本不匹配告警。`node::<Op>(名字, 输入)` 是**泛型函数、不是宏** —— 算子把 `ID / VERSION / SOURCE_HASH / INPUTS / eval` 放进一个 `impl FieldOp for X`，「算子表」就是 trait 实现、编译期解析 ⇒ **框架语法为零**，图程序读起来就是普通 Rust。
- 算子 5 个、**一算子一文件**、各带自己的 ID / VERSION / SOURCE_HASH：`field.constant`、`field.fbm`、`field.ridged`、`field.mix`（3 输入 a/b/mask）、`field.remap`。
- 图：`px_graphs/src/bin/planet.rs`（手写、带 `GRAPH_VERSION`），节点 continents / mountains / weight → terrain → height。参数：`art/planet/<节点>.toml`（一节点一文件，文件缺失即用默认值）。用法：`cargo run -p px_graphs --bin planet`。
- 缓存：`target/pcg/ab/<前两位>/<blake3>.pxart`（**内容就是 `px_protocol` 的流格式**）+ `index.json` + `manifest.json`。
- 实测：5 个节点全量 cook（384×192，6 阶 fbm + 5 阶 ridged）**52 ms**；全命中 **10 ms**；改一行 `px_ops` 重编（不链 Bevy）**0.5 s 量级**。⚠️ 这个规模下**缓存买不到速度**（全量也才 52 ms），它买的是**边界** —— `cook` 的接口、参数文件的约定、产物格式；等要烘的东西变贵（高分辨率场、网格、侵蚀模拟）时才回本。
- **键是纯函数**：把参数改回去，键就回到原值、缓存自动命中 —— **不需要任何「撤销 / 失效」逻辑**。验收口径（`cargo test --workspace` 133 个全绿，六条）：改一个参数 ⇒ 命中 continents / weight、重算 mountains → terrain → height；只改注释 ⇒ 全部命中、键不变；`PX_PCG_FRESH=1` ⇒ 全部重算、且重算出的值域与缓存里的完全一致；改算子源码但没升 VERSION ⇒ ⚠ 精确点名 continents、缓存照常命中；升 VERSION ⇒ 只有该算子与其下游重算（continents + terrain + height）；参数改回去后再全量重算 ⇒ **11 个产物逐字节不变**。
- 还没做：**只有 `Field` 一种产物类型**（Mesh / Instances 只在协议里有类型声明）；图程序与 sim 还没接（拿 `WorldView` 当输入仍未做）；输入个数是常量（`INPUTS` 是 `&'static [&'static str]`，可变输入要等真有需求）；噪声是**自己写的值噪声**，与另一条血缘的 GLSL（perlin / fbm / ridged / warp）**还没对过**（§14.5 第 4 条）。

## §21 P2c：PCG → 渲染器这条通路
```
art/planet/<节点>.toml          改这里（数据，不重编）
   ↓  cargo run -p px_graphs --bin planet          命中时 10 ms、全算 52 ms
target/pcg/ab/<xx>/<blake3>.pxart                 高度场（px_protocol 流格式）
   ↓  px_render --planet <该文件> --palette rocky --out shot.png     0.34 s
PNG（960×640，星空背景 + 位移球体 + 明暗界线）
```
- **渲染器不依赖 `px_ops`**：它用 `px_protocol` 自己解 `.pxart` —— 缓存里存的就是渲染器要吃的东西，两个 crate 之间零转换层。
```bash
cargo run -p px_graphs --bin planet          # 烘高度场
target/debug/px_render --serve               # 常驻服务（一次预热，之后每张图 ~0.34 s）
target/debug/px_render --planet target/pcg/ab/xx/yy.pxart --palette rocky --out target/planet.png
```
- 四个色板：`rocky`（海洋 + 陆地渐变 + 沙滩 + 雪线）/ `gas`（条带）/ `ice`（冰海 + 裂纹）/ `lava`（暗岩 + 自发光裂缝）；`--displace / --sea / --radius / --spin` 可覆盖各色板的默认值。**全链路（改一个参数 → 烘图 → 出图）< 1 s。**
- 三条仍然生效的约定（都属于「看着像那么回事、其实是错的」）：Bevy 的 UV 球**极轴在 ±Z、不在 ±Y**（`Sphere::mesh().uv()` 里 `z = radius * sin(stack_angle)`）⇒ 要 `Quat::from_rotation_x(-FRAC_PI_2)` 立起极轴，否则相机正对北极；`emissive_texture: None` 时 **`emissive` 必须为 0**（**缺失的自发光贴图被当白色** ⇒ 整个物体均匀发白光、与光照无关、调光照没反应）；着色必须按海平面分成水体 / 陆地两段，并让 `height < sea` 时取 `sea`，海岸线才干净（`--sea` 必须同时被位移与着色用，否则命名与实际不符）。
- **诊断方法论**（延续 §12.2）：**在图上猜三次，不如打一行统计** —— 每个会「看起来不对」的中间产物都该有一行可打印的统计（平均 RGB / 近白像素占比那一类）。

## §22 把「各种星球」做宽
- 新增 `field.warp`（域扭曲：用第二个场偏移采样坐标，双线性 + 经度环绕，把笔直的 ridged 脊线变成**蛇曲峡谷**）；第二个图 `px_graphs/src/bin/desert.rs`（台地 `fbm` + 峡谷 `ridged` → `warp` 扭曲 → `mix` 切入 → `remap`，参数在 `art/desert/*.toml`）；环形系统（径向条带 + 卡西尼缝 + 透明度，与星球一起倾斜 **19°**，深度测试正确：环在星球前挡住、在后被遮）；`desert` 色板（沙丘 / 盆地两段 + 层理条纹）。
- **色板移出协议**：`Scene::Planet.palette` 由枚举改成**字符串** —— 色板是美术内容、不是 wire schema ⇒ **以后加色板不再动协议**（省掉每次加色板都要升 `SCHEMA` + 重录流）。**加一张图仍然不用改 `Cargo.toml`**（`src/bin/*.rs` 自动发现）。
- 五种可区分的星球：`rocky`（海洋 / 陆地 / 雪线）、`gas`（条带 + 风暴斑 + 环）、`ice`（冰海 + 裂纹）、`lava`（暗岩 + 自发光裂缝）、`desert`（峡谷 + 沙丘）。

### §22.1 热路径实测

| | 时间 |
|---|---|
| 直接跑烘图二进制、全部命中 | **0.06 s** |
| 出图（服务已热） | **0.34 s** |
| **整轮：改一个参数 → 拿到一张图** | **0.49 s** |
| 同样两步但走 `cargo run` | 1.67 s（其中 **1.2 s 是 cargo 自己的启动开销**） |

- ⇒ 「参数迭代走缓存热路径、单张图 < 1 s」达标。⚠️ 迭代时应直接跑 `target/debug/planet.exe`，走 `cargo run` 会白花 1.2 s。

### §22.2 已知瑕疵（没修，别当成已完成）
- 环**没有投在星球上的阴影**。
- 沙漠色板偏灰，沙与峡谷的对比还不够「沙漠」。
- 噪声仍是自写的值噪声，与另一条血缘的 GLSL 没对过（§14.5 第 4 条）。
