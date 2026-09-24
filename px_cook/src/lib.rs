//! **px_cook**：图脚本这一侧**唯一的门**。
//!
//! 它交回四样东西：
//!
//! * [`cached`] —— **图里唯一的那个函数**：读缓存；缓存脏了/没有就调算子算。**唯一**那条缓存路径。
//! * [`node_params`] —— 把 `art/<图>/<节点>.toml` 读成**普通 Rust 值**（缺文件 → `Default`）。
//!   ⚠ 它**不是**缓存那一半：就是"读一个文件 + 解一份 TOML"。
//! * 图的生命周期（`GraphSpec` / `begin` / `Graph` / `finish`，re-export 自 `px_graph`）。
//! * 各域的**算子表**（`field` / `volume` / `mesh`，re-export 自各域 schema 的 `ops`）。
//!
//! 于是图脚本顶上就是**一行**：
//!
//! ```ignore
//! use px_cook::{Domain, GraphSpec, begin, cached, field, mesh, node_params};
//! ```
//!
//! 图脚本是**普通 Rust**：参数与上游都是**普通的值**，脚本想怎么算就怎么算。
//!
//! ```ignore
//! let graph = begin(GraphSpec { name: "planet".to_string() });
//! // 参数：TOML 打底 + 任意 rust 计算（想改哪个字段改哪个）
//! // 尺寸/投影是**参数**（生成类算子那几档的 `Shape`）；这里形状只写一处、每个生成节点都拿它。
//! let shape = field_params::Shape { width: 780, height: 520, projection: Domain::Cube };
//! let p = field_params::fbm::Params { shape, zonal: 2.0, ..node_params(&graph, "clusters")? };
//! // 节点：cached 读缓存；缓存脏了才调算子
//! let clusters = cached(&graph, "clusters", field::Fbm, p, ())?;
//! // 不缓存：直接调那个普通函数（同一个参数、同一个上游），拿到的裸值 `Cooked::of` 一下就能进图
//! let preview = field::Fbm.render(&p2, &())?;
//! // ⚠ element 那一档（`elem::Constant` / `elem::Mix` / `elem::Remap` / `elem::Fuse`）**不长在
//! //   这一扇门后面**：它的算子类型由图侧的生成物给（`px_graphs/build.rs`），所以脚本从
//! //   `px_graphs::elem` 取它；其余手感一模一样（`px_graphs/src/bin/nebula.rs` 那种就是样本）。
//! ```
//!
//! ⚠ **参数是 `Cooked<T>` 就表示"这个参数可以被缓存"**（`HashField for Cooked<T>` 写的是它的键）
//! —— 于是包装对象是嵌套的、递归的：`Cooked<T>` 能喂给下游，也能嵌进参数结构里。
//!
//! ⚠ **输入 struct 由算子自己定义**（就住在它的声明旁边），字段名就是它吃的东西的名字。
//! 于是图侧写错一个字段、少给一个上游，都是**编译错**。本 crate 只提供那个"不吃上游"的 `()`。
//!
//! ⚠ 本 crate **一个算子实现都不依赖**：实现在 `px_*_op` 的 dylib 里，运行时按身份装载。
//! 这条线一破，「改算子实现不重编图程序」那条性质就没了。

pub mod inst;
/// **按文本读源码的工具**（扫宏调用 / 收 `.rs` / 读 workspace 成员）。
///
/// ⚠ **图侧的执行路径不再用它**（`21-codegen-types.md`）：体从图侧源码搬进了
///   `px_graphs/src/inst_recipe.rs` 那张数据表，`px build` 读生成器落下的
///   `inst_out/insts_gen_catalogue.rs` —— 两个扫描点（`claim` / `check_template`）都删掉了。
///   今天用它的是**门**（`px_decls/tests/inst_gate.rs`：数 `px_op!` 的处数）与
///   **生成器**（`px_graphs/build.rs`：收要盯的 `.rs`、读 `members`）。
pub mod inst_scan;

use std::time::Instant;

use px_graph_schema::payload::Build;
use px_graph_schema::{OpId, PxInputs, PxOp, Report};

/// 契约层那几样：图脚本从这一扇门一并拿到（`Cooked` / 身份哈希 / `blake3` 的 re-export）。
pub use px_graph_schema::{Cache, Cooked, blake3, fnv1a, fnv1a_sources};

/// 各域的**算子表**：声明在这里，实现在 dylib 里。
pub use px_field_schema::ops as field;
/// 场域那几档算子的**参数类型**（生成类的形状参数要从脚本里给）。
pub use px_field_schema::params as field_params;
/// 驱动那一半（图的生命周期 + 清单 + 键）。图脚本只从这里拿机制，别处不用再开一扇门。
pub use px_graph::{
    BakedShader, Graph, GraphSpec, ManifestEntry, SHADER_VERSION, apply_store_args,
    args_without_store, artifact_path_of, bake_shader_graph, begin, cache_root, graph_manifest,
    hex, hex_short, manifest_key_of, param_root, scene_key, shader_key, workspace_root,
    write_graph_manifest, write_shader,
};
pub use px_mesh_schema::ops as mesh;
/// NURBS 域的算子表（曲线 / 曲面）。
pub use px_nurbs_schema::ops as nurbs;
/// 图脚本动不动就要写 `Domain::Cube`：从这里一并给出，省得再添一行依赖。
pub use px_protocol::art::Domain;
pub use px_volume_schema::ops as volume;
/// 体积域的参数（`cloud.density` 的 `res` 现在是绝对值）。
pub use px_volume_schema::params as volume_params;

/// 宏生成出来的代码按 `$crate::…` 走 —— 于是用宏的人不必自己依赖契约层。
pub use px_graph_schema;
/// ⚠ `PxInputs::collect` 的签名里就是 `blake3::Hasher` —— 实现者得拿到它，
/// 所以从契约层 re-export，别让人为一个签名去加依赖。
pub use px_graph_schema::HashField;
pub use px_graph_schema::payload;

/// 超参数那一侧：TOML 原文 → `(解析后的参数, 进键的规范 JSON)`。
///
/// ⚠ 缺文件（`None`）走 `Default`，**不是**解一个空串。
pub fn canonical_params<P>(toml_text: Option<&str>) -> Result<(P, String), String>
where
    P: serde::Serialize + serde::de::DeserializeOwned + Default,
{
    let parsed: P = match toml_text {
        Some(text) => toml::from_str(text).map_err(|err| format!("参数解不开：{err}"))?,
        None => P::default(),
    };
    let json = px_graph_schema::canonical_params(&parsed);
    Ok((parsed, json))
}

/// **图里唯一的那个函数**：读缓存；缓存脏了/没有就调算子算。
///
/// 它是图与缓存之间**唯一**的接缝 —— 没有第二条路、没有包装类型：
///
/// * `params`（超参数）与 `inputs`（上游）都是**普通 Rust 值**：参数由脚本给（可以是
///   [`node_params`] 读来的、也可以是完全算出来的），上游是别的节点的 `Cooked<T>`、
///   或者是脚本自己用 [`Cooked::of`] 包出来的裸值；
/// * **参数是 `Cooked<T>` ⇒ 这个参数可以被缓存**（键由它自己的键贡献）；
/// * 缓存命中 ⇒ 解出产物交回去（**不算**）；脏了/没有 ⇒ 调 `f`（实现在实现库里，
///   运行期按身份装载）、编码、落盘；
/// * **不缓存** ⇒ 直接调那个普通函数：`f.render(&params, &inputs)`（`PxOp` 那条路），
///   拿到的裸值要在图里用就 `Cooked::of(…)` 包一下。
///
/// ⚠ 画布从 `cache` 取，**不是参数**：它本来就住在驱动里（`GraphSpec`），
///   而键里那一份与算子 `render` 手里那一份必须是同一个值。
pub fn cached<O>(
    cache: &dyn Cache,
    node: &str,
    f: O,
    params: O::Params,
    inputs: O::Inputs,
) -> Result<Cooked<O::Payload>, String>
where
    O: PxOp,
{
    // ⚠ 参数是**值**（不是从文件读的）：键里那一份与算子手里那一份因此必然是同一个。
    let params_json = px_graph_schema::canonical_params(&params);
    // ⚠ 参数索引那一格由**这里**记（生效值）：`node_params` 只管"这份值是不是从文件打底的"。
    //   两条记录在驱动那里**合并**（先文件、后生效值）—— 见 `Graph::record_params`。
    cache.record_params(node, O::ID, &params_json, false);

    // ⚠ 接口形状哈希只算一次：键、读数、清单三处用的是**同一个值**。
    let interface = O::interface();
    // ⚠ **实现的源码指纹**是运行期取的（从实现库的身份符号）—— 图程序不重编也能看见它变了，
    //   于是"改了实现却命中旧产物"这件事不可能发生。
    let source_hash = O::source_hash()?;
    // ⚠ **尺寸与投影是参数**（`field.*` 那些算子的 `Shape`），不是驱动塞的画布 ——
    //   于是"改尺寸要不要重算"由**参数表**回答：参数里有它的算子自然换键，
    //   没有它的（体积/网格/贴图/NURBS）自然不受影响。
    let base = px_graph_schema::node_key(
        &OpId {
            id: O::ID,
            interface,
            source_hash,
        },
        &params_json,
        |hasher| inputs.collect(hasher),
    );
    // ⚠ **没有相机那一轴**：相机是**场景脚本**里的普通数据（`px-scene` 的 recipe），
    //   不进产物、也就不进键 —— 它属于「怎么看」，不属于「这个节点算什么」。
    let key = base;

    if let Some(payload) = cache.fetch(key) {
        let value = <O::Payload as Build>::decode(&payload, node)?;
        let detail = <O::Payload as Build>::detail(&value);
        cache.store(
            Report {
                node,
                op: O::ID,
                interface,
                key,
                hit: true,
                millis: 0,
                detail,
            },
            &payload,
        )?;
        return Ok(Cooked::new(key, value, true, 0, payload.bytes()));
    }

    let started = Instant::now();
    let value = f.render(&params, &inputs)?;
    let millis = started.elapsed().as_millis() as u64;
    let payload = <O::Payload as Build>::encode(&value)?;
    let detail = <O::Payload as Build>::detail(&value);
    cache.store(
        Report {
            node,
            op: O::ID,
            interface,
            key,
            hit: false,
            millis,
            detail,
        },
        &payload,
    )?;
    Ok(Cooked::new(key, value, false, millis, payload.bytes()))
}

/// 读某个节点的**普通 Rust 参数**：`art/<图>/<节点>.toml`（缺文件 → `Default`）。
///
/// ⚠ 它**不是**缓存那一半：就是"读一个文件 + 解一份 TOML"。有了它，图脚本可以**任意 rust
///   计算**任何一个字段：
///
/// ```ignore
/// let p = field::params::fbm::Params { zonal: look.stretch, ..node_params(&graph, "clusters")? };
/// ```
///
/// ⚠ 它顺带把"这个节点的参数是从文件打底的"记进参数索引（`finish()` 打的那一行为什么
///   要区分两种来源：改调参不重编是**设计者**那条路，参数写死在脚本里是另一条）。
/// ⚠ 缺文件**不报错**（走 `Default`）—— 与从前 `cook` 的口径一致；要发现"我少写了哪个文件"，
///   看 `finish()` 那一行与 `<图>/params.json`。
pub fn node_params<P>(cache: &dyn Cache, node: &str) -> Result<P, String>
where
    P: serde::Serialize + serde::de::DeserializeOwned + Default,
{
    let text = cache.params_text(node);
    let from_file = text.is_some();
    let (parsed, json) = canonical_params::<P>(text.as_deref())?;
    cache.record_params(node, "", &json, from_file);
    Ok(parsed)
}

// ⚠ **`px_inst!` 宏已删除**（2026-09-20，`.agents/notes/art/21-codegen-types.md`）：
//   图侧今天**零宏** —— 它写一张**数据表**（`px_graphs/src/inst_recipe.rs`），
//   `px_graphs/build.rs` 按它**生成类型**（`OUT_DIR/insts_gen.rs`，形状就是这里从前展开出来的
//   那一份：`PxOp` / `InstNode` impl + 几个 const）。⇒ 这条宏路径一个调用都没有了。
//   ⚠ 它当年那份**体模板的口径仍活在 recipe 里**：泛型参数那一位写占位符 `ARG`，
//   生成器做整词替换（`InstRecipe::generated_body()`）—— **那一栏是 key 的一轴，不许改**。
//   ⚠ 下面 `px_local_op!`（图侧**现写**算子）是**另一条路**，与它无关，照旧。

/// **图侧现写的算子**：与 `px_graph_schema::px_op!` 同一张表，但**实现就写在本图程序里**。
///
/// ```ignore
/// struct Band;
/// px_local_op! { Band, "local.band", BandParams, (), Field,
///     |p, _i| bake(&p.shape.filled(0.0), |d| (d[2] * p.frequency).sin() * p.gain) }
/// ```
///
/// ⚠ 它与 `px_op!` 的差别**只在身份那一半**：
///
/// * `px_op!`：实现在 `px_*_op` 那份 dylib 里，身份 = **那份库**的源码指纹（运行期从库里读）；
/// * `px_local_op!`：实现就是**本图程序**，身份 = **本 crate** 的源码指纹（`env!("PX_SOURCE_HASH")`，
///   由本 crate 的 `build.rs` 给）。⇒ 改图脚本里这一行，键跟着变；而它与实现库互不影响。
///
/// ⚠ 这是"**泛型算子**"那一档的落点：泛型参数（比如一个场函数闭包）在**图侧**实例化，
///   单态化出来的代码就编在图程序里 —— 实现库不参与、也不需要在场
///   （`px_graphs/tests/local_op.rs` 有一个真样本）。
///
/// ⚠ 别拿它当"正式算子"用：它跟着图程序重编（那正是它存在的意义），跨图复用请写进 `px_*_op`。
///
/// ⚠ 它住在**那扇门**（这一层）而不是契约层，理由是量出来的：契约层（`px_graph_schema`）的源码
///   在**每一个实现库的身份里**（实现库链它，`build.rs` 的指纹覆盖它）⇒ 往那里加一行宏会让
///   三个库全换身份、全仓换键。图侧算子与实现库无关，放在这一层最省事。
#[macro_export]
macro_rules! px_local_op {
    ($(#[$meta:meta])* $name:ident, $id:literal, $params:ty, $inputs:ty, $payload:ty,
     |$p:ident, $i:ident| $body:expr) => {
        $(#[$meta])*
        impl ::px_graph_schema::PxOp for $name {
            const ID: &'static str = $id;
            /// **空串** = 这个算子不住在任何实现库里（它就编在本图程序里）。
            const LIB: &'static str = "";
            /// 没有实现库，也就没有"库里那个符号"。
            const SYMBOL: &'static str = ::core::stringify!($name);

            type Params = $params;
            type Inputs = $inputs;
            type Payload = $payload;

            fn new() -> Self {
                $name
            }

            /// 身份 = **本 crate**（图程序）这一份源码的指纹 —— 由本 crate 的 `build.rs` 给
            /// （`px_fingerprint::cargo_fingerprint_for_crate(&[])`）。
            fn source_hash() -> ::core::result::Result<&'static str, ::std::string::String> {
                ::core::result::Result::Ok(env!("PX_SOURCE_HASH"))
            }

            /// 现写的实现：泛型参数在这一行里实例化，**单态化落在图程序里**。
            fn render(
                &self,
                $p: &$params,
                $i: &$inputs,
            ) -> ::core::result::Result<$payload, ::std::string::String> {
                ::core::result::Result::Ok($body)
            }
        }
    };
}

#[cfg(test)]
mod tests {

    /// **尺寸只出现在"产出场的生成类算子"的参数里** —— 这决定"改尺寸要不要重算"。
    ///
    /// ⚠ 从前这是靠每域手写一条 `Build::RESOLUTION_IS_CANVAS` 声明来回答的（"场的分辨率
    ///   就是画布，体积/网格不是"）。用户 2026-09-27 的裁定之后**没有画布了**：尺寸与投影
    ///   是**参数**（`field.*` 那几档的 `Shape`）⇒ 这条性质现在是**结构性的**：
    ///   参数里有 `shape` 的算子改尺寸必换键；参数里没有的（体积/网格）自然不受影响。
    #[test]
    fn only_the_field_generators_carry_a_shape_parameter() {
        let field = px_graph_schema::canonical_params(
            &<px_field_schema::ops::Fbm as px_graph_schema::PxOp>::Params::default(),
        );
        assert!(
            field.contains("\"shape\""),
            "场生成类算子的参数里没有 `shape`：{field}"
        );
        for (name, params) in [
            (
                "体积",
                px_graph_schema::canonical_params(
                    &<px_volume_schema::ops::Density as px_graph_schema::PxOp>::Params::default(),
                ),
            ),
            (
                "网格",
                px_graph_schema::canonical_params(
                    &<px_mesh_schema::ops::CubeSphere as px_graph_schema::PxOp>::Params::default(),
                ),
            ),
        ] {
            assert!(
                !params.contains("\"shape\""),
                "{name} 算子的参数里冒出了 `shape`（尺寸该由它自己的参数说）：{params}"
            );
        }
    }
}
