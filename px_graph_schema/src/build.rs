//! **载荷的烘图契约**：一个域怎么把自己的值编成清单+数据块 / 怎么解回来 / 怎么说自己长什么样。
//!
//! ⚠⚠ 它**不住线格式那一层**（用户 2026-09-27 的裁定：拆开）。两半各归其位：
//!
//! * **线格式那一半**住 `px_protocol::payload`：`PayloadBundle` 就是 CAS 里那份文件的形状
//!   （清单帧 + 数据块），读写它的是**两侧**（烘图写、渲染读）⇒ 属于线格式；
//! * **烘图那一半**是这里：`WITH_CAMERAS`（评审相机表）、`detail`（清单读数）、
//!   `decode(…, node)`（节点名）—— 这三样都是**驱动**的概念，线格式层不该认识它们。
//!   从前它们挤在 `px_protocol` 里，于是"删一个参数"这种契约改动要去动线格式 crate。
//!
//! ⚠ 各域的 `impl` 仍然**跟着自己的载荷类型走**（孤儿规则）：类型住 `px_protocol::art` 的
//!   四个（贴图 / 体积 / 网格 / 折线）在这一份文件里；`Field` / `StarField` / `Curve` …
//!   住各自的 schema crate，那几份 `impl` 就在那边（`impl px_graph_schema::Build for Field`）。

use std::collections::BTreeMap;

use px_protocol::art::{
    CUBE_FACES, MeshData, PolylineData, TextureData, TextureFormat, VolumeData,
};
use px_protocol::payload::PayloadBundle;
use px_protocol::wire::{Blob, BlobHeader, DType};

/// 每个域的「怎么编自己的产物 / 怎么解上游载荷 / 怎么说自己长什么样」。
///
/// ⚠ 它**实现在载荷类型旁边**（见文件头那条口径）：于是驱动不认识任何域，
///   而类型化的值也不必跨边界 —— 谁的值谁说话。
pub trait Build: Sized {
    /// 清单里那一行读数：**域自己说这份产物长什么样**。
    fn detail(payload: &Self) -> String;
    /// 编成载荷（**无名** —— 那一样由驱动落盘时补）。
    fn encode(payload: &Self) -> Result<PayloadBundle, String>;
    /// 载荷 → 类型化的值。⚠ 不再有"从画布补一个投影"这一手：载荷自己说清楚
    /// （场把投影写进清单，见 `px_field_schema::payload`）。
    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String>;
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

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
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

/// **体积**这一域的编解码与读数。
///
/// ⚠ 体积有特别之处：`inner`/`outer` 住在**清单参数**里，不在 blob 里
/// （`VolumeData::from_blob` 只还原数组）⇒ `decode` 必须把清单一起看。
impl Build for VolumeData {
    fn detail(payload: &Self) -> String {
        let (min, max, mean) = volume_stats(payload);
        // ⚠ 多通道才印通道数（单通道的读数**逐字不变** —— 有快照判据钉着它）。
        let lanes = payload.lanes();
        let lane_text = if lanes > 1 {
            format!(" × {lanes} 通道")
        } else {
            String::new()
        };
        format!(
            "{} 面 × {}² × {} 层{lane_text}｜值域 {min:.4}..{max:.4}｜均值 {mean:.4}",
            CUBE_FACES, payload.res, payload.layers,
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(PayloadBundle::new(
            BTreeMap::from([
                ("res".to_string(), payload.res as f64),
                ("layers".to_string(), payload.layers as f64),
                ("inner".to_string(), payload.inner as f64),
                ("outer".to_string(), payload.outer as f64),
            ]),
            payload.blobs(),
        ))
    }

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
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
    fn detail(payload: &Self) -> String {
        format!(
            "{} 顶点 / {} 三角形",
            payload.vertices(),
            payload.triangles()
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(PayloadBundle::new(
            BTreeMap::from([
                ("vertices".to_string(), payload.vertices() as f64),
                ("triangles".to_string(), payload.triangles() as f64),
            ]),
            payload.blobs(),
        ))
    }

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
        let _ = node;
        let blobs: Vec<&Blob> = bundle.blobs.iter().collect();
        MeshData::from_blobs(&blobs).map_err(|err| err.to_string())
    }
}

/// **折线**这一域：两块 blob（位置 / 线段下标）+ 清单里的顶点与线段数。
///
impl Build for PolylineData {
    fn detail(payload: &Self) -> String {
        format!(
            "{} 顶点 / {} 段折线",
            payload.vertices(),
            payload.segments()
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(PayloadBundle::new(
            BTreeMap::from([
                ("vertices".to_string(), payload.vertices() as f64),
                ("segments".to_string(), payload.segments() as f64),
            ]),
            payload.blobs(),
        ))
    }

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
        let _ = node;
        let blobs: Vec<&Blob> = bundle.blobs.iter().collect();
        PolylineData::from_blobs(&blobs).map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
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
        let back = TextureData::decode(&bundle, "sky").expect("解码");
        assert_eq!(back, original);
        assert_eq!(back.bytes, original.bytes, "字节必须逐字相同");
    }

    /// **形状进清单**（不是藏在字节里）：解码方不看 blob 头也能知道这张图多大。
    #[test]
    fn the_shape_travels_in_the_manifest() {
        let bundle = TextureData::encode(&sample()).expect("编码");
        for key in ["width", "height", "layers", "levels", "format"] {
            assert!(bundle.params.contains_key(key), "清单里缺 `{key}`");
        }
        let back = TextureData::decode(&bundle, "sky").expect("解码");
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

    /// **多通道体积往返**（6 条通道的发射体积必须编得进 blob 形状）。
    ///
    /// ⚠⚠ 钉的就是这一档踩过的那个坑：形状只写 `[面, 层, t, s]`（单通道的量）、
    ///   字节却是 `samples × 6` ⇒ 产物**自相矛盾**、读回被"长度不符"拒收，
    ///   而症状只是"每次烘图都判未命中、每次重算"（不报错、不崩溃、画面对不对也看不出）。
    #[test]
    fn a_six_lane_volume_survives_the_round_trip() {
        let (res, layers) = (2_u32, 3_u32);
        let samples = (CUBE_FACES * layers * res * res) as usize;
        let data: Vec<f32> = (0..samples * 6).map(|index| index as f32 * 0.25).collect();
        let volume = VolumeData {
            res,
            layers,
            inner: 1.0,
            outer: 2.5,
            lanes: 6,
            data: data.clone(),
        };
        let bundle = <VolumeData as Build>::encode(&volume).expect("编得出来");
        let back = <VolumeData as Build>::decode(&bundle, "emission").expect("解得回来");
        assert_eq!(back.res, res);
        assert_eq!(back.layers, layers);
        assert_eq!(back.inner, 1.0);
        assert_eq!(back.outer, 2.5);
        assert_eq!(back.data.len(), data.len(), "字节数必须一致");
        for index in 0..data.len() {
            assert_eq!(
                back.data[index].to_bits(),
                data[index].to_bits(),
                "第 {index} 格不是逐位相同"
            );
        }
    }
}
