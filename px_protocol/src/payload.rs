//! 载荷的**线格式**：`PayloadBundle` 就是 CAS 里那份文件的形状（清单帧 + 数据块），
//! 外加「各域怎么把类型化的值编成它 / 从它解回来 / 说它长什么样」的那条契约 [`Build`]。
//!
//! ⚠ 它住这一层，是因为**载荷类型就住这一层**（[`crate::art::VolumeData`] / [`crate::art::MeshData`]）：
//!   `impl Build for VolumeData` 只有在这里才合法（trait 与类型都是本地的，孤儿规则）。
//!   场域不一样 —— `Field` 住 `px_field_schema`，所以那一份 impl 在那边。
//!   两边是同一条口径：**一个域的载荷编解码，与它的载荷类型住在一起。**
//!
//! 只有三样是**驱动**才知道的东西不进载荷：节点名（`id`）、相机表、指纹
//! ⇒ 算子那一侧只交出**无名、无相机**的载荷，驱动 `to_bytes(node, cameras)` 把它补成产物。

use std::collections::BTreeMap;

use crate::art::{
    ArtBundle, AssetKind, AssetManifest, CUBE_FACES, Camera, Domain, MeshData, TextureData,
    TextureFormat, TextureShape, VolumeData,
};
use crate::fnv::fnv1a;
use crate::stream::{self, Frame};
use crate::wire::{Blob, BlobHeader, DType};

/// 每个域的「怎么编自己的产物 / 怎么解上游载荷 / 怎么说自己长什么样」。
///
/// ⚠ 它**实现在载荷类型旁边**（见文件头那条口径）：于是驱动不认识任何域，
///   而类型化的值也不必跨边界 —— 谁的值谁说话。
pub trait Build: Sized {
    /// **产物里带不带评审相机表？** 场/网格带，体积不带。
    ///
    /// ⚠ 就是这一条 —— 域**就是**这个类型：`O::Payload = VolumeData` 已经说明了一切，
    ///   再声明一次只会多一个能写错的地方。
    const WITH_CAMERAS: bool;
    /// **这个域的产物尺寸是不是就是画布？**
    ///
    /// * `true`（场）：分辨率 = 画布 ⇒ 画布必须进键，否则"改了画布却命中旧分辨率"。
    /// * `false`（体积/网格）：自己的分辨率由参数给 ⇒ 画布与产物无关，
    ///   掺进去只会让"改画布"连带重烘它们。
    const RESOLUTION_IS_CANVAS: bool;
    /// 清单里那一行读数：**域自己说这份产物长什么样**。
    fn detail(payload: &Self) -> String;
    /// 编成载荷（**无名、无相机** —— 那两样由驱动落盘时补）。
    fn encode(payload: &Self) -> Result<PayloadBundle, String>;
    /// 载荷 → 类型化的值。场载荷不含投影，所以要 `projection` 补回去。
    fn decode(bundle: &PayloadBundle, projection: Domain, node: &str) -> Result<Self, String>;
}

pub struct PayloadBundle {
    pub kind: AssetKind,
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<Blob>,
}

impl PayloadBundle {
    pub fn new(kind: AssetKind, params: BTreeMap<String, f64>, blobs: Vec<Blob>) -> Self {
        Self {
            kind,
            params,
            blobs,
        }
    }

    /// 载荷内容的指纹（清单里那一格）。
    pub fn fingerprint(&self, id: &str) -> u64 {
        payload_fingerprint(id, &self.blobs)
    }

    /// 数据本身的字节数（不含清单帧与块头）—— 读数用。
    pub fn bytes(&self) -> usize {
        self.blobs.iter().map(|blob| blob.bytes.len()).sum()
    }

    /// 包成产物字节。`id` / 相机表 / 指纹是驱动补上的那三样。
    pub fn to_bytes(&self, id: &str, cameras: &[Camera]) -> Result<Vec<u8>, String> {
        let bundle = ArtBundle {
            assets: vec![AssetManifest {
                id: id.to_string(),
                kind: self.kind,
                params: self.params.clone(),
                blobs: self.blobs.iter().map(|blob| blob.header.clone()).collect(),
                fingerprint: self.fingerprint(id),
                cameras: cameras.to_vec(),
            }],
        };
        let mut frames = vec![Frame::Art(bundle)];
        frames.extend(self.blobs.iter().cloned().map(Frame::Blob));
        let mut out = Vec::new();
        stream::write_stream(&mut out, &frames).map_err(|err| err.to_string())?;
        Ok(out)
    }

    /// 从产物字节（CAS 里那份）还原。清单帧必须在最前面，与读法一致。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = bytes;
        let frames = stream::read_stream(&mut cursor).map_err(|err| err.to_string())?;
        let (kind, params) = frames
            .iter()
            .find_map(|frame| match frame {
                Frame::Art(bundle) => bundle
                    .assets
                    .first()
                    .map(|asset| (asset.kind, asset.params.clone())),
                _ => None,
            })
            .ok_or_else(|| "产物里没有清单帧（`Frame::Art`）".to_string())?;
        let blobs: Vec<Blob> = frames
            .iter()
            .filter_map(|frame| match frame {
                Frame::Blob(blob) => Some(blob.clone()),
                _ => None,
            })
            .collect();
        if blobs.is_empty() {
            return Err("产物里没有数据块".to_string());
        }
        Ok(Self {
            kind,
            params,
            blobs,
        })
    }

    pub fn one(&self) -> Result<&Blob, String> {
        self.blobs
            .first()
            .ok_or_else(|| "产物里没有数据块".to_string())
    }
}

/// 载荷内容的 FNV-1a。节点名也混进去，免得两个载荷相同的节点指纹撞上。
pub fn payload_fingerprint(id: &str, blobs: &[Blob]) -> u64 {
    let mut hash = fnv1a(id);
    for blob in blobs {
        for byte in &blob.bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(crate::fnv::FNV_PRIME);
        }
    }
    hash
}

/// **贴图**这一域的编解码与读数。
///
/// ⚠ 形状（宽高 / 层数 / mip 级数 / 格式）走**清单参数**，字节走 blob：
///   形状不是 blob 头那一档（`DType`）能表达的东西（"每通道 1 字节 sRGB"与
///   "每通道半精度线性"都是 U8/U16，但语义完全不同）。
///
/// ⚠ 字节**一个都不重排**：贴图载荷与渲染器读的那串逐字节相同 ⇒ "编一下再解回来"是恒等。
///   这是内容寻址（键 = 完整产物字节）的前提：一挪动字节，"同一份内容"就会有两个键，
///   而症状只是"每次都重烘"。
///
/// ⚠ `WITH_CAMERAS = false`：贴图是**天空/数据**那类东西，与评审相机表无关。
impl Build for TextureData {
    /// ⚠ 贴图是**天空/数据**那类东西，与评审相机表无关（相机表是网格与场那一档的事）。
    const WITH_CAMERAS: bool = false;
    /// ⚠ 贴图的分辨率由参数（`face` / `width`）给，**不是**画布 ⇒ 画布不进键。
    ///   掺进去会让"改画布"连带重烘一张本来就一样的贴图。
    const RESOLUTION_IS_CANVAS: bool = false;

    fn detail(payload: &Self) -> String {
        format!(
            "{}×{}×{} 层｜{} 级 mip｜{}｜{:.1} KB",
            payload.width,
            payload.height,
            payload.layers,
            payload.levels,
            payload.format.name(),
            payload.bytes.len() as f64 / 1024.0,
        )
    }

    /// ⚠ 形状（宽高 / 层数 / mip 级数 / 格式）走**清单参数**，字节走 blob：
    ///   形状不是 blob 头那一档（`DType`）能表达的东西（"每通道 1 字节 sRGB"与
    ///   "每通道半精度线性"都是 U8/U16，但语义完全不同）。
    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(PayloadBundle::new(
            AssetKind::Texture,
            BTreeMap::from([
                ("width".to_string(), f64::from(payload.width)),
                ("height".to_string(), f64::from(payload.height)),
                ("layers".to_string(), f64::from(payload.layers)),
                ("levels".to_string(), f64::from(payload.levels)),
                ("format".to_string(), payload.format.code()),
            ]),
            vec![
                Blob::new(
                    BlobHeader {
                        dtype: match payload.format {
                            TextureFormat::Rgba8Srgb => DType::U8,
                            TextureFormat::Rgba16Float => DType::U16,
                        },
                        // ⚠ 形状写作"字节数 / 2"只为满足 blob 头的自检（它要按 `DType` 算长度）；
                        //   真正的形状在清单参数里。
                        shape: vec![(payload.bytes.len() / 2) as u32],
                    },
                    payload.bytes.clone(),
                )
                .map_err(|err| err.to_string())?,
            ],
        ))
    }

    fn decode(bundle: &PayloadBundle, _projection: Domain, node: &str) -> Result<Self, String> {
        let number = |key: &str| -> Result<u32, String> {
            bundle
                .params
                .get(key)
                .copied()
                .map(|value| value as u32)
                .ok_or_else(|| format!("贴图 {node} 的清单里没有 `{key}`"))
        };
        let format = bundle
            .params
            .get("format")
            .copied()
            .and_then(TextureFormat::from_code)
            .ok_or_else(|| format!("贴图 {node} 的 `format` 认不出来"))?;
        Ok(Self {
            width: number("width")?,
            height: number("height")?,
            layers: number("layers")?,
            levels: number("levels")?,
            format,
            // ⚠ 字节**一个都不重排**（`Blob::new` 收的就是原始 `Vec<u8>`）⇒
            //   "编一下再解回来"是恒等，这是内容寻址的前提。
            bytes: bundle.one()?.bytes.clone(),
        })
    }
}

#[cfg(test)]
mod texture_tests {
    use super::*;

    
    fn sample() -> TextureData {
        // 4×4×6 层、1 级、半精度 ⇒ 4×4×6×8 = 768 字节。
        TextureData {
            width: 4,
            height: 4,
            layers: 6,
            levels: 1,
            format: TextureFormat::Rgba16Float,
            bytes: (0..768).map(|index| (index % 251) as u8).collect(),
        }
    }

    /// **编一下再解回来是恒等**（逐字节）。
    ///
    /// ⚠ 这是内容寻址的地基：键是"完整产物字节"的 blake3，如果编解码会把字节挪动，
    ///   "同一份内容"就会有两个键 —— 缓存从此不命中，而症状只是"每次都重烘"。
    #[test]
    fn encoding_and_decoding_round_trips_byte_for_byte() {
        let original = sample();
        let bundle = TextureData::encode(&original).expect("编码");
        let back = TextureData::decode(&bundle, Domain::CubeMap, "sky").expect("解码");
        assert_eq!(back, original);
        assert_eq!(back.bytes, original.bytes, "字节必须逐字相同");
    }

    /// **形状进清单**（不是藏在字节里）：解码方不看 blob 头也能知道这张图多大。
    #[test]
    fn the_shape_travels_in_the_manifest() {
        let bundle = TextureData::encode(&sample()).expect("编码");
        assert_eq!(bundle.kind, AssetKind::Texture);
        for key in ["width", "height", "layers", "levels", "format"] {
            assert!(bundle.params.contains_key(key), "清单里缺 `{key}`");
        }
        let back = TextureData::decode(&bundle, Domain::CubeMap, "sky").expect("解码");
        assert_eq!(
            (back.width, back.height, back.layers, back.levels),
            (4, 4, 6, 1)
        );
        assert_eq!(back.format, TextureFormat::Rgba16Float);
    }

    /// **形状与字节数对不上就当场炸**（不静默产出一份采样会越界的贴图）。
    #[test]
    #[should_panic(expected = "贴图载荷与形状不符")]
    fn a_payload_of_the_wrong_length_is_rejected() {
        let _ = TextureData::new(4, 4, CUBE_FACES, 1, TextureFormat::Rgba16Float, vec![0; 10]);
    }
}

/// **体积**这一域的编解码与读数。
///
/// ⚠ 体积有特别之处：`inner`/`outer` 住在**清单参数**里，不在 blob 里
/// （`VolumeData::from_blob` 只还原数组）⇒ `decode` 必须把清单一起看。
impl Build for VolumeData {
    /// 体积没人看（渲染器只读 mesh）⇒ 不掺评审相机。
    const WITH_CAMERAS: bool = false;
    /// 分辨率由参数（`res`/`layers`）给 ⇒ 画布与它无关。
    const RESOLUTION_IS_CANVAS: bool = false;

    fn detail(payload: &Self) -> String {
        let (min, max, mean) = volume_stats(payload);
        format!(
            "{} 面 × {}² × {} 层｜值域 {min:.4}..{max:.4}｜均值 {mean:.4}",
            CUBE_FACES, payload.res, payload.layers,
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(PayloadBundle::new(
            AssetKind::Volume,
            BTreeMap::from([
                ("res".to_string(), payload.res as f64),
                ("layers".to_string(), payload.layers as f64),
                ("inner".to_string(), payload.inner as f64),
                ("outer".to_string(), payload.outer as f64),
            ]),
            payload.blobs(),
        ))
    }

    fn decode(bundle: &PayloadBundle, _projection: Domain, node: &str) -> Result<Self, String> {
        let _ = node;
        let mut volume = VolumeData::from_blob(bundle.one()?).map_err(|err| err.to_string())?;
        volume.inner = bundle.params.get("inner").copied().unwrap_or(0.0) as f32;
        volume.outer = bundle.params.get("outer").copied().unwrap_or(0.0) as f32;
        Ok(volume)
    }
}

/// ⚠ 逐元素 + 用 f64 累加：与老路径 `volume_stats` 逐字一致。
fn volume_stats(payload: &VolumeData) -> (f64, f64, f64) {
    let (mut min, mut max, mut sum) = (f32::INFINITY, f32::NEG_INFINITY, 0.0_f64);
    for value in &payload.data {
        min = min.min(*value);
        max = max.max(*value);
        sum += *value as f64;
    }
    let mean = if payload.data.is_empty() {
        0.0
    } else {
        sum / payload.data.len() as f64
    };
    (min as f64, max as f64, mean)
}

/// **网格**这一域的编解码与读数：四个 blob（位置 / 法线 / uv / 索引）
/// + 清单里的顶点与三角形数。
impl Build for MeshData {
    const WITH_CAMERAS: bool = true;
    const RESOLUTION_IS_CANVAS: bool = false;

    fn detail(payload: &Self) -> String {
        format!(
            "{} 顶点 / {} 三角形",
            payload.vertices(),
            payload.triangles()
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(PayloadBundle::new(
            AssetKind::Mesh,
            BTreeMap::from([
                ("vertices".to_string(), payload.vertices() as f64),
                ("triangles".to_string(), payload.triangles() as f64),
            ]),
            payload.blobs(),
        ))
    }

    fn decode(bundle: &PayloadBundle, _projection: Domain, node: &str) -> Result<Self, String> {
        let _ = node;
        let blobs: Vec<&Blob> = bundle.blobs.iter().collect();
        MeshData::from_blobs(&blobs).map_err(|err| err.to_string())
    }
}
