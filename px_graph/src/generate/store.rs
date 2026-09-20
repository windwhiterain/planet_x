//! 落盘与读回：按清单里的键读一份场（`load_field`），以及把生成物写进 CAS
//! （`Generated` / `write_cas` / `write_texture[_at]` / `write_generated_mesh[_at]` /
//! `fingerprint_of`）。
//!
//! 边界：这一档只认字节与路径，不认调色板也不认 mip —— 贴图/网格怎么生成在
//! `super::texture` 与 `super::mesh`，这里只负责"算键、比形状、写文件、说清命中没命中"。
//! 键 = 完整产物字节的 blake3、路径 = `px_protocol::scene::cas_path`，与渲染器无关，
//! 所以读回的那份场也能在没开图的时候用（`super::shade` 的取数口径就靠它）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use px_field_schema::field::Field;
use px_graph_schema::Key;
use px_protocol::art::{
    ArtBundle, AssetKind, AssetManifest, Domain, MeshData, TextureFormat, TextureShape,
};
use px_protocol::stream::{self, Frame};
use px_protocol::wire::{Blob, BlobHeader, DType};

// ---------------------------------------------------------------------------
// 产物读取：按清单里的键读一份场（读法与渲染器 `load_field` 相同）
// ---------------------------------------------------------------------------

/// 读一份场产物。投影由清单里的 `AssetKind` 决定 —— 与渲染器 `planet::load_field`
/// 同一套映射（`Field2D` → Equirect、`OctahedralField` → Octahedral、
/// `CubeField` → Cube、`CubeMap` → CubeMap）。**这一步不能省**：投影决定
/// `texel_latitude` 走哪一支、颜色贴图走 `mip_chain` 还是 `mip_chain_cube`、
/// 要不要 `pole_cap_filter`。
pub fn load_field(path: &str) -> Result<Field, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let frames = stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;

    let mut kind = None;
    for frame in &frames {
        if let Frame::Art(bundle) = frame {
            if let Some(asset) = bundle.assets.first() {
                kind = Some(asset.kind);
            }
        }
    }
    let projection = match kind {
        Some(AssetKind::Field2D) => Domain::Equirect,
        Some(AssetKind::OctahedralField) => Domain::Octahedral,
        Some(AssetKind::CubeField) => Domain::Cube,
        Some(AssetKind::CubeMap) => Domain::CubeMap,
        Some(other) => {
            return Err(format!(
                "{path} 是 {other:?}，星球需要 Field2D / OctahedralField / CubeField / CubeMap 产物"
            ));
        }
        None => return Err(format!("{path} 里没有 Art 帧")),
    };

    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .ok_or_else(|| format!("{path} 里没有数据块"))?;
    if blob.header.shape.len() != 2 {
        return Err(format!("{path} 的场不是二维的：{:?}", blob.header.shape));
    }

    let mut field = Field::from_blob(blob).map_err(|err| err.to_string())?;
    field.projection = projection;
    Ok(field)
}

// ---------------------------------------------------------------------------
// 落盘：内容寻址（键 = 完整产物字节的 blake3）
// ---------------------------------------------------------------------------

/// 一份落进 CAS 的生成物。
#[derive(Debug, Clone)]
pub struct Generated {
    pub id: String,
    pub key: Key,
    pub path: PathBuf,
    pub bytes: u64,
    /// 这份内容在 CAS 里已经有了（没重写文件）。
    pub hit: bool,
    pub millis: u64,
}

/// 把一份生成物写进 CAS。
///
/// **键 = 内容**：键是对完整产物字节（清单帧 + 载荷帧，就是 `stream::write_stream`
/// 的输出）算的 blake3，路径是 `px_protocol::scene::cas_path(cache_root, hex)`。
/// 文件已存在 ⇒ `hit = true` 且一个字节都不重写。
fn write_cas(
    root: &Path,
    id: &str,
    kind: AssetKind,
    params: BTreeMap<String, f64>,
    blobs: Vec<Blob>,
) -> Result<Generated, String> {
    let started = Instant::now();
    let fingerprint = crate::payload_fingerprint(id, &blobs);
    let bundle = ArtBundle {
        assets: vec![AssetManifest {
            id: id.to_string(),
            kind,
            params,
            blobs: blobs.iter().map(|blob| blob.header.clone()).collect(),
            fingerprint,
            cameras: Vec::new(),
        }],
    };
    let mut frames = vec![Frame::Art(bundle)];
    frames.extend(blobs.into_iter().map(Frame::Blob));

    let mut out = Vec::new();
    stream::write_stream(&mut out, &frames).map_err(|err| err.to_string())?;

    let key = *blake3::hash(&out).as_bytes();
    let path = px_protocol::scene::cas_path(root, &crate::hex(&key))?;
    let hit = path.exists();
    if !hit {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(&path, &out).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
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

/// 把一份贴图写进 CAS：清单帧（`kind = AssetKind::Texture`、
/// `params = TextureShape::params()`、`fingerprint` = 对载荷算的 FNV-1a 指纹）
/// + 一个 blob（`Rgba8Srgb` → `DType::U8`；`Rgba16Float` → `DType::U16`，字节原样）。
///
/// 要一个 CAS 根：默认走 [`crate::cache_root`]（纯函数，与开不开图无关）；
/// 不方便依赖它时用 [`write_texture_at`] 显式给一个根。
pub fn write_texture(
    id: &str,
    shape: TextureShape,
    payload: &[u8],
    dtype: DType,
) -> Result<Generated, String> {
    write_texture_at(&crate::cache_root(), id, shape, payload, dtype)
}

/// [`write_texture`] 的显式根变体。
pub fn write_texture_at(
    root: &Path,
    id: &str,
    shape: TextureShape,
    payload: &[u8],
    dtype: DType,
) -> Result<Generated, String> {
    let expected = shape.chain_bytes();
    if payload.len() != expected {
        return Err(format!(
            "贴图载荷 {id} 是 {} 字节，形状（{}×{}×{} 层、{} 级、{:?}）说应当是 {expected} 字节",
            payload.len(),
            shape.width,
            shape.height,
            shape.layers,
            shape.levels,
            shape.format,
        ));
    }
    let wanted = match shape.format {
        TextureFormat::Rgba8Srgb => DType::U8,
        TextureFormat::Rgba16Float => DType::U16,
    };
    if dtype != wanted {
        return Err(format!(
            "贴图载荷 {id} 的格式是 {:?}，位深应当是 {wanted:?}，实际给了 {dtype:?}",
            shape.format
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
    .map_err(|err| err.to_string())?;

    write_cas(root, id, AssetKind::Texture, shape.params(), vec![blob])
}

/// 网格产物同理（`kind = AssetKind::Mesh`，用 `MeshData::blobs()`），供环用。
///
/// 要一个 CAS 根：默认走 [`crate::cache_root`]；不方便依赖它时用 [`write_generated_mesh_at`]。
pub fn write_generated_mesh(id: &str, mesh: &MeshData) -> Result<Generated, String> {
    write_generated_mesh_at(&crate::cache_root(), id, mesh)
}

/// [`write_generated_mesh`] 的显式根变体。
pub fn write_generated_mesh_at(
    root: &Path,
    id: &str,
    mesh: &MeshData,
) -> Result<Generated, String> {
    let params = BTreeMap::from([
        ("vertices".to_string(), mesh.vertices() as f64),
        ("triangles".to_string(), mesh.triangles() as f64),
    ]);
    write_cas(root, id, AssetKind::Mesh, params, mesh.blobs())
}

/// 一份生成物的指纹（与 `px_graph::write_artifact` 用的是同一个函数）。审计/对账用。
pub fn fingerprint_of(id: &str, blobs: &[Blob]) -> u64 {
    crate::payload_fingerprint(id, blobs)
}
