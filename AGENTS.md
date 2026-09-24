# Planet X

- 禁止查看 main 分支

## 体渲染 / 天空烘焙：以 GPU 为准

- 实现只有一处真源：**GPU**（`px_volume_gpu_op` + `src/sampler.wgsl`）。出图、探针、读数走 GPU。
- **禁止在渲染/出图路径上跑 CPU 实现**；`px_volume_alg` 那一份只当 GPU 实现的**草稿与对账参考**
  （可以存在、可以被测试当 oracle，但**不许**有"算子或图走 CPU 那条"的路，也不许新增只在 CPU 上的功能）。
- 参考步进也用**同一份 WGSL**（关掉被验证的那一项）当基线，不要另写一个 CPU 版本去"对账"。

## 空跳的占用掩码：三处布局必须同源

- 真源一处：`px_volume_gpu_op/src/occupancy.rs` 的 `from_emission`。**内部存储**（`sidecar` 一格一字、
  `words` 每块 `WORDS_PER_BLOCK` 字）与**上传布局**（`upload_words`：`[L1 位][L2 掩码]` 两段）
  不是同一套，读写口各按各的。
- 改布局必须同时改三处：`from_emission`（写）、`at_cell`（按内部布局读）、WGSL 的
  `occupancy_class`（按上传布局读）。这一轮**三次**栽在"其中一处与另两处不一致"上，
  症状都是"掩码看着是对的、skip 侧整片错"：
  1. 用 `cube_face_of` 反查面内参数（另一套面序）⇒ 读到别的面；
  2. `WORDS_PER_BLOCK` 在 WGSL 里硬编码 16、Rust 里算出来是 2 ⇒ 每块的 L2 读到八个块以外；
  3. `upload_words` 把 `sidecar` 按 `index · 32 + bit` 铺位 ⇒ L1 位整体错位。
- ⚠ `WORDS_PER_BLOCK = 4³/32 = 2`：L2 正好占满两个 `u32`，**L1 必须独占一段**，不能塞进
  "每块的第 0 个字"。
- ⚠ 跨粗块边界要**真的跨过去**：`layer_radius` 是 `pow`，`layer_index` 在边界上差一个 ulp
  就会退回上一块 ⇒ 光标原地打转 ⇒ 贴着块缝的那几条视线整条全黑。
- 对账判据：`the_skip_matches_the_dense_march_on_a_blocky_volume`（同一份 WGSL 开/关空跳，
  逐 texel 比）。实测最大 Δ 0.0142（相对 3.9%），来源是两档 quadrature 不同，不是漏气 ——
  数写在 `occupancy.rs` 的模块文档里。
- 实测收益（`px_graphs nebula --face 64`，release）：真实星云 **66.8% 的粗块精确空**
  （3072 里活 1020），冷启一次 13.0 s；天穹那一段约快 14%。
- ⚠ CAS 的键**不含环境变量**：量 `PX_SKIP_OFF` 那一档必须让参数（比如 `steps`）变一下，
  否则第二次直接命中缓存、量到的是 0.9 s 的读盘时间。
