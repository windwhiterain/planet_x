# 图库按领域拆分：schema / op / 动态链接

> ⚠ 后续（算子改回实现库、运行期按身份装载）：见 `18-operator-libraries.md`。
> 本篇的 §159 是**当时那一轮**的裁决；今天生效的形状以 `18-operator-libraries.md` 为准。

> 起因（2026-09-19，用户裁决）：**把图库（今天的 `px_ops`）按它自己的领域拆开**；每个领域给一对
> `xxx_schema`（数据）与 `xxx_op`（实现），**图脚本依赖 `xxx_schema`、把 `xxx_op` 当动态链接用**；
> **算子之间只通过 `xxx_schema` 里那份序列化数据说话**；**需要单态化的算子**，图脚本自己再建一个
> `xxx_op` crate **并着用**来单态化。
>
> § 号接 `15-render-wgpu.md`（§157/§158）。这一篇是**决议 + 事实 + 判据**，读数随做随记。

---

## §159 这一轮的裁决（用户口径，逐条落成判据）

| # | 口径 | 落成的判据 |
|---|---|---|
| 1 | 图库按领域拆成 `xxx_schema` + `xxx_op` | §159.2 的 crate 名录；§159.5 的门「谁依赖谁」 |
| 2 | 图脚本依赖 `xxx_schema` | 图脚本不许静态依赖任何 `*_op`（§159.5 第 2 道门） |
| 3 | `xxx_op` 是**动态链接** | op 代码不住在图程序 exe 里；§159.4 的 dll 身份对账 |
| 4 | 算子之间**只**走 `xxx_schema` 的序列化数据 | 边界签名上不许出现算子之间的 Rust 类型（§159.3） |
| 5 | **需要单态化的算子**由图自建的 `xxx_op` crate 并着用 | §159.3 后半：什么算「需要单态化」的判据 |
| 6 | dll 身份**不进键**（用户裁决） | 进 `index.json` / 清单，命中时对账（§159.4） |

### §159.1 领域划分的形状（用户口径：「各个领域指的是 graph library 的各个领域」）

今天 `px_ops` **一个 crate 干了四件事**，所以「拆」就是把它们分家：

| 今天住在哪 | 是什么 | 出处 |
|---|---|---|
| `px_ops/src/field.rs` | 场的数据（`Field` / `Stats` / `Projection` / `direction_at` / `tangent_frame`） | 逐行 |
| `px_ops/src/volume.rs` | 体积的参数空间约定（`PATCHES` / `direction_of` / `point_of` / `VolumeSampler` / `VolumeGrid`） | 逐行 |
| `px_ops/src/noise.rs` | 噪声算法 + `Scalar` 数值抽象 + FNV 身份哈希 | 逐行 |
| `px_ops/src/ops/{constant,fbm,ridged,mix,remap,gradient,warp}.rs` | 七个**场**算子 | 逐行 |
| `px_ops/src/ops/cubesphere.rs` | 一个**网格**算子 | 逐行 |
| `px_ops/src/{cameras,generate}.rs` | 评审相机表、程序化资产（色板/立方图/星空/环）——**不是图领域，是烘图侧工具** | 逐行 |
| `px_ops/src/lib.rs:26-68` | 四个算子契约（`FieldOp` / `MeshOp` / `VolumeOp` / `IsosurfaceOp`） | 逐行 |
| `px_ops/src/lib.rs:70-160` | 载荷与清单类型（`Payload` / `Artifact` / `GraphSpec` / `Grid` / `IndexEntry` / `ManifestEntry`） | 逐行 |
| `px_ops/src/lib.rs:162-1210` | **驱动**（`begin` / `node` / `mesh_node` / `volume_node` / `surface_node` / CAS / 参数装载 / 键 / 清单） | 逐行 |
| `px_mc/src/lib.rs` | 等值面算子（网格域，叶子：只认识 `isosurface` 与体积约定） | 逐行 |
| `px_graphs/src/cloud_proxy.rs` | 体积算子 `CoarseVolume` + **判据仪器**（`containment` / `measure_gradient_bound`） | 逐行 |

### §159.2 拆完的 crate 名录（**已落地**）

用户口径两条改了名录的形状：**① 驱动 / 键 / 清单不是算子**（不能留在叫 ops 的 crate 里）；
**② dyn lib 的调用放 schema（契约）那一层**。于是：

```text
px_graph_schema    契约层（叶子）：Key / PayloadBundle / GraphSpec / Grid / IndexEntry / ManifestEntry
                   + canonical_params / node_key / key_with_cameras / payload_fingerprint / hex
                   + fnv1a（身份哈希）+ 算子描述符（OpKind / OpDescriptor / OpTable / OpCall）
                   + **dyn lib 的调用**（OpLibrary：装载 / 取表 / 调 op / dll 指纹 / 找目录）
px_graph           图库本体（静态）：驱动 begin/node/finish + CAS + 参数 + 清单
                   + cameras / generate + shader 写入（SHADER_VERSION / shader_key / write_shader / scene_key）
px_field_schema    场域数据：Field / Stats / Grid 的 helper / Projection / Scalar / FbmSettings
                   + 七个场算子的 Params 与「TOML → 规范 JSON」入口
px_field_op        场域算子：noise 算法 + constant/fbm/ridged/mix/remap/gradient/warp   ← dylib
px_volume_schema   体积域数据：VolumeData / PATCHES / direction_of / point_of / VolumeSampler
                   + CoarseVolume 的 Params / FieldKind
px_volume_op       体积域算子：CoarseVolume（用 `px_verify::proxy` 那份参照场）           ← dylib
px_mesh_schema     网格域数据：MeshData + CubeSphere / ProxySurface 的 Params
px_mesh_op         网格域算子：CubeSphere + ProxySurface（isosurface）                  ← dylib
px_graphs          图脚本：依赖 *_schema + px_graph；运行时装载 *_op；单态化的算子图自建 op crate
```

**`px_ops` / `px_mc` 解散**：算子进三个 `_op`，躯干（驱动/键/清单/CAS/cameras/generate）进 `px_graph`。
**跨域只走 schema**：`px_volume_op` 吃一张**场**（按 `px_field_schema` 序列化）吐一份**体积**
（按 `px_volume_schema`）；`px_mesh_op` 吃**体积**吐**网格**。参照场（`px_verify::proxy`）
算子与判据仪器共用同一份 ⇒ 仪器不必依赖算子。依赖关系：
`px_verify → px_field_schema`、`px_volume_schema`；`px_volume_op → px_verify`；不成环。

**边界签名**（`px_graph_schema::op`）：

```rust
type ParamsCanonical = extern "Rust" fn(&str, Option<&str>) -> Result<String, String>;
type OpCall = extern "Rust" fn(&str, &str, Grid, &[&[u8]]) -> Result<Vec<u8>, String>;
struct OpTable { ops: &'static [OpDescriptor], canonical_params: ParamsCanonical, call: OpCall }
```

入口符号**由库名派生**（`<file stem>_table`，`px_field_op.dll` → `px_field_op_table`）——
固定名字在静态链到一起时（测试 / dev-dependency）会 `LNK2005`，派生名字顺手把「图自建的
op crate」也接上了：编出来放在同一个目录里就会被装载，不必改驱动一行。
图脚本的 API 收敛成一句：`node("field.fbm", "continents", &[])`。

### §159.3 动态边界的契约

- **边界上流过的只有字节**：驱动把上游产物（CAS 里那份**流格式**，就是渲染器要吃的东西）交给
  op，op 回一串字节；op 之间不共享内存里的 Rust 类型（口径 4）。
- **参数也在 schema 里**：每个算子的 `Params` 类型住在它领域的 `*_schema`，键用的**规范 JSON**
  由 schema 那一侧从 TOML 算出来（今天 `canonical_params` 的同一份口径）⇒ **算键不必求值**
  （§17.1 那套「先 key 后 cook」原样保留），op 收到的是同一份 JSON。
- **什么算「需要单态化」**（口径 5 的判据）：**实例的选择权在图脚本手里**。
  动态边界只有类型擦除后的固定签名，泛型实例是图脚本编译期才知道的事 ⇒ 这类算子必须由图自建的
  `xxx_op` crate 单态化。⚠ 用户点名的情形是「**场相关的 op 要能处理各种场函数**」——
  **场函数过不去边界**（边界上只有采样后的场），所以拿场函数当参数的算子只能在图侧单态化。
  ⚠ `px_verify::cloud_field` 那族 `<S: Scalar>`（`density` / `shape` / `billows`）**不是**这一类的判据：
  它的两个实例（`f32`、`Dual`）分别落在 dll 与探针**内部**，选择权不在图脚本手里。

### §159.4 dll 的身份与缓存（用户裁决：不进键）

- `node_key` 一维都不加 ⇒ §17.1 那条「dll 不在 exe 里 ⇒ 键要把 dll 纳入哈希」的盲区**改用**：
  每个算子的 `SOURCE_HASH`（编译期常量，随 dll 过来）与 `VERSION` 进 `index.json`，命中时对账，
  不一致就按 §19.1 那套喊出来；**dll 文件的 blake3 也进 index/清单**（「换的是哪个二进制」可查）。
- 为什么不整包进键：dll 是一个字节序列，改任一算子会让**所有**图的全部节点换键 ——
  正是 §19 当年否掉的「一动全废」；而 per-op 的 `SOURCE_HASH` 粒度到算子、且已经覆盖共享依赖（§28.2）。

### §159.5 判据（逐条对账）

| # | 判据 | 读数 |
|---|---|---|
| 1 | **键不变** | 拆分后 `planet` 6/6、`desert` 8/8、`clouds` 14/14 节点**全部命中**拆分前的 CAS（键未动） |
| 2 | **字节不变**（决定性的一条） | `PX_PCG_FRESH=1` 全量重算后，`target/pcg/ab` 下 **46/46 份产物与基线逐字节相同**（sha256 逐份比对） |
| 3 | **anchor 不变** | 六档老形状场景文件字节 = `hashes.txt` §三 **逐格 ✓**（2795F948E6987E11 / F60B19B0AE7E229F / 1A27679882A10D6B / ACB824E9EC0BA893 / CB363DA74A90F63E / 2C6C8592AD45214B） |
| 4 | **门：图脚本不许静态依赖算子** | `px_graphs/tests/crate_graph.rs` 3 条（`px_graphs` 的 `[dependencies]` 里没有 `*_op`；`px_graph` 没有；四个 schema 都没有） |
| 5 | **门：`SOURCE_HASH` 覆盖共享依赖（§28.2）** | `px_graph/tests/source_hash.rs` 指向新的算子位置：7 个场算子 + `px_mesh_op` 两个 + `px_volume_op` 一个 |
| 6 | **dll 身份对账**（§159.4） | `IndexEntry.dll`（dll 文件 blake3 前 8 字节）+ `SOURCE_HASH` + `VERSION` 命中时三条对账，不一致就喊 |
| 7 | 测试全绿 | 82 个用例、0 失败（基线 74 个；新增两条门、`keys` 挪家） |

⇒ **用户允许「键变、事后重登记 anchor」，而实测是一个都没动** ⇒ anchor **不用重登记**，比裁决允诺的更好。

#### §159.5.1 读数（2026-09-19，本 worktree）

```text
基线（拆分前，冷编）：cargo build -p px_graphs --bins        19.65 s
                     planet 6 节点全量 2348 ms；desert 8 节点 2869 ms；clouds 14 节点 25638 ms
拆分后（全量重算，走 dylib 边界）：
                     planet 6 节点 2514 ms；desert 8 节点 3179 ms；clouds 14 节点 26609 ms
                     ⇒ 动态装载 + 跨 dylib 序列化的代价约 +4~7%（在这些算子上量不出来噪声之外的东西）
命中一路：planet 109 ms / desert 132 ms / clouds 420 ms（与拆分前同一量级）
```

⚠ 这一份副本的 `art/` 字节与参照工作副本**逐字节相同**（robocopy 同步 + `git status art` 干净 +
五份抽检字节数一致）⇒ 上表能复现登记读数的前提成立（`hashes.txt` §六 那条行尾陷阱在这里不咬人）。

### §159.6 这一轮做到哪（分期对账）

| 序 | 那一刀 | 状态 |
|---|---|---|
| S1 | 按领域拆 crate：schema（数据/参数）+ op（算子）+ 图库本体 | **已落** |
| S2 | `*_op` 变 dylib + 驱动按描述符表查算子 + 图脚本改按 id 取 | **已落** |
| S3 | 门：图脚本不许静态依赖算子；`SOURCE_HASH` 门指向新位置 | **已落** |
| S4 | 单态化那一支（图自建 op crate 并着用）+ 一个真实样本 | **没做**：机制已通（入口名按库名派生、目录扫描认 `px_*_op`，图自建的同名库放同一目录即被接上），但今天没有「实例选择权在图脚本手里」的算子（§159.3 后半）⇒ 缺的是**样本**，不是机制 |

> ⚠ **本节之后的一轮改了 §159 的一条口径**（2026-09-19，`feature/typed-graph`，提交 `7feaecf`）：
> 「图脚本不许静态依赖任何 `*_op`」**不再成立** —— 图脚本现在静态链接算子的 **rlib**
> 去拿「参数类型 / 输入个数 / 输出域」的编译期检查（新契约 crate `px_cook`）。
> 替代门与逐字节判据（14/14）见 **`17-typed-ops.md` §162/§163**。
| S5 | 文档指针与旧裁决标注 | **部分**：`.agents/notes/art/02-pcg.md` §15.2 那条「算子静态链、cdylib 留接口不做」**已被这一轮取代**（那一条的理由是「只买到不重启图程序」，没算上「改算子不必重编图程序」与「场函数过不去边界」这两条）；本文件即取代记录 |

### §159.7 还没做 / 明确的代价（别当成已完成）

> ⚠ **本节两条已被后来的两轮推翻（2026-09-20）**：
> ① **S4「单态化那一支没做」**（§159.6 那一行也在此列）：**样本有了**，而且换过两次形状 ——
> `17` §166 有一版"每图自建 `mono-gen`"生成器（**那条原型期的路已删**），今天以
> **`px_inst!` + build graph** 的形式回来了（`19-generic-inst.md` §174–§180、
> `20-build-graph.md` §183/§184），另有图侧现写的 `px_local_op!`（`18` §173）。
> ② **「算子库的开发期热路径没有读数」**：已经有了 —— 改一行实现 ⇒ 图 exe 字节不变、
> 只重编那份库（`18` §171.2 的量法，0.32 s 空转 / 1.09 s 改一行）。
> 下面第一条（跑图前要先编算子库）**仍然成立**（那条是"不重编图程序"的另一面）。

- **跑图之前要先编算子库**：`cargo build -p px_field_op -p px_volume_op -p px_mesh_op`
  （`cargo build` 不带 `-p` 也会编 —— 三个 `_op` 都在 `default-members` 里）。
  ⚠ `cargo run -p px_graphs --bin planet` **不会**自动把算子库编出来（这正是「不重编图程序」的
  另一面）。驱动找不到库时**当场拒**，并把该跑的命令打出来（不许静默什么都不算）。
- **`px_render` 那六张判据图（J1）与 J3 这一轮没重出**：那要 GPU 与 wgpu 栈；
  本轮的判据是键 / 字节 / 冻产物那一档（§159.5 第 1–3 条）。
- **单态化的样本**（S4）与**算子库的开发期热路径**（改了算子只重编那一个 `_op`）都还没有实测读数。

### §159.8 顺手改掉的两处 `art/` 注释：**量过才改**（2026-09-19）

清理文档时碰到两处指向已解散 crate 的注释：`art/scene/orbit.toml:12`（`px_ops::cameras::review()`
→ `px_graph::cameras::review()`）与 `art/clouds/proxy.toml:7`（`px_mc` → `px_mesh_op/src/proxy.rs`）。
`art/` 下的字节是这个仓最敏感的东西（`hashes.txt` §六），所以**先量再改**：

| 量法 | 读数 |
|---|---|
| `scene orbit --no-frame-graph`，改注释前 / 后各跑一遍 | 键 **18b091fc2654 → 18b091fc2654**（未动）；文件字节 **430623F8D4902EE6 → 430623F8D4902EE6**（未动） |
| `clouds` 全量重算（`proxy.toml` 改过）与基线清单逐节点比键 | **14/14 未变** |
| 六档老形状场景重烘 + `target/pcg/ab` 与基线逐份比 | 六格登记值**全中**；**46/46 逐字节相同** |

⇒ 配方 / 参数文件里的**注释不进任何键**（它们不参与编译出来的文档，也不参与 `canonical_params`
的规范化值）。⚠ 但这条**只管注释**：`art/shaders/*.wgsl` 的字节**在**闭包指纹里（§155.4 明令不许改），
那是另一回事。

