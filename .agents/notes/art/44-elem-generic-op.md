# 一个泛型 element-wise 算子：`px_elem`

> 2026-09-27（`feature/elementwise-op`）。用户的裁定逐句：
>
> 1. "为什么这些 element wise 的函数都要搞个 op 出来？一个 generic 的 element wise Op 在图脚本测
>    单态化成任意 element wise 算子，**还能融合算子**"；
> 2. （问"彻底收到哪一步"）**A**：只收纯 pointwise 的 `constant` / `mix` / `remap`，
>    噪声源（fbm/fbm3/ridged/ridged3）与空间核（craters/stamps）保留；
> 3. （问参数从哪来）**"用 rust 泛型"**；
> 4. （问实例怎么共享）**A：实例身份 = 内容**；
> 5. （问融合）**A：实例支持多上游**；
> 6. **"实例必须要 dylib，否则拖慢编译速度；泛型是对脚本编写者的手感要求"**；
> 7. element 函数住**新开的 `px_elem`**。

## §1 一句话

**声明与手感用 Rust 泛型（`Elementwise::<Waves>` + 每个函数自己的 `Params` 类型），
实现仍然编译成 dylib（改一个函数只重编那一份库，图程序一位不动），身份只看内容
（两个图用同一个函数 ⇒ 同一个键、同一份库、同一份产物）。**

## §2 三样东西各在哪

```text
px_elem/src/lib.rs      ElementFn（Params / Inputs / NAME / SOURCE / ROOTS / BODY / SYMBOL / shape）
                        + Elementwise<F>（PxOp 实现，按内容键装库）+ fill（唯一那条循环）
px_elem/src/specs.rs    **数据表**：ty / name / roots / source（体文件）/ body（体模板）
px_elem/src/<函数>.rs   作者那一面：`struct Waves;` + `impl ElementFn for Waves` + `WavesParams`
px_elem/body/<函数>.rs  真正那一格怎么算（**不在 `src/` 下**）
```

⚠ **为什么体文件必须住在 `src/` 之外**：图程序依赖 `px_elem`（要拿 `Waves` 与 `WavesParams`），
而 cargo 的增量编译是**按 crate** 的 —— 体文件若在 `px_elem/src/` 里，改一行算法就会重编
`px_elem`（连带所有图程序）。住在 `body/` 下就与 cargo 无关：只有**生成的那一份实例库**要重编
（R1，与 `art/inst/*.rs` 同一条规矩）。体文件里的 `use` 一律全路径（它被 `include!` 进生成物）。

## §3 身份里必须拿掉的两轴（这是"可共享"的全部机关）

`px_cook::inst::key` 从前哈希了这两样：

| 轴 | 为什么从前在里面 | 现在 |
| --- | --- | --- |
| `inst.op_id`（手写名 `"field.remap/waves"`） | 它是实例的"名字" | **拿掉**：手写名降为**读数/报错**用的标签（`PxOp::ID` 仍在 `OpId` 里进键 —— 但那是**规格**的名字，写在 `px_elem` 里、所有图共用，不是每个图自己起的） |
| `px_graph_schema::SOURCE_HASH`（**图程序自己**的源码指纹） | 保守：怕图侧源码影响产物 | **拿掉**：体与它的依赖全在 dylib 里（`decl_hash` + `roots` 名册 + 体模板 + 体文件字节已经覆盖）⇒ 图程序改一个字不该让实例换键 |

⇒ 实例身份 = `decl_hash ‖ roots 名册哈希 ‖ interface ‖ 体模板(去空白) ‖ 体文件字节`。
**两个图用同一个规格 ⇒ 同一个键**（这就是"共享"），改那个体文件 ⇒ 只换那一条的键。

## §4 `ElementFn` 的形状（"用 rust 泛型"落点）

```rust
pub trait ElementFn: 'static {
    /// ⚠ **参数类型是关联类型**：每个 element 函数有自己的参数 struct（不再共用
    ///   `RemapParams` 那种"三个 float 撑所有函数"的口袋）⇒ 脚本作者拿到的是**类型**，
    ///   字段名错、少给一个上游都是编译错。
    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    /// **多上游**：1 个 / 2 个 / 3 个由函数自己声明。融合 = "一个函数一次拿到全部上游、
    /// 在**一个循环**里算完整条链"（一个节点、一个产物、上游只读一次）。
    type Inputs: PxInputs;
    const NAME: &'static str;
    /// 下面这四栏**由 `px_elem_specs!` 填**（作者不写）：身份与构建的输入，不是艺术参数。
    const SOURCE: &'static str;                 // 体文件（相对 workspace 根）：内容进身份
    const ROOTS: &'static [&'static str];       // 体编译时链的 crate（build graph 的边）
    const BODY: &'static str;                   // px_body! 的体表达式
    const SYMBOL: &'static str;                 // px_inst__<类型名>
    /// 输出那张场的形状：生成类从参数来、过滤类从上游来。
    fn shape(params: &Self::Params, inputs: &Self::Inputs) -> Shape;
}

pub struct Elementwise<F: ElementFn>(PhantomData<F>);
impl<F: ElementFn> PxOp for Elementwise<F> {
    const ID: &'static str = F::NAME;
    const LIB: &'static str = "";          // 实现不在预置库里
    const SYMBOL: &'static str = F::SYMBOL;
    type Params = F::Params; type Inputs = F::Inputs; type Payload = Field;
    fn source_hash() -> Result<&'static str, String> { cached_key::<F>() }   // ← 内容键，运行期算
    fn decl_hash() -> &'static str { px_elem::DECL_HASH }
    fn render(&self, p, i) -> Result<Field, String> {
        let body = ops::load_at::<Self>(inst::library_path(Self::source_hash()?), F::SYMBOL)?;
        body(p, i)
    }
}
```

* **键在运行期算**：`px_elem::key_of::<F>()` = `px_cook::inst::key_of_facts("", interface,
  DECL_HASH, F::ROOTS, F::SOURCE, F::BODY)` —— 与 `px build` 判断"缺哪些库"**同一个函数**。
  `source_hash()` 要返回 `&'static str`，所以每个 `F` 缓存一格（`TypeId` → 泄漏的字符串）。
  ⚠ 这里**不是** `interface()` 那道"泛型里的 static 不按单态化分开"的坑：缓存键用了 `TypeId`。
* `interface()` 由 `Self::Params/Inputs/Payload` 的**类型名**算（继承 `PxOp` 的默认实现）：
  泛型参数进类型名 ⇒ 不同函数天然不同接口。`facts_of::<F>()` 走**同一个** `interface_hash`，
  于是"看不见类型的那一侧"（build script）与图程序拿到的是同一份事实。
* `decl_hash()` = `px_elem` 那一份源码指纹（`px_elem/build.rs` 发的 `PX_SOURCE_HASH`）：
  参数 struct 加一栏就换键（`19-generic-inst.md` §177 那条老账仍然钉着）。
* **没有"生成物补 const"那种 `Facts` 了**（第一版设计有，实测绕不过孤儿规则：
  `impl Facts for <px_elem 的类型>` 写在图程序里非法 —— 两个类型都外来）。

## §5 收掉的三个预置

`field.constant` / `field.mix` / `field.remap` 的 `px_op!` 声明与 `px_field_op` 实现删掉，
三档各变成一个 element 函数（`px_elem` 的规格 + 体文件）；图侧 **20 处**调用改成
`Elementwise::<X>`：

| 图 | 处数 |
| --- | --- |
| clouds | 3 |
| desert | 3 |
| gasgiant | 1 |
| moon | 1 |
| nebula | 9 |
| planet | 3 |

⚠ **代价（要记在明面上）**：这三档的**参数类型**从共享结构（`params::constant::Params` /
`mix::Params` / `remap::Params`）变成**每个函数自己的** `Params` ⇒ 对应的
`art/<图>/<节点>.toml` 字段名要跟着改（脚本层的 `..node_params(...)` 照旧）。
好处是"这个节点吃什么参数"从此由**函数**说了算，不再是"三个 float 的口袋"。

**保留**：`fbm` / `fbm3` / `ridged` / `ridged3`（噪声源，参数是一整套八度设置、不是"这一格怎么算"）、
`craters` / `stamps`（空间核：27 邻域 + 哈希撒章）、`gradient` / `warp` / `warp3`
（要**再采样上游** —— 图侧函数今天拿不到那个口子，等补上再收，见 §7）。

## §6 判据

1. `the_same_spec_in_two_graphs_is_the_same_key`（**共享**）：同一个规格 + 同一份参数，
   在两个不同图名的 `begin` 里算出**同一个节点键**。
2. `editing_the_body_file_changes_the_key`（**内容身份**）：改体文件字节 ⇒ 换键；
   把同名体文件挪到另一个路径 ⇒ **不**换键（R1 的等价物）。
3. `the_generic_op_computes_what_the_preset_did`（**逐位等价**）：迁移前后同一个节点的产物
   逐字节相同（收 `constant`/`mix`/`remap` 时用来钉住"没换产物"）。
4. 融合那条：一条链（例如 `remap → mix`）写成**一个**函数 ⇒ 键只有一个节点、上游只读一次。

## §7 还没做（下一轮）

* **图侧函数"再采样上游"**：`gradient` / `warp` / `warp3` / `craters` / `stamps` 要它才能搬过来。
* `px_elem` 的规格表给脚本作者之外的人看的那一面（`px list` 里列出来）。

## §8 落地进度（2026-09-27）

已到位：

* `px_elem`（新 crate，进 workspace）：`ElementFn`（`Params`/`Inputs`/`NAME`/`SOURCE`/`ROOTS`/`BODY`/
  `SYMBOL`/`shape`）+ `Elementwise<F>`（`PxOp`，`LIB = ""`）+ `fill`（唯一那条循环）+
  `px_elem_specs!`（一处声明展开出：标记类型 + `ElementFn` 实现 + `ElemSpec` + 事实）。
* **图侧不需要生成物**（**不是**"element 这一档不需要代码生成" —— 那句话说错了，2026-09-27 纠正）：
  `px build` 那一侧**照样**写一个极小的具体 crate（`target/jit/<内容键>/src/lib.rs`）再编成 dylib
  —— **单态化就发生在编它的时候**（Rust 的泛型只在编译期单态化，dylib 里装不下泛型函数，
  这是"实例必须要 dylib"的硬约束）。那一份生成物长这样：

  ```rust
  type Constant = px_elem::Elementwise<px_elem::specs::Constant>;
  include!("<体文件绝对路径>");                     // 作者的算法原样进来
  px_graph_schema::px_body! { Constant, |p, i|
      // ⚠ `fill` 的实参写**全路径**：裸名 `Constant` 被 `px_body!` 占着（那个要 `PxOp`），
      //   而 `fill` 要的是 `ElementFn` —— 同一个名字不可能同时是这两种类型（§9 第 3 条）。
      px_elem::fill::<px_elem::specs::Constant>(p, i, |uv, direction| value(p, i, uv, direction)) }
  px_graph_schema::px_impl_lib!();
  ```

  它省的只是**图侧**那一份生成物（`OUT_DIR/insts_gen.rs` 的类型 + 事实表）：图侧能在
  **运行期**算出同一个内容键（`px_elem::key_of::<F>()` 走 `px_cook::inst::key_of_facts`，
  与 `px build` 判断"缺哪些库"同一个函数）⇒ 没有"把事实烘成 const"这一步，
  也就没有孤儿规则那道墙（`impl Facts for <px_elem 的类型>` 写在图程序里非法）。
  代价是每个节点算一次键（几毫秒的文件读取 + 名册哈希，与既有实例那一档同量级）。
* **实例身份 = 内容**（`px_cook::inst::key`）：`op_id` 与图程序的 `SOURCE_HASH` 都拿掉了。
  实测：3 条既有实例（band/latbands/waves）用新键编出来、`inst_probe` 装载→算→命中全过。
* 存量 bug 顺手修掉：`px_body!` 早已改成两参，而 `px_cook::inst::lib_text` 与
  `inst_recipe.rs` 的体模板还在传 `g` ⇒ **实例那一档本来编不过**（老 dylib 还在盘上，
  所以判据没红）。今天两处都改成两参。

未到位（当天就补上了，见 §9）：让 `Elementwise::<Constant>` 端到端跑起来。

## §9 端到端接线（同一天，同一分支的后一段）

已到位：

* `px_cook::inst` 认识**两档**（`InstKind::Decl { schema / module / decl }` 与
  `InstKind::Element { ty }`）：`lib_text` 按档分叉（element 那一档写
  `type <ty> = px_elem::Elementwise<px_elem::specs::<ty>>;` + `include!(体文件绝对路径)` +
  `px_body! { <ty>, |p, i| … }` + `px_impl_lib!()`），`manifest_text` 按档给
  `[dependencies]`（element 的 `px_elem` 由规格的 `roots` 那一栏给，不需要额外 schema 行）。
* `InstCatalogue` 变成**运行期可增长**的（`Vec<InstCodegen>` + `insert` + `from_generated`；
  生成物里落的是一串条目切片，不是表本身）—— element 那几条**不是 build script 生成的**
  （**图侧**没有生成物）：由 `px_graphs::insts::codegen()` 从 `ELEM_SPECS` 现读、
  用 `key_of_facts("", …)` 现算键插进来，整张表 `OnceLock` 缓存一次。
* `px_graphs::insts::build()` 除 recipe 那 3 条外，再遍历 `ELEM_SPECS` 登记节点
  （两条路走**同一个** `BuildGraph::facts`）。
* `px_graphs` 加 `px_elem` 依赖。⚠ 它不是实现库：算法仍住在 `px_elem/body/*.rs`、由
  `px build` 编进实例库，而那几个文件**不在 `src/` 下** ⇒ 加这个依赖不动 R1。
* 判据 `px_graphs/tests/elem.rs`：端到端（缺才编库、真装载、逐格等于参数）＋跨图共享
  （两个图名 ⇒ **同一个节点键**，且第二张图**必命中**）＋内容身份（键 == `key_of_facts` 的
  返回值；同一份体字节换个路径**不**换键、改一行就换键、模板多一个真 token 就换键）＋
  符号名口径（`<Constant as ElementFn>::SYMBOL == px_cook::inst::symbol(ty)`）。
  ⚠ "改体文件"那一条**不碰工作树**：副本写在 `target/elem_key_probe/` 下。
* `tests/inst_gate.rs` 的"图不漏"改成数**两张表**（recipe + `ELEM_SPECS`），等式仍是等式。

⚠ 顺手修掉三处"其实编不过"（第一次真跑 `px build` 才暴露；此前判据看不见，因为老 dylib 还在盘上）：

1. `px_elem/body/constant.rs` 的 `//!` 内层文档注释：它被 `include!` 在生成物的**第二段**
   ⇒ `error[E0753]: expected outer doc comment`（同一个坑 `inst_recipe.rs` 头上已记过一次）。
   改成 `//`。
2. `px_elem::ConstantParams` **不存在**（只有 `px_elem::constant::ConstantParams`）⇒ 体文件的
   签名编不过。`px_elem/src/lib.rs` 里加一行 re-export（作者面本来就该是这个短路径）。
3. 体模板里的 `px_elem::fill::<Constant>` 用的是**裸名**，而生成物里裸名被 `px_body!` 占着、
   必须是 `Elementwise<…>`（它只有 `PxOp`）⇒ `E0277: ElementFn is not satisfied`。
   模板改成全路径 `px_elem::fill::<px_elem::specs::<ty>>`（与"生成物里的 `use` 一律全路径"
   同一条规矩）。⚠ 模板是键的一轴 ⇒ element 实例的键跟着换；这一档此前**一份库都没编出来过**，
   没有陈旧库可命中。

⚠ 记在明面上、**上一段末尾已由修法 A 收掉**的两处（历史，别再照着写）：

* ~~element 实例库会把 `px_cook` / `px_graph` 链进去~~（**已修**，见 §10）。
* ~~`Elementwise::<Constant>` 不是一个值~~（**已修**：图侧生成单元结构体，脚本写 `elem::Constant`）。

## §10 修法 A：拆依赖 + 裸体宏 + 图侧单元结构体（2026-09-27，同一天第三段）

那两条不是"记着的小账"，是**真问题**：element 库 **15.0 MB vs 声明档 5.3–5.8 MB**
（`px_elem` 为了 `Elementwise<F>` 的 `render`/`source_hash` 依赖 `px_cook` ⇒ 整条驱动链被静态
链进每一份实例库），而"实例编成 dylib 是为了编译速度"是用户明确的口径；另外
`crate_graph.rs` 那条"实现库不许依赖驱动"的不变式**在这一档没有门看着**。
用户选 A，落地如下：

| 层 | 现在 | 为什么 |
| --- | --- | --- |
| `px_elem`（作者面） | 参数 struct / `ElementFn` / 规格表 / `fill` / 体文件；依赖只剩 `px_graph_schema` + `px_field_schema` | **不懂驱动**——懂了就会把驱动链进每一份实例库 |
| 生成的实例库 | `px_body_raw! { <ty>, <Params>, <Inputs>, <Payload>, \|p,i\| … }`（契约层新增的**裸体宏**）：不引任何算子类型 | 引 `Elementwise<…>` = 引整条驱动链 |
| `px_graphs::elem`（图侧） | 每条规格一个**单元结构体**（脚本写 `elem::Constant` 当值用；与 `field::Fbm` 同形）+ 内容键 + 装载（`ElemOp<F>` 只在这儿存在） | 它要 `px_cook`；而实例库不引它 |
| `px_graphs/build.rs` | 为每条 `ELEM_SPECS` 生成那个单元结构体（`OUT_DIR/elem_gen.rs`）+ catalogue 条目 | 类型必须住图侧；手感因此与预置档一致 |

* **读数**：element 库 **15.0 MB → 5.3 MB**；`px build --gc` 回收 31.3 MB；
  内容键随根集合更新（多了 `px_field_schema`：参数里的 `Shape` 住那儿，改它必须换键）；
  节点键在 `elem-one`/`elem-two` 下仍相同、第二个图必命中；`px_graphs` 23 个测试二进制全绿。
* **新增的门**（`tests/elem.rs` ④）：对每条规格断言 `px_elem::all_roots(spec)` **不含**
  `px_graph`/`px_cook`/`px_render`/`px-scene`/`px_pass`/`px_graphs` —— 15.0 MB 那件事从此有门。
* `px_elem::all_roots` 是**唯一**那处根口径（生成器算键、图侧算键、生成物 `[dependencies]`
  三处都走它）—— 实测栽过一次：payload 写 `px_field_schema::field::Field` 而 manifest 里
  没这个依赖 ⇒ 生成的库编不过。
* ⚠ **图侧代码生成回来了**（`elem_gen.rs`），但理由与从前不同：从前是"把事实烘成 const"，
  现在是"**算子类型必须住图侧**"（住作者面 = 15 MB 库）+ 手感对齐。两档的键都由生成器算：
  element 那一档生成器读得到 `px_elem::ELEM_SPECS`（build-dependency）与体文件字节 ⇒
  与图侧运行期算的是同一个值（同一个 `key_of_facts`）。
* 仍记着的两条小账：element 条目的报错行号指向**规格表那一行**（体模板是宏拼的，表里没有
  那段原文，指到那一行是诚实的近似）；`InstCatalogue::insert` 的唯一性仍是**断言**
  （与 `BuildGraph::facts` 一致）。
