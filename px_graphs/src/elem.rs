//! **element 算子的图侧那一半**：生成的单元结构体 + 内容键 + 装载。
//!
//! 分工（用户裁定"实例必须要 dylib …泛型是对脚本编写者的手感要求"）：
//!
//! | 住哪 | 是什么 | 为什么住那儿 |
//! | --- | --- | --- |
//! | `px_elem`（作者面） | 参数 struct、`ElementFn`、规格表、`fill`（唯一那条循环）、体文件 | **不懂驱动** —— 懂了就会把驱动链进每一份实例库（实测 15.0 MB vs 5.3 MB） |
//! | 这里（图侧） | 每条规格一个**单元结构体**（脚本写 `elem::Constant` 当值用）+ 内容键 + 装载 | 它要 `px_cook`（库落点、算键）；而实例库**不引**它（生成物写裸体，见 `px_body_raw!`） |
//! | `target/pcg/inst/<键>.dll` | 真正算的那份单态化产物 | 改一行算法只重编它（R1），图程序一位不动 |
//!
//! ⚠ 单元结构体是**生成物**（`px_graphs/build.rs` 读 `px_elem::ELEM_SPECS` 写
//!   `OUT_DIR/elem_gen.rs`）：类型名当值用（与预置那一档 `field::Fbm` 同一个手感），
//!   而那个类型**必须住图侧**（住作者面就会把驱动链进去）。

use px_elem::ElementFn;
use px_field_schema::field::Field;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/elem_gen.rs"));
}

/// 每条 element 函数一个算子类型（`elem::Constant`）—— 脚本里**当值用**。
pub use generated::*;

/// 参数与上游那几个类型也从这儿拿（脚本写 `elem::RemapParams` / `elem::MixInput`）——
/// 它们是**作者面**的东西（`px_elem`），在这儿再导一次只是省得脚本两边各 `use` 一行。
pub use px_elem::{
    ConstantParams, FuseInput, FuseParams, MixInput, MixParams, RemapInput, RemapParams,
};

/// **这一条实例的内容键**（= `target/pcg/inst/<键>.dll` 的名字，也是它进节点键的那一轴）。
///
/// ⚠ 与生成器（`px_graphs/build.rs`）算的是**同一个函数、同一组输入**：两处各写一份算法
///   就会出"图脚本算的键与 `px build` 编出来的库不是同一个"那种最难查的错。
pub fn key_of<F: ElementFn>() -> Result<String, String> {
    let facts = px_elem::facts_of::<F>();
    px_cook::inst::key_of_facts(
        // ⚠ **空 op id**：手写名不进身份（用户裁定"实例身份 = 内容"）——
        //   两个图用同一个函数才会落到同一个键上。
        "",
        facts.interface,
        facts.decl_hash,
        &px_elem::all_roots(spec_of::<F>()),
        spec_of::<F>().source,
        spec_of::<F>().body,
    )
}

/// 找 `F` 那一条规格（键与"体在哪"的唯一来源）。
///
/// ⚠ 找不到就是**编译期漏登记**（宏发了类型但没进 `ELEM_SPECS`）—— 当场 panic 说清楚，
///   不许退回一个默认值（那会静默用错身份）。
fn spec_of<F: ElementFn>() -> &'static px_elem::ElemSpec {
    px_elem::ELEM_SPECS
        .iter()
        .find(|spec| F::NAME == spec.name)
        .unwrap_or_else(|| panic!("element 函数 `{}` 不在规格表里", F::NAME))
}

/// `PxOp::source_hash()` 要 `&'static str`，而键是**运行期**算的（要读体文件字节）
/// ⇒ 每个函数缓存一格。
///
/// ⚠ 缓存键用 `TypeId`（不是泛型里的 `static`）：`PxOp::interface()` 那道注释记着的坑是
///   "泛型函数里的 `static` 不按单态化分开"，这里按 `TypeId` 分，就没有那个问题。
pub fn source_hash_of<F: ElementFn>() -> Result<&'static str, String> {
    static KEYS: std::sync::Mutex<Vec<(std::any::TypeId, &'static str)>> =
        std::sync::Mutex::new(Vec::new());
    let id = std::any::TypeId::of::<F>();
    let mut keys = KEYS.lock().expect("内容键缓存锁坏了");
    if let Some((_, key)) = keys.iter().find(|(each, _)| *each == id) {
        return Ok(key);
    }
    let key: &'static str = Box::leak(key_of::<F>()?.into_boxed_str());
    keys.push((id, key));
    Ok(key)
}

/// 按内容键装那一份实例库，调它的符号。
///
/// ⚠ 组装的类型是**泛型参数**：`<F as ElementFn>::Params` 与生成物里
///   `px_body_raw! { … }` 那三个路径是同一批类型（所以签名对得上）。
pub fn render<F: ElementFn>(params: &F::Params, inputs: &F::Inputs) -> Result<Field, String> {
    let path = px_cook::inst::library_path(source_hash_of::<F>()?);
    let body =
        px_graph_schema::ops::load_at::<ElemOp<F>>(path.to_string_lossy().as_ref(), F::SYMBOL)?;
    body(params, inputs)
}

/// `load_at` 要一个**算子类型**才认得出那三个投影类型 —— 这一位只是把 `F` 包一层。
///
/// ⚠ 它**只在这儿存在**（图侧、运行期）：生成的实例库不引它（那儿写的是裸体，
///   三个类型直接摊在签名上）—— 这正是"驱动不许进算法库"那条不变式的落点。
pub struct ElemOp<F: ElementFn>(std::marker::PhantomData<F>);

impl<F: ElementFn> px_graph_schema::PxOp for ElemOp<F> {
    const ID: &'static str = F::NAME;
    const LIB: &'static str = "";
    const SYMBOL: &'static str = F::SYMBOL;
    type Params = F::Params;
    type Inputs = F::Inputs;
    type Payload = Field;
    fn new() -> Self {
        Self(std::marker::PhantomData)
    }
    fn source_hash() -> Result<&'static str, String> {
        source_hash_of::<F>()
    }
    fn decl_hash() -> &'static str {
        px_elem::DECL_HASH
    }
}
