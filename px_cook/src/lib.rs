//! See docs/graph.md

pub mod fault;
pub mod inst;
pub mod inst_scan;

use std::time::Instant;

use px_graph_schema::payload::Build;
use px_graph_schema::{OpId, PxInputs, PxOp, Report};

pub use px_graph_schema::{Cache, Cooked, blake3, fnv1a, fnv1a_sources};

pub use px_field_schema::ops as field;
pub use px_field_schema::params as field_params;
pub use px_graph::{
    BakedShader, Graph, GraphSpec, ManifestEntry, SHADER_VERSION, apply_store_args,
    args_without_store, artifact_path_of, bake_shader_graph, begin, cache_root, graph_manifest,
    hex, hex_short, manifest_key_of, param_root, scene_key, shader_key, workspace_root,
    write_graph_manifest, write_shader,
};
pub use px_mesh_schema::ops as mesh;
pub use px_nurbs_schema::ops as nurbs;
pub use px_protocol::art::Domain;
pub use px_volume_schema::ops as volume;
pub use px_volume_schema::params as volume_params;

pub use px_graph_schema;
pub use px_graph_schema::HashField;
pub use px_graph_schema::payload;

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
    let params_json = px_graph_schema::canonical_params(&params);
    cache.record_params(node, O::ID, &params_json, false);

    let interface = O::interface();
    let source_hash = O::source_hash()
        .map_err(|err| fault::line("library", &format!("node={node} op={}", O::ID), &err))?;
    let base = px_graph_schema::node_key(
        &OpId {
            id: O::ID,
            interface,
            source_hash,
        },
        &params_json,
        |hasher| inputs.collect(hasher),
    );
    let key = base;
    let subject = fault::node_subject(node, O::ID, &key);

    if let Some(payload) = cache.fetch(key) {
        let value = <O::Payload as Build>::decode(&payload, node)
            .map_err(|err| fault::line("payload", &subject, &err))?;
        let detail = <O::Payload as Build>::detail(&value);
        cache
            .store(
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
            )
            .map_err(|err| fault::line("write", &subject, &err))?;
        return Ok(Cooked::new(key, value, true, 0, payload.bytes()));
    }

    let started = Instant::now();
    let value = f
        .render(&params, &inputs)
        .map_err(|err| fault::line("operator", &subject, &err))?;
    let millis = started.elapsed().as_millis() as u64;
    let payload = <O::Payload as Build>::encode(&value)
        .map_err(|err| fault::line("payload", &subject, &err))?;
    let detail = <O::Payload as Build>::detail(&value);
    cache
        .store(
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
        )
        .map_err(|err| fault::line("write", &subject, &err))?;
    Ok(Cooked::new(key, value, false, millis, payload.bytes()))
}

pub fn node_params<P>(cache: &dyn Cache, node: &str) -> Result<P, String>
where
    P: serde::Serialize + serde::de::DeserializeOwned + Default,
{
    let text = cache.params_text(node);
    let from_file = text.is_some();
    let (parsed, json) = canonical_params::<P>(text.as_deref())
        .map_err(|err| fault::line("params", &format!("node={node}"), &err))?;
    cache.record_params(node, "", &json, from_file);
    Ok(parsed)
}

#[macro_export]
macro_rules! px_local_op {
    ($(#[$meta:meta])* $name:ident, $id:literal, $params:ty, $inputs:ty, $payload:ty,
     |$p:ident, $i:ident| $body:expr) => {
        $(#[$meta])*
        impl ::px_graph_schema::PxOp for $name {
            const ID: &'static str = $id;
            const LIB: &'static str = "";
            const SYMBOL: &'static str = ::core::stringify!($name);

            type Params = $params;
            type Inputs = $inputs;
            type Payload = $payload;

            fn new() -> Self {
                $name
            }

            fn source_hash() -> ::core::result::Result<&'static str, ::std::string::String> {
                ::core::result::Result::Ok(env!("PX_SOURCE_HASH"))
            }

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
