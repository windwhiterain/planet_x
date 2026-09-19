//! **这一张图的单态化实例**（stage 1 与它的身份）。
//!
//! 一个 `xxx_op` 在图脚本这一侧只需要两样东西：
//!
//! * **stage 1**（[`FIELDS`] 指的那个文件）：图自己的场函数 —— 它是**编辑面**；
//! * **身份**（[`MONO_ID`] / [`VERSION`] / [`SOURCE_HASH`]）：这一份实例是谁。
//!
//! 生成器（`--bin mono-gen`）按这一份身份与 `mono/template/` 生成一个 cdylib，
//! 图程序在运行时**动态装载**它 —— 于是改一行场函数只需重编那一个小 dylib，
//! `clouds.exe` 一个字节都不用动。

/// 这一份实例的算子 id。
///
/// ⚠ **不能用 `params::CLOUD_COARSE`**：`Context::find` 是「在已装载的库里取**第一个**
/// 命中的 op_id」（`px_graph/driver.rs`），而装载顺序是路径字典序 ⇒ 同 id 时
/// 这一份会**静默抢掉** `px_volume_op` 的那一个，老路径就变质了。两档并存必须两个身份。
pub const MONO_ID: &str = "volume.cloud.coarse.closed";

/// 与 `SOURCE_HASH` 配对的版本号：只在**接口**变了（描述符的 inputs / 参数字段）时才升。
/// 改场函数**不需要**动它 —— `SOURCE_HASH` 自己会变。
pub const VERSION: u32 = 1;

/// stage 1 的源码（相对 `px_graphs/`）。生成器把它复制/引用进生成物。
pub const FIELDS: &str = "mono/fields.rs";

/// 身份要覆盖的**全部**源码：stage 1 + stage 2 的模板 + 它两侧的泛型体与契约。
///
/// ⚠ 漏一份就会「改了它而身份没变」⇒ 缓存静默给旧产物（§28.2 那条病的同一个形状）。
/// 生成器把它算成 `fnv1a_sources` 的常量写进 `identity.rs`。
pub const INGREDIENTS: [(&str, &str); 15] = [
    // stage 1（编辑面）
    ("px_graphs/mono/fields.rs", include_str!("../mono/fields.rs")),
    ("px_graphs/src/mono.rs", include_str!("mono.rs")),
    // stage 2（模板的三份）
    ("px_graphs/mono/template/Cargo.toml", include_str!("../mono/template/Cargo.toml")),
    ("px_graphs/mono/template/lib.rs", include_str!("../mono/template/lib.rs")),
    // 泛型体与它依赖的参照实现
    ("px_cook/src/field_fn.rs", include_str!("../../px_cook/src/field_fn.rs")),
    ("px_cook/src/lib.rs", include_str!("../../px_cook/src/lib.rs")),
    ("px_volume_op/src/lib.rs", include_str!("../../px_volume_op/src/lib.rs")),
    ("px_verify/src/cloud_field.rs", include_str!("../../px_verify/src/cloud_field.rs")),
    ("px_verify/src/noise.rs", include_str!("../../px_verify/src/noise.rs")),
    ("px_verify/src/dual.rs", include_str!("../../px_verify/src/dual.rs")),
    ("px_verify/src/proxy.rs", include_str!("../../px_verify/src/proxy.rs")),
    // 领域约定与载荷
    ("px_volume_schema/src/volume.rs", include_str!("../../px_volume_schema/src/volume.rs")),
    ("px_volume_schema/src/params.rs", include_str!("../../px_volume_schema/src/params.rs")),
    ("px_volume_schema/src/payload.rs", include_str!("../../px_volume_schema/src/payload.rs")),
    ("px_field_schema/src/field.rs", include_str!("../../px_field_schema/src/field.rs")),
];

/// 身份哈希（生成器写进 `identity.rs` 的那个数）。
///
/// ⚠ 走 `fnv1a_sources`（每段带长度前缀）而不是自己拼串：`["ab","c"]` 与 `["a","bc"]`
/// 不会撞，顺序也进哈希 —— 与仓里每一条 `SOURCE_HASH` 同一条口径。
pub fn source_hash() -> u64 {
    // ⚠ `INGREDIENTS` 写成定长数组就是为了这一行：`fnv1a_sources` 收 `&[&str]`，
    //   而 `.map()` 只有定长数组才有 —— 多一份配料编译期就报。
    let [a, b, c, d, e, f, g, h, i, j, k, l, m, n, o] = INGREDIENTS.map(|(_, text)| text);
    px_graph_schema::fnv1a_sources(&[a, b, c, d, e, f, g, h, i, j, k, l, m, n, o])
}
