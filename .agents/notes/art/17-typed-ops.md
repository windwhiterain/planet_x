# 图脚本走「类型化算子契约」：op_id/字节边界换成普通 Rust 调用链

> 起因（2026-09-19，用户口径）：**「不显式建图，只是普通 Rust 逻辑加缓存辅助函数，类型检查最大化」**。
> § 号接 `16-graph-split.md`（§159/§160 是拆分那一轮）。
> 这一篇是**决议 + 事实 + 读数 + 未做**，代码已落（提交 `7feaecf`，分支 `feature/typed-graph`）。

---

## §161 先钉三条**测量**出来的事实（它们改了设计）

这一轮开头我按「静态链算子 + 类型化调用」画了一版设计，**量了三组读数之后推翻了它的一半**。
读数是这一轮的起点，所以放在最前面（同一 worktree、`dev` profile、Windows/MSVC、rustc 1.97.1）：

| # | 量法 | 读数 | 它推翻了什么 |
|---|---|---|---|
| 1 | 冷编全套（三个算子库 + 图程序） | **19.0 s** | 「编译时间」这条线的收益上限**很低** —— 为它做复杂机制不划算 |
| 2 | `cargo build -p px_graphs` 空转 | **0.16 s** | 基线 |
| 3 | 改 `px_field_op` 的算子体（新增 `pub fn`、真改 MIR）→ `cargo build -p px_graphs` | **0.16 s，什么都没重编** | ⚠ **「改算子不重编图程序」原本就是真的** —— 因为 `px_field_op` 只住在 `px_graphs` 的 `[dev-dependencies]` |

⚠ 第 3 条最要紧：我原计划把算子做成图脚本的**普通依赖**，那会**把这个性质弄丢**。
所以设计改成：**算子体留 dylib；图脚本静态链接的只是"契约"那一层（没有实现）**。

### §161.1 一条方法上的坑（别浪费时间去撞）

**rustc 的指纹是按内容（MIR）的，不是按文件字节**：
- 加一个没人调用的 `pub fn` ⇒ 什么都没重编（0.15 s）；
- `0.0 * 1.0` 被常量折叠 ⇒ 也没重编；
- 只有改了**会留进机器码**的东西才触发。

⇒ 量增量编译时，"追加个注释/死函数"这种量法是**空转**（[同一现象在社区也被独立发现过](https://trigodil.github.io/blog/rust-compiler-build-times.html)：
作者一开始用"追加注释"量，报出的 15% 全是噪声）。

---

## §162 决议：契约在 `px_cook`，实现在 `px_*_op`，两者分家

```text
px_protocol          线格式
     ↓
px_graph_schema      契约层（Key / PayloadBundle / 描述符 / OpLibrary）
     ↓
px_*_schema          各领域的数据与参数
     ↓
px_cook        ★新   类型化契约：Op trait（Params / Inputs / Payload）+ 缓存辅助函数
     ↓                ⚠ 一个算子实现都不依赖 —— 它只认识 px_graph 的 Cache 接口
px_*_op              各领域算子：**实现**（dylib 那一半）+ **typed 接线**（rlib 那一半）
     ↓
px_graph             图库本体：新增 `Cache` trait + `Driver` 实现（查/写 CAS、清单、读数）
     ↓
px_graphs            图脚本：普通 Rust —— `cook_field::<field::Fbm>(&cache, "clusters", (), grid)?`
```

**两条不许破的线**（`px_graphs/tests/crate_graph.rs` 看住）：

1. **动态装载的那一半（`crate-type` 含 dylib/cdylib）不许依赖 `px_graph`** ——
   否则运行时装载进图程序会让**同一个进程里出现两份驱动**（CAS / 清单 / 索引各一份）。
   ⚠ 这是替代老门的那一条：老门禁的是"图脚本静态依赖算子"，而图脚本现在**必须**静态依赖算子
   （那是拿回类型检查的手段）。
2. **`px_graph` 仍然不许静态依赖任何算子**：它只认 `Cache` 那几个方法。

### §162.1 图脚本长什么样（老 vs 新）

```rust
// 老（§159 的形状）：字符串 id + 字节边界，参数类型/输入个数/输出域都是运行期的事
let clusters = node(field_params::FBM, "clusters", &[]);
let mixed    = node(field_params::MIX, "mixed", &[&clusters, &carved, &weight]);

// 新：普通 Rust —— 类型检查拿回来了，缓存藏在辅助函数里
let clusters = cook_field::<field::Fbm>(&cache, "clusters", (), canvas)?;
let mixed    = cook_field::<field::Mix>(&cache, "mixed", &[&clusters, &carved, &weight], canvas)?;
let coarse   = cook_volume::<volume::CloudCoarse>(&cache, "coarse", &mixed, canvas)?;
let proxy    = cook_mesh::<mesh::Proxy>(&cache, "proxy", &coarse, canvas)?;
```

拿回来的三样：**参数类型**（`Params` 是具体类型，写错字段编译不过）、
**输入个数**（`Inputs<'a>` 里写死，接错编不过）、**输出域**（`Payload = Field`，
把它喂给吃体积的算子编不过）。

### §162.2 键多了一维：**源码哈希进键**

`px_cook::cook_key` 与 `px_graph_schema::node_key` 的差别只有一项：
`Identity.source_hash`（编译期常量，`include_str!` 的 FNV）**也进键**。

理由：类型化这条路里，算子语义可能随源码变而 `VERSION` 没升 ⇒ 那样缓存会**静默给旧产物**。
进了键就是"必然重算"，而 `VERSION` 仍旧只用于打告警。
⚠ 老路径（`node_key`）**一位未动** ⇒ 老图的老键全部照旧命中。
⚠ 相机那一档照旧：产物里带着相机表 ⇒ 相机进键（`key_with_cameras`），体积不进。

---

## §163 判据（这一轮的硬读数）

### §163.1 产物**逐字节相同**：14/14 ✓（决定性的一条）

做法：同一 worktree，先跑新路径存清单，再 `git stash` 回老路径跑一遍，逐节点对比产物**去掉清单帧里节点名之后**的字节。

```text
  billows       1574081 B  sha 628affb25d = 628affb25d   [逐字节相同]
  carved        1574080 B  sha 48fb6aa1ec = 48fb6aa1ec   [逐字节相同]
  clusters      1574082 B  sha 9247318ba3 = 9247318ba3   [逐字节相同]
  coarse        6591297 B  sha b65e7f630f = b65e7f630f   [逐字节相同]
  coarse_fine  25960267 B  sha 8588bde6a9 = 8588bde6a9   [逐字节相同]
  coverage      1574081 B  sha c387437b33 = c387437b33   [逐字节相同]
  flow          1574078 B  sha 3b00266d12 = 3b00266d12   [逐字节相同]
  mixed         1574077 B  sha b3b4a7cc92 = b3b4a7cc92   [逐字节相同]
  proxy         1639947 B  sha 1594399211 = 1594399211   [逐字节相同]
  proxy_fine   14774239 B  sha f7f6b04f07 = f7f6b04f07   [逐字节相同]
  slope_x       1574080 B  sha 931fde9a1e = 931fde9a1e   [逐字节相同]
  slope_y       1574079 B  sha c8c047422e = c8c047422e   [逐字节相同]
  slope_z       1574081 B  sha 53320a83cf = 53320a83cf   [逐字节相同]
  weight        1574079 B  sha 3fe565a10e = 3fe565a10e   [逐字节相同]

⇒ 14/14 逐字节相同
```

⚠ 这条**抓到过一个真缺陷**，记在这里（它是这一轮最值钱的一处发现）：

> 算子回的是**无名、无相机**的占位载荷 —— `bundle.placeholder()`。
> 「节点名 + 相机表」是**驱动**在写盘时补的（老路径那句 `bundle.to_bytes(name, &cameras)`）。
> 新路径第一版直接把算子回的字节写盘了 ⇒ 产物里 `id` 空、相机空。
> 逐字节对账当场抓到（差异正好是**节点名长度 + 相机表大小**）。
> 修在 `Driver::store` 里：先 `bundling(...)` 补成产物，再写、再记清单。

**这一条说明"逐字节对账"值得当规矩**：它抓到的不是浮点差异，是**协议语义漏了一步**。

### §163.2 判据（判据本身）照旧跑通

`check("coarse", ...)` 与 `check("coarse_fine", ...)` 两条：
包住判据（212 / 127 条有交点的方向，漏 0 条）、梯度上界（140.671 ≤ scale 240、162.550 ≤ 1024）、
场网格规模（6 面 × 65² × 65 层、6 面 × 129² × 65 层）—— 与老路径**同一组数**。

### §163.3 测试

`cargo test`（默认 members）：**51 个测试目标、180 个用例通过、0 失败**。
其中 `px_graphs/tests/crate_graph.rs` 改成 4 条（新口径，见 §162）。

### §163.4 增量编译**代价**（如实记，这条是退步）

| 改动 | 老路径 | 新路径 |
|---|---|---|
| 空转 | 0.16 s | 0.29 s |
| **改算子体**（`px_field_op/src/ops/fbm.rs`） | **0.16 s（什么都不重编）** | **3.5 s（重编 `px_field_op` + `px_graphs` + 重链）** |

⇒ **这一刀把「改算子不重编图程序」那条性质换成了类型检查**。冷编 19 s、改算子 3.5 s，
都在可接受范围；但要把这条性质**也**拿回来，唯一的路是**把单态化实例放进独立的 dylib**
（§164 的第二条），那需要生成器。⚠ 按 §161 的读数，这件事的收益上限不高，**别先做**。

---

## §164 泛型方法：`bake<F: FieldFn>`（这一轮的原始目标）

**要的是泛型方法，不是 `dyn`。** 落成的形状：

```rust
// px_cook/src/field_fn.rs —— 算子唯一需要的那个抽象
pub type CoverCloud = px_verify::cloud_field::CloudFieldParams;
pub trait FieldFn {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32;
}
pub struct SampleField<'a> { pub field: &'a Field }     // 上游采样场当函数（老路径）
pub struct ClosedForm<F> { pub f: F }                    // 闭包：图脚本现算

// px_volume_op/src/lib.rs —— 泛型方法
pub fn bake<F: FieldFn>(params: &params::Params, cover: &F) -> VolumeData
pub fn eval_sampled(params, &Field) -> VolumeData        // = bake::<SampleField>
pub fn eval_closed<F: FieldFn>(params, &F) -> VolumeData // = bake::<F>
```

⚠ **一处接口设计上的坑**（记下来，别重犯）：第一版把云参数放进 `SampleField` 的字段里，
结果是「`bake` 自己建云、而场函数也要云」——**同一个东西建两遍/借两次**。
正确的分法是：**算子建上下文、当参数交给场函数**（`cover(&cloud, direction)`）。
这样「云参数怎么算」只有一处（`proxy::from_volume`），闭式场不必自己再推一遍
`to_local` / `cover_from_mask` 的口径。

### §164.1 判据

* **默认路径逐字节不变：14/14 ✓**（`PX_PCG_FRESH=1` 强制重算后对账，同 §163.1 的方法）。
  泛型化**没有**动语义 —— `SampleField::cover` 就是转发到 `proxy::cover_at`。
* **闭式路径能跑**（`cargo run -p px_graphs --bin clouds -- --closed-cover`）：

```text
覆盖度对账（2000 条方向）：老路（回采混合场）0.0000..0.8479｜闭式路（图上现算）0.0000..1.0000
  两路最大差 +1.0000 —— 两条路问的是**不同的场函数**，这个差就是「换了一个覆盖度」的代价，不是误差
  ⇒ 同一个算子、同一份 `bake<F>` 体；闭式路省掉了整张覆盖度场（393216 个采样）
```

⚠ 第一次量的时候我比的是**烘出来的体积**，两条路的值域**完全相同**（都 `-0.0008..0.0033`）——
因为 `shape` 在覆盖度低于阈值时两边都归零 ⇒ **那个比较没有信息量**。
改成在**同一批方向**上并排量覆盖度本身之后才有读数。这条值得记住：
**判据要选在信号还没被下游压平的那一层。**

### §164.2 泛型方法的代价（必须说清）

`bake<F>` 的实例在**图脚本那一侧**生成 ⇒ **`bake` 的体被静态链进图程序**。
后果：改 `bake` 的体要重编图脚本（这是 §161 第 3 条那条性质在**类型化这一支**上必然的代价）。
老路径（dylib 里那份 `eval_sampled`）仍在，所以 dylib 那一支一位没动。

---

## §165 未做 / 下一步（按价值排）

1. **另外三张图没迁**（`planet` / `desert` / `scene` 仍是老 `node()` 路径）。
   两条路**并存**且互不干扰（老驱动一位未动），所以这不是缺陷；但"契约"只有在图脚本
   真的用它的时候才有类型检查的收益。迁移是纯机械活。
2. **闭式那一支只到"演示 + 判据"，没有进任何一张图的正路**：`--closed-cover` 是个
   单独的分支（`clouds.rs` 里 early return）。要让它成为正路，得改 `art/clouds/*.toml`
   的语义或加一个算子 id（`volume.cloud.coarse.closed` 之类）——
   那又回到「同一个算子、两个身份」的问题（§162.2 的键那一维就是为它准备的）。
3. **`source_hash.rs` 那道门没覆盖新的 `typed.rs`**：`typed.rs` 里的
   `Identity.source_hash` 是手写清单，门看不见它。该把三份 `typed.rs`
   按同一条形状加进门（每个算子至少两段、点到共享依赖）。
4. **`node()` 与 `cook_*` 的键坐标系不同**：前者 `node_key`（无源码哈希），
   后者 `cook_key`（含源码哈希）。两条路的**同名节点会得到不同的键** ⇒
   迁移过程中同一张图**不能混用**（混用会让上半截命中、下半截重算，白烧一遍）。⚠ 已知、未堵。
5. **`px_graphs` 的 `[dev-dependencies]` 与 `[dependencies]` 现在都点了三个算子库** ——
   前者只为把 dylib 编出来（老路径/测试用），后者为 rlib（类型用）。语义重复，但**不能合并**
   （合并会让 `cargo test` 不再编 dylib）。已在 Cargo.toml 的注释里写死。

---

## §166 与 `16-graph-split.md` 的关系

| §159 的口径 | 这一轮之后 |
|---|---|
| 图脚本依赖 `xxx_schema` | **仍成立**，但它现在**也**依赖 `px_cook`（类型化契约） |
| 图脚本不许静态依赖任何 `*_op` | **改口径**：允许（那是类型检查的手段）；替代门见 §162 |
| `xxx_op` 是动态链接 | **仍成立**：实现那一半照旧是 dylib，入口名派生不变 |
| 算子之间只走 schema 的序列化数据 | **仍成立**：跨边界仍是 `PayloadBundle`；**载荷仍逐字节相同**（§163.1 / §164.1） |
| 需要单态化的算子由图自建的 `xxx_op` 并着用 | **样本有了**：`bake<F: FieldFn>` + 图侧的 `SampleField` / `ClosedForm`（§164） |

⇒ 一句话：**这一轮把"算子 id 字符串 + 字节"这层擦除去掉了；泛型方法落在 `bake<F>` 上，
实例由图脚本生成。产物逐字节不变的判据（14/14）说明它不是重写而是换壳。**
