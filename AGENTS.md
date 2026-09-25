# Planet X

**一个基于图的 PCG + 渲染引擎，面向 agent 的内容创作。** 地图与索引在 `docs/README.md`。

- 禁止查看 main 分支

## 文档

- **`docs/` 是文档的唯一根**：进门先读 `docs/README.md`（地图），再读 `docs/model.md`
  （一次运行经过什么），改任何东西之前读 `docs/invariants.md`（每条都付过代价）。
- **所有文档用英文，且只描述当下**。文档不是变更日志：不写日期、不写"从前是"、
  不写"本轮"、不写 `§` 号。历史在 git 里，那是它该待的地方。
- 文档必须与代码同步。发现文档与代码不符 ⇒ **以代码为准，改文档**。
- 讨论与想法：能落成当下事实的写进对应文档；未决的记 `docs/backlog.md`。

## 测试

- **禁止全量测试**，只跑影响到的；不写冗余测试；大计算量的判据转 probe。
- 默认成员里没有 `game`（模拟）、`px_render`（宿主）、`px_probe`（探针）：
  `cargo test` 是快速链，`cargo test --workspace` 才是全量。

## 提交

- **先 `./format.sh` 再提交**（它只跑 `cargo fix` + `cargo fmt`，不删任何东西；
  产物回收是 `px build --gc`）。格式化动到的别的文件也一并提交。
- 提交信息写**为什么**与判据读数，不写流水账。

## 身份与键（改之前先读这一节）

- 键 = **身份（源码字节）+ 内容**。`px_fingerprint` 哈希 crate `src/` 下每个 `.rs` 与 `.wgsl` 的
  **全文**，加上可达的 path 依赖 ⇒ **改一行注释就换键**。
- 进键的 crate 包括 `px_graphs`（`build.rs` 自算指纹，是 `px_local_op!` 节点的身份）与
  `px_elem`（`build.rs` 的声明哈希进实例键）。
- ⚠ `px_cook` / `px_decls` **在 `px_graphs` 的名册里**（那份名册走 `[build-dependencies]`，
  把两者的源码都收了）。它们今天够不到任何键，只因为 `px_graphs/src` 里没有 `px_local_op!`
  （唯一用处是 `px_graphs/tests/local_op.rs`，而测试被名册排除）⇒ **往任何出货的图里加第一个
  local op 的那天起，改 `px_cook` / `px_decls` 一个字节就会换掉那个节点的键。别当它们免费。**
- ⇒ 改任何参与指纹的源码之前，**先数清楚要换多少键**，并准备好重编全部实例库 + 重烘。
- **键变 ≠ 内容变**：身份是键的一半，所以注释改动会换掉所有键而输出一位不动。
  反过来，**"命中缓存"也不能证明内容没变** —— 只有依赖没被重编时才成立。

## 体渲染 / 天空烘焙：以 GPU 为准

- 实现只有一处真源：**GPU**（`px_volume_gpu_op` + `src/sampler.wgsl`）。出图、探针、读数走 GPU。
- **禁止在渲染/出图路径上跑 CPU 实现**；`px_volume_alg` 那一份只当 GPU 实现的**草稿与对账参考**
  （可以存在、可以被测试当 oracle，但**不许**有"算子或图走 CPU 那条"的路，也不许新增只在 CPU 上的功能）。
- 参考步进也用**同一份 WGSL**（关掉被验证的那一项）当基线，不要另写一个 CPU 版本去"对账"。

## 空跳的占用掩码：三处布局必须同源

- 真源一处：`px_volume_gpu_op/src/occupancy.rs` 的 `from_emission`。**内部存储**（`sidecar` 一格一字、
  `words` 每块 `WORDS_PER_BLOCK` 字）与**上传布局**（`upload_words`：`[L1 位][L2 掩码]` 两段）
  不是同一套，读写口各按各的。
- **改布局必须同时改三处**：`from_emission`（写）、`at_cell`（按内部布局读）、WGSL 的
  `occupancy_class`（按上传布局读）。三处里任意一处与另两处不一致，症状都是"掩码看着对、skip 侧整片错"。
- ⚠ 面序**已经统一**：`px_protocol::art::cube_face_of` 与 `grid_coords_of` 与 WGSL 的面序表是**同一张**，
  `px_protocol/tests/cube.rs` 钉着它是 `cube_direction` 的精确逆 ⇒ 面内参数可以直接用它反查。
  别再用"另有一套面序"当理由绕过它。
- ⚠ `WORDS_PER_BLOCK = 4³/32 = 2`：L2 正好占满两个 `u32`，**L1 必须独占一段**，不能塞进
  "每块的第 0 个字"。WGSL 里不许硬编码这个数，必须与 Rust 侧同源。
- ⚠ 跨粗块边界要**真的跨过去**：`layer_radius` 是 `pow`，`layer_index` 在边界上差一个 ulp
  就会退回上一块 ⇒ 光标原地打转 ⇒ 贴着块缝的那几条视线整条全黑。
- ⚠ **单格边界不膨胀**：块级判据是精确零，而采样模板读"本格 + 上一格" ⇒ 空块最后一格的样本
  在下一格是活的时候仍可能有非零真值。掩码**没有**为此膨胀一格 —— 要动它先量一个专门踩这条边界的夹具。
- 对账判据：`the_skip_matches_the_dense_march_on_a_blocky_volume`（同一份 WGSL 开/关空跳，
  逐 texel 比）。实测最大 Δ 0.0142（相对 3.9%），来源是两档 quadrature 不同，不是漏气 ——
  数写在 `occupancy.rs` 的模块文档里。
- 实测收益（`px_graphs nebula --face 64`，release）：真实星云 **66.8% 的粗块精确空**
  （3072 里活 1020），冷启一次 13.0 s；天穹那一段约快 14%。
- ⚠ CAS 的键**不含环境变量**：量 `PX_SKIP_OFF` 那一档必须让参数（比如 `steps`）变一下，
  否则第二次直接命中缓存、量到的是 0.9 s 的读盘时间。
