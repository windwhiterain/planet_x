// ⚠ 这一份实例的**身份**，由 `px_graphs/src/bin/mono-gen.rs` 从 `px_graphs::mono` 具体化。
// 它被 `lib.rs` 用 `include!` 进来 ⇒ **不能有 `//!` 文档注释**（外层文档只能在 crate 根顶部）。
//
// `SOURCE_HASH` 覆盖「stage 1 的场函数 + 泛型体 + 模板 + 契约」的内容：
// 改其中任何一份 ⇒ 身份变 ⇒ 缓存键变 ⇒ 下一次烘必然重算
// （于是「记得升 VERSION」这件事在这一支上不需要人来做）。

const MONO_ID: &str = @MONO_ID@;
const VERSION: u32 = @VERSION@;

const SOURCE_HASH: u64 = @SOURCE_HASH@;

const DESCRIPTOR: OpDescriptor = OpDescriptor {
    id: MONO_ID,
    version: VERSION,
    source_hash: SOURCE_HASH,
    inputs: &["coverage"],
    kind: OpKind::Volume,
};
