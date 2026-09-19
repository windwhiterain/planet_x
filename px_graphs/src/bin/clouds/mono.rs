// clouds 这一张图的**单态化声明**读的东西只有三样，所以它就是一个 `key = value` 文件：
//   lib       —— 生成物的库名（⚠ 每张图必须不同：装载目录里同名会互相覆盖）
//   id        —— 这一份实例的算子 id（⚠ 每张图不同，且不能复用 px_volume_op 的那个）
//   version   —— 只在**接口**变了时才升；改场函数不需要动它（内容哈希自己会变）
//   fields    —— stage 1：要单态化的那个场函数（相对本文件的 `lib.rs`）
//   template  —— stage 2 的模板目录（相对本文件）
//   ingredient —— 身份要覆盖的额外源码（可多条；相对 workspace 根）
//
// ⚠ 它**不是** Rust 模块：`lib.rs`、`mono-gen` 与 `fields.rs` 都不 include 它。
// 于是它不必为三个不同的宿主凑出可编译的形状，也不会被任何一条编译路径牵动。
// 读取者是 `src/bin/mono-gen.rs`（按行解析，见那里的 `read_declaration`）。
//
// ⚠ 为什么 `id` 要带图名：`Context::find` 取的是「已装载库里第一个命中的 op_id」，
// 而装载顺序是路径字典序 ⇒ 两张图的实例同 id 时会互相抢，而且各自的语义并不相同。

lib = px_mono_clouds
id = clouds.volume.cloud.coarse.closed
version = 1

fields = mono/fields.rs
template = mono

# 身份要覆盖的共享依赖（stage 1 与 stage 2 由生成器自动收进去，这里只列两侧的契约）。
ingredient = px_cook/src/field_fn.rs
ingredient = px_volume_op/src/lib.rs
ingredient = px_verify/src/cloud_field.rs
ingredient = px_verify/src/noise.rs
ingredient = px_verify/src/proxy.rs
ingredient = px_volume_schema/src/volume.rs
ingredient = px_volume_schema/src/params.rs
ingredient = px_volume_schema/src/payload.rs
ingredient = px_field_schema/src/field.rs
