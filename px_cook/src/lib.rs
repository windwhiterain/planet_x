//! **px_cook**：图脚本这一侧**唯一的门**。
//!
//! 它交回三样东西：
//!
//! * [`cook`] —— 算键 → 查 → 命中就解载荷；不命中就调实现、编码、落盘。**唯一**那个缓存辅助函数。
//! * 图的生命周期（`GraphSpec` / `begin` / `Graph` / `finish`，re-export 自 `px_graph`）。
//! * 各域的**算子表**（`field` / `volume` / `mesh`，re-export 自各域 schema 的 `ops`）。
//!
//! 于是图脚本顶上就是**一行**：
//!
//! ```ignore
//! use px_cook::{Domain, GraphSpec, begin, cook, field, mesh};
//! ```
//!
//! 图脚本是**普通 Rust**：
//!
//! ```ignore
//! let graph = begin(GraphSpec::field("planet", 780, 520, Domain::Cube));
//! let clusters = cook::<field::Fbm>(&graph, "clusters", ())?;
//! let mixed    = cook::<field::Mix>(&graph, "mixed",
//!                    field::MixInput { a: clusters, b: carved, mask: weight })?;
//! graph.finish();
//! ```
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

/// 契约层那几样：图脚本从这一扇门一并拿到（`Cooked` / `Grid` / 身份哈希 / `blake3` 的 re-export）。
pub use px_graph_schema::{Cache, Cooked, Grid, blake3, fnv1a, fnv1a_sources};

/// 各域的**算子表**：声明在这里，实现在 dylib 里。
pub use px_field_schema::ops as field;
/// 驱动那一半（图的生命周期 + 清单 + 键）。图脚本只从这里拿机制，别处不用再开一扇门。
pub use px_graph::{
    BakedShader, Graph, GraphSpec, ManifestEntry, SHADER_VERSION, artifact_path_of,
    bake_shader_graph, begin, cache_root, cameras, graph_manifest, hex, hex_short, manifest_key_of,
    scene_key, shader_key, workspace_root, write_graph_manifest, write_shader,
};
pub use px_mesh_schema::ops as mesh;
/// 图脚本动不动就要写 `Domain::Cube`：从这里一并给出，省得再添一行依赖。
pub use px_protocol::art::Domain;
pub use px_volume_schema::ops as volume;

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

/// **缓存辅助函数**：算键 → 查 → 命中就解载荷；不命中就 `render` → 编码 → 落盘。
///
/// 参数来自 `art/<图>/<节点名>.toml`（按节点名取）；上游来自 `inputs`（图参数 struct，
/// 键由它聚合）；域由 `PxOp::Payload` 决定。
pub fn cook<O>(
    cache: &dyn Cache,
    node: &str,
    inputs: O::Inputs,
) -> Result<Cooked<O::Payload>, String>
where
    O: PxOp,
{
    // ⚠ 画布从 `cache` 取，**不是参数**：它本来就住在驱动里（`GraphSpec`），
    //   而键里那一份与算子 `render` 手里那一份必须是同一个值。
    let grid = cache.grid();
    let op = O::new();
    let toml_text = cache.params_text(node);
    let from_file = toml_text.is_some();
    let (params, params_json) = canonical_params::<O::Params>(toml_text.as_deref())?;
    // ⚠ 缺文件是静默用默认值的 —— 这一条记录让"我少写了什么/写错了哪个字段名"跑完就看得见。
    cache.record_params(node, O::ID, &params_json, from_file);

    // ⚠ 接口形状哈希只算一次：键、读数、清单三处用的是**同一个值**。
    let interface = O::interface();
    // ⚠ **实现的源码指纹**是运行期取的（从实现库的身份符号）—— 图程序不重编也能看见它变了，
    //   于是"改了实现却命中旧产物"这件事不可能发生。
    let source_hash = O::source_hash()?;
    // ⚠ **画布按域决定要不要进键**：场的分辨率就是画布，体积/网格不是。
    //   一刀切（都掺）会让"改画布"连带重烘体积；一刀切（都不掺）会让场出现
    //   "同一个键、不同分辨率"。
    let canvas = <O::Payload as Build>::RESOLUTION_IS_CANVAS.then_some(grid);
    let base = px_graph_schema::node_key(
        &OpId {
            id: O::ID,
            interface,
            source_hash,
        },
        &params_json,
        canvas,
        |hasher| inputs.collect(hasher),
    );
    // ⚠ 相机那一档：产物里带着相机表 ⇒ 相机变了产物内容就变 ⇒ 必须进键。
    // ⚠ 相机口径从**载荷类型**推：域就是这个类型，没有第二处声明。
    let with_cameras = <O::Payload as Build>::WITH_CAMERAS;
    let key = if with_cameras {
        px_graph_schema::key_with_cameras(base, cache.cameras())
    } else {
        base
    };

    if let Some(payload) = cache.fetch(key) {
        let value = <O::Payload as Build>::decode(&payload, grid.projection, node)?;
        let detail = <O::Payload as Build>::detail(&value);
        cache.store(
            Report {
                node,
                op: O::ID,
                interface,
                key,
                hit: true,
                millis: 0,
                with_cameras,
                detail,
            },
            &payload,
        )?;
        return Ok(Cooked::new(key, value, true, 0, payload.bytes()));
    }

    let started = Instant::now();
    let value = op.render(&params, &inputs, grid)?;
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
            with_cameras,
            detail,
        },
        &payload,
    )?;
    Ok(Cooked::new(key, value, false, millis, payload.bytes()))
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
///     |p, _i, g| bake(g, |d| (d[2] * p.frequency).sin() * p.gain) }
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
     |$p:ident, $i:ident, $g:ident| $body:expr) => {
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
                $g: ::px_graph_schema::Grid,
            ) -> ::core::result::Result<$payload, ::std::string::String> {
                ::core::result::Result::Ok($body)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use px_graph_schema::Build;

    /// **哪些域的产物尺寸就是画布** —— 这条决定键里要不要掺画布。
    ///
    /// 病根：一刀切都会错。
    /// * 都掺 ⇒ 改画布连带重烘体积/网格（它们的尺寸由参数给，与画布无关）。
    /// * 都不掺 ⇒ 场出现「同一个键、不同分辨率」。
    #[test]
    fn only_the_field_domain_bakes_the_canvas_into_its_size() {
        assert!(
            <px_field_schema::field::Field as Build>::RESOLUTION_IS_CANVAS,
            "场的分辨率就是画布 ⇒ 画布必须进键"
        );
        assert!(
            !<px_volume_schema::VolumeData as Build>::RESOLUTION_IS_CANVAS,
            "体积的分辨率由参数（res/layers）给 ⇒ 画布与它无关"
        );
        assert!(
            !<px_mesh_schema::MeshData as Build>::RESOLUTION_IS_CANVAS,
            "网格的尺寸由参数给 ⇒ 画布与它无关"
        );
        // 顺带把"相机口径"也读一遍（`cook` 用它，不再有并行的 `OpKind`）。
        assert!(<px_field_schema::field::Field as Build>::WITH_CAMERAS);
        assert!(!<px_volume_schema::VolumeData as Build>::WITH_CAMERAS);
        assert!(<px_mesh_schema::MeshData as Build>::WITH_CAMERAS);
    }
}
