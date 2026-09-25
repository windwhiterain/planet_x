use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use px_field_schema::field::Field;
use px_field_schema::params::Shape;
use px_graph_schema::Key;
use px_protocol::art::{ArtBundle, AssetManifest, MeshData, TextureFormat, TextureShape};
use px_protocol::payload::PayloadBundle;
use px_protocol::stream::{self, Frame};
use px_protocol::wire::{Blob, BlobHeader, DType};

pub fn load_field(path: &str) -> Result<Field, px_graph_schema::Fault> {
    let bytes = std::fs::read(path)
        .map_err(|err| px_graph_schema::Fault::read(format!("读不到 {path}：{err}")))?;
    let frames = stream::read_stream(&mut bytes.as_slice()).map_err(|err| {
        px_graph_schema::Fault::new(px_graph_schema::Kind::Payload, err.to_string())
    })?;

    let bundle = PayloadBundle::from_bytes(&bytes)
        .map_err(|err| px_graph_schema::Fault::new(px_graph_schema::Kind::Payload, err))?;
    let projection = px_field_schema::payload::stored_projection(&bundle).ok_or_else(|| {
        px_graph_schema::Fault::new(
            px_graph_schema::Kind::Shape,
            format!(
                "{path} 的场产物清单里没有 `{}`（投影）—— 它决定这一格在世界里的哪，读不了",
                px_field_schema::payload::PROJECTION_KEY,
            ),
        )
    })?;

    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .ok_or_else(|| {
            px_graph_schema::Fault::new(
                px_graph_schema::Kind::Payload,
                format!("{path} 里没有数据块"),
            )
        })?;
    if blob.header.shape.len() != 2 {
        return Err(px_graph_schema::Fault::new(
            px_graph_schema::Kind::Shape,
            format!("{path} 的场不是二维的：{:?}", blob.header.shape),
        ));
    }

    let mut field = Field::from_blob(blob).map_err(|err| {
        px_graph_schema::Fault::new(px_graph_schema::Kind::Payload, err.to_string())
    })?;
    field.projection = projection;
    Shape {
        width: field.width,
        height: field.height,
        projection,
    }
    .check()
    .map_err(|err| {
        px_graph_schema::Fault::new(px_graph_schema::Kind::Shape, format!("{path}：{err}"))
    })?;
    Ok(field)
}

#[derive(Debug, Clone)]
pub struct Generated {
    pub id: String,
    pub key: Key,
    pub path: PathBuf,
    pub bytes: u64,
    pub hit: bool,
    pub millis: u64,
}

fn write_cas(
    root: &Path,
    id: &str,
    params: BTreeMap<String, f64>,
    blobs: Vec<Blob>,
) -> Result<Generated, px_graph_schema::Fault> {
    let started = Instant::now();
    let fingerprint = crate::payload_fingerprint(id, &blobs);
    let bundle = ArtBundle {
        assets: vec![AssetManifest {
            id: id.to_string(),
            params,
            blobs: blobs.iter().map(|blob| blob.header.clone()).collect(),
            fingerprint,
        }],
    };
    let mut frames = vec![Frame::Art(bundle)];
    frames.extend(blobs.into_iter().map(Frame::Blob));

    let mut out = Vec::new();
    stream::write_stream(&mut out, &frames).map_err(|err| {
        px_graph_schema::Fault::new(px_graph_schema::Kind::Payload, err.to_string())
    })?;

    let key = *blake3::hash(&out).as_bytes();
    let path = px_protocol::scene::cas_path(root, &crate::hex(&key))?;
    let hit = path.exists();
    if !hit {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                px_graph_schema::Fault::new(px_graph_schema::Kind::Write, err.to_string())
            })?;
        }
        std::fs::write(&path, &out).map_err(|err| {
            px_graph_schema::Fault::new(
                px_graph_schema::Kind::Write,
                format!("写 {} 失败：{err}", path.display()),
            )
        })?;
    }
    Ok(Generated {
        id: id.to_string(),
        key,
        path,
        bytes: out.len() as u64,
        hit,
        millis: started.elapsed().as_millis() as u64,
    })
}

pub fn write_texture(
    id: &str,
    shape: TextureShape,
    payload: &[u8],
    dtype: DType,
) -> Result<Generated, px_graph_schema::Fault> {
    write_texture_at(&crate::cache_root(), id, shape, payload, dtype)
}

pub fn write_texture_at(
    root: &Path,
    id: &str,
    shape: TextureShape,
    payload: &[u8],
    dtype: DType,
) -> Result<Generated, px_graph_schema::Fault> {
    let expected = shape.chain_bytes();
    if payload.len() != expected {
        return Err(px_graph_schema::Fault::new(
            px_graph_schema::Kind::Payload,
            format!(
                "贴图载荷 {id} 是 {} 字节，形状（{}×{}×{} 层、{} 级、{:?}）说应当是 {expected} 字节",
                payload.len(),
                shape.width,
                shape.height,
                shape.layers,
                shape.levels,
                shape.format,
            ),
        ));
    }
    let wanted = match shape.format {
        TextureFormat::Rgba8Srgb => DType::U8,
        TextureFormat::Rgba16Float => DType::U16,
    };
    if dtype != wanted {
        return Err(px_graph_schema::Fault::new(
            px_graph_schema::Kind::Payload,
            format!(
                "贴图载荷 {id} 的格式是 {:?}，位深应当是 {wanted:?}，实际给了 {dtype:?}",
                shape.format
            ),
        ));
    }
    let elems = payload.len() / dtype.elem_size();
    let blob = Blob::new(
        BlobHeader {
            dtype,
            shape: vec![elems as u32],
        },
        payload.to_vec(),
    )
    .map_err(|err| px_graph_schema::Fault::new(px_graph_schema::Kind::Payload, err.to_string()))?;

    write_cas(root, id, shape.params(), vec![blob])
}

pub fn write_generated_mesh(
    id: &str,
    mesh: &MeshData,
) -> Result<Generated, px_graph_schema::Fault> {
    write_generated_mesh_at(&crate::cache_root(), id, mesh)
}

pub fn write_generated_mesh_at(
    root: &Path,
    id: &str,
    mesh: &MeshData,
) -> Result<Generated, px_graph_schema::Fault> {
    let params = BTreeMap::from([
        ("vertices".to_string(), mesh.vertices() as f64),
        ("triangles".to_string(), mesh.triangles() as f64),
    ]);
    write_cas(root, id, params, mesh.blobs())
}

pub fn fingerprint_of(id: &str, blobs: &[Blob]) -> u64 {
    crate::payload_fingerprint(id, blobs)
}
