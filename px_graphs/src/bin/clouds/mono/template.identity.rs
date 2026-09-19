// ⚠ 这一份实例的**身份**，由 `px_graphs/src/bin/mono-gen.rs` 从
// `src/bin/<图>/mono/fields.mono` 具体化。它被 `lib.rs` 用 `include!` 进来
// ⇒ **不能有 `//!` 文档注释**。
//
// 身份两半，**都不需要人维护**：
// * `interface()` —— 三个类型名（参数 / 输入 / 输出）的哈希。改了接口自动变。
// * `SOURCE_HASH` —— 这个 crate 编译进去**全部源码**的指纹（`build.rs` 算的，
//   它把 stage 1 的场函数、stage 2 的模板、两侧的契约都算进去）。
//   改其中任何一份 ⇒ 身份变 ⇒ 缓存键变 ⇒ 下一次烘必然重算。
//   于是「记得升 VERSION」这件事在这一支上**不存在**。

const MONO_ID: &str = @MONO_ID@;

/// 接口形状哈希：与算子库那边同一条口径（`px_cook::interface_hash`）。
///
/// ⚠ 只取**两侧共有的**三样：参数类型、输出域、（空的）图参数类型。
///   这一份实例的输入形状由模板手写（见 `template.lib.rs` 那段注释），
///   所以它固定折算成常量 —— 它不是这里的变量。
fn interface() -> u64 {
    px_cook::interface_hash(&[
        ::core::any::type_name::<params::Params>(),
        "mono-instance",
        ::core::any::type_name::<VolumeData>(),
    ])
}

/// 这个 crate 编译进去全部源码的指纹（`build.rs` 算的，见 `generated/build.rs`）。
const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");
