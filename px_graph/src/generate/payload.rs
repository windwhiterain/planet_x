//! **贴图域的契约实现**：`TextureData` 的编解码与读数。
//!
//! ⚠ **为什么它住这里**（而不是像体积/网格那样住 `px_protocol::payload`）：
//!   那条口径是"一个域的载荷编解码，与它的载荷类型住在一起"（孤儿规则）。
//!   体积与网格的载荷类型在 `px_protocol::art`，所以它们的 `Build` 在那里；
//!   而 `TextureData` 在**本 crate**（`generate::texture`）⇒ 它的 `Build` 只能在这里。
//!   两边是同一条口径，落点由"类型住哪"决定。
//!
//! 有了它，**贴图就是一个正常的图产物**：`sky.nebula` 这类算子可以直接把
//!   `TextureData` 当 `Payload` 交出去，驱动照常进键、落盘、登记清单
//!   —— 于是场景文档的 `environment.skybox = "图名::节点名"` 取得到它，
//!   渲染器一个字节都不用改。

use std::collections::BTreeMap;

use px_graph_schema::payload::{Build, PayloadBundle};
use px_protocol::art::{AssetKind, Domain, TextureFormat};
use px_protocol::wire::{Blob, BlobHeader, DType};

use super::texture::TextureData;

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
}
