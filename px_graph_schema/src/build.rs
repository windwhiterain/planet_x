use std::collections::BTreeMap;

use px_protocol::art::{
    CUBE_FACES, MeshData, PolylineData, TextureData, TextureFormat, VolumeData,
};
use px_protocol::payload::PayloadBundle;
use px_protocol::wire::{Blob, BlobHeader, DType};

pub trait Build: Sized {
    fn detail(payload: &Self) -> String;
    fn encode(payload: &Self) -> Result<PayloadBundle, String>;
    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String>;
}

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
            bytes: bundle.one()?.bytes.clone(),
        })
    }
}

impl Build for VolumeData {
    fn detail(payload: &Self) -> String {
        let (min, max, mean) = volume_stats(payload);
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
        TextureData {
            width: 4,
            height: 4,
            layers: 6,
            levels: 1,
            format: TextureFormat::Rgba16Float,
            bytes: (0..768).map(|index| (index % 251) as u8).collect(),
        }
    }

    #[test]
    fn encoding_and_decoding_round_trips_byte_for_byte() {
        let original = sample();
        let bundle = TextureData::encode(&original).expect("编码");
        let back = TextureData::decode(&bundle, "sky").expect("解码");
        assert_eq!(back, original);
        assert_eq!(back.bytes, original.bytes, "字节必须逐字相同");
    }

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

    #[test]
    #[should_panic(expected = "贴图载荷与形状不符")]
    fn a_payload_of_the_wrong_length_is_rejected() {
        let _ = TextureData::new(4, 4, CUBE_FACES, 1, TextureFormat::Rgba16Float, vec![0; 10]);
    }

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
