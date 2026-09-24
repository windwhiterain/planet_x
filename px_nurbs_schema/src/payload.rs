//! **NURBS 载荷的编解码与读数**：`f64` 数组 ↔ blob、清单里的形状，以及三个
//! [`Build`] 实现 —— 话由域自己说（契约是 `px_graph_schema::Build`）。
//!
//! ⚠ 为什么数组**原样走 blob**（而不是塞进清单参数）：清单参数只有 `f64` 一格一个名
//!   （`BTreeMap<String, f64>`），控制点动辄成百上千 ⇒ 挤进去既写不出又读不快。
//!   形状（次数 / 点数 / 节点数 / 有没有权）走清单参数，数组走 blob —— 与 `VolumeData`
//!   的 `inner`/`outer` 同一个口径。
//!
//! ⚠ 数**一个都不重排**：`f64` 按小端原样写，读回来逐位相同（内容寻址的地基）。
//!
//! ⚠ **载荷里没有"资产种类"那一栏**：缓存按代码位置取载荷，类型是 `O::Payload`
//!   （编译期已知）；而这三个类型**不进场景**（渲染器不读它们 —— 要看得先细分，
//!   细分吐的是 `MeshData`）⇒ 没有任何消费者需要"按种类认出这坨字节"。

use std::collections::BTreeMap;

use px_graph_schema::PayloadBundle;
use px_protocol::wire::{Blob, BlobHeader, DType};

use crate::curve::Curve;
use crate::point::PointData;
use crate::surface::Surface;

/// 一段 `f64` → 一个 blob（`name` 只进报错，不参与解码）。
pub fn blob(name: &str, values: &[f64]) -> Result<Blob, String> {
    let mut bytes = Vec::with_capacity(values.len() * 8);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Blob::new(
        BlobHeader {
            dtype: DType::F64,
            shape: vec![values.len() as u32],
        },
        bytes,
    )
    .map_err(|err| format!("{name} 那一块：{err}"))
}

/// 一个 blob → 一段 `f64`。
pub fn values(bundle: &PayloadBundle, index: usize) -> Result<Vec<f64>, String> {
    let blob = bundle.blobs.get(index).ok_or_else(|| {
        format!(
            "载荷里缺第 {} 块（一共 {} 块）",
            index + 1,
            bundle.blobs.len()
        )
    })?;
    if blob.header.dtype != DType::F64 {
        return Err(format!(
            "第 {} 块是 {:?}，NURBS 的数组一律是 F64",
            index + 1,
            blob.header.dtype
        ));
    }
    Ok(blob
        .bytes
        .chunks_exact(8)
        .map(|chunk| {
            f64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ])
        })
        .collect())
}

/// 清单参数里取一个计数。
pub fn count(params: &BTreeMap<String, f64>, key: &str) -> Result<usize, String> {
    params
        .get(key)
        .copied()
        .map(|value| value as usize)
        .ok_or_else(|| format!("载荷清单里没有 `{key}`"))
}

/// 一块 `f64` 的长度与一个计数对不上 ⇒ 当场拒（不静默截断）。
pub fn expect_len(values: &[f64], want: usize, what: &str) -> Result<(), String> {
    if values.len() != want {
        return Err(format!(
            "{what} 有 {} 个数，清单说应当是 {want} 个 —— 载荷与形状对不上",
            values.len()
        ));
    }
    Ok(())
}

/// 曲线：blob 顺序 = 控制点 / 节点 /（有权才有）权。
pub fn encode_curve(curve: &Curve) -> Result<PayloadBundle, String> {
    curve.check()?;
    let mut blobs = vec![
        blob("控制点", &curve.control)?,
        blob("节点向量", &curve.knots)?,
    ];
    let rational = !curve.weights.is_empty();
    if rational {
        blobs.push(blob("权", &curve.weights)?);
    }
    let mut params = BTreeMap::from([
        ("degree".to_string(), curve.degree as f64),
        ("count".to_string(), curve.count() as f64),
        ("knots".to_string(), curve.knots.len() as f64),
    ]);
    params.insert("rational".to_string(), if rational { 1.0 } else { 0.0 });
    Ok(PayloadBundle::new(params, blobs))
}

pub fn decode_curve(bundle: &PayloadBundle) -> Result<Curve, String> {
    let degree = count(&bundle.params, "degree")?;
    let points = count(&bundle.params, "count")?;
    let knots = count(&bundle.params, "knots")?;
    let control = values(bundle, 0)?;
    let knot_vector = values(bundle, 1)?;
    expect_len(&control, points * 3, "曲线的控制点")?;
    expect_len(&knot_vector, knots, "曲线的节点向量")?;
    let rational = bundle.params.get("rational").copied().unwrap_or(0.0) != 0.0;
    let weights = if rational {
        let weights = values(bundle, 2)?;
        expect_len(&weights, points, "曲线的权")?;
        weights
    } else {
        Vec::new()
    };
    Curve::new(degree, control, weights, knot_vector)
}

/// 曲面：blob 顺序 = 控制点 / u 节点 / v 节点 /（有权才有）权。
pub fn encode_surface(surface: &Surface) -> Result<PayloadBundle, String> {
    surface.check()?;
    let mut blobs = vec![
        blob("控制点", &surface.control)?,
        blob("u 节点向量", &surface.knots_u)?,
        blob("v 节点向量", &surface.knots_v)?,
    ];
    let rational = !surface.weights.is_empty();
    if rational {
        blobs.push(blob("权", &surface.weights)?);
    }
    let mut params = BTreeMap::from([
        ("degree_u".to_string(), surface.degree.0 as f64),
        ("degree_v".to_string(), surface.degree.1 as f64),
        ("nu".to_string(), surface.nu as f64),
        ("nv".to_string(), surface.nv as f64),
        ("knots_u".to_string(), surface.knots_u.len() as f64),
        ("knots_v".to_string(), surface.knots_v.len() as f64),
    ]);
    params.insert("rational".to_string(), if rational { 1.0 } else { 0.0 });
    Ok(PayloadBundle::new(params, blobs))
}

pub fn decode_surface(bundle: &PayloadBundle) -> Result<Surface, String> {
    let degree = (
        count(&bundle.params, "degree_u")?,
        count(&bundle.params, "degree_v")?,
    );
    let nu = count(&bundle.params, "nu")?;
    let nv = count(&bundle.params, "nv")?;
    let knots_u = count(&bundle.params, "knots_u")?;
    let knots_v = count(&bundle.params, "knots_v")?;
    let control = values(bundle, 0)?;
    let vector_u = values(bundle, 1)?;
    let vector_v = values(bundle, 2)?;
    expect_len(&control, nu * nv * 3, "曲面的控制点")?;
    expect_len(&vector_u, knots_u, "曲面的 u 节点向量")?;
    expect_len(&vector_v, knots_v, "曲面的 v 节点向量")?;
    let rational = bundle.params.get("rational").copied().unwrap_or(0.0) != 0.0;
    let weights = if rational {
        let weights = values(bundle, 3)?;
        expect_len(&weights, nu * nv, "曲面的权")?;
        weights
    } else {
        Vec::new()
    };
    Surface::new(degree, nu, nv, control, weights, vector_u, vector_v)
}

impl px_graph_schema::Build for Curve {
    /// 控制点由参数给 ⇒ 画布与它无关。

    fn detail(payload: &Self) -> String {
        let (low, high) = payload.domain();
        format!(
            "{} 次{}曲线｜{} 个控制点 / {} 个节点｜参数 {low:.3}..{high:.3}",
            payload.degree,
            if payload.weights.is_empty() {
                "非有理"
            } else {
                "有理"
            },
            payload.count(),
            payload.knots.len(),
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        encode_curve(payload)
    }

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
        decode_curve(bundle).map_err(|err| format!("{node}：{err}"))
    }
}

impl px_graph_schema::Build for Surface {
    fn detail(payload: &Self) -> String {
        format!(
            "{}×{} 次{}曲面｜{}×{} 控制点｜节点 {} / {}",
            payload.degree.0,
            payload.degree.1,
            if payload.weights.is_empty() {
                "非有理"
            } else {
                "有理"
            },
            payload.nu,
            payload.nv,
            payload.knots_u.len(),
            payload.knots_v.len(),
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        encode_surface(payload)
    }

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
        decode_surface(bundle).map_err(|err| format!("{node}：{err}"))
    }
}

impl px_graph_schema::Build for PointData {
    fn detail(payload: &Self) -> String {
        payload.detail()
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(PayloadBundle::new(
            BTreeMap::new(),
            vec![blob("读数", &payload.values())?],
        ))
    }

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
        let values = values(bundle, 0).map_err(|err| format!("{node}：{err}"))?;
        PointData::from_values(&values).map_err(|err| format!("{node}：{err}"))
    }
}
