use std::collections::BTreeMap;

use px_graph_schema::PayloadBundle;
use px_protocol::art::Domain;

use crate::field::Field;

pub const PROJECTION_KEY: &str = "projection";

pub fn encode(field: &Field) -> PayloadBundle {
    PayloadBundle::new(
        BTreeMap::from([
            ("width".to_string(), field.width as f64),
            ("height".to_string(), field.height as f64),
            (
                PROJECTION_KEY.to_string(),
                f64::from(field.projection.code()),
            ),
        ]),
        vec![field.to_blob()],
    )
}

pub fn decode(bundle: &PayloadBundle) -> Result<Field, String> {
    let mut field = Field::from_blob(bundle.one()?).map_err(|err| err.to_string())?;
    field.projection = stored_projection(bundle).unwrap_or(crate::field::Projection::Equirect);
    // 载荷解码是产物进内存的入口：行数与立方图关系对不上的场在这里就被拒，
    // 而不是等到逐格取方向时把行号夹成最后一张面。
    let shape = crate::params::Shape {
        width: field.width,
        height: field.height,
        projection: field.projection,
    };
    shape.check()?;
    Ok(field)
}

pub fn stored_projection(bundle: &PayloadBundle) -> Option<Domain> {
    bundle
        .params
        .get(PROJECTION_KEY)
        .copied()
        .and_then(|value| Domain::from_code(value as u8))
}

impl px_graph_schema::Build for Field {
    fn detail(payload: &Self) -> String {
        let stats = payload.stats();
        format!(
            "{}×{}｜值域 {:.4}..{:.4}｜均值 {:.4}",
            payload.width, payload.height, stats.min, stats.max, stats.mean
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(crate::payload::encode(payload))
    }

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
        let _ = node;
        crate::payload::decode(bundle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::wire::Blob;

    fn bundle(width: u32, height: u32, projection: Domain) -> PayloadBundle {
        PayloadBundle::new(
            BTreeMap::from([
                ("width".to_string(), width as f64),
                ("height".to_string(), height as f64),
                (PROJECTION_KEY.to_string(), f64::from(projection.code())),
            ]),
            vec![Blob::from_f32(
                vec![height, width],
                &vec![0.0_f32; (width * height) as usize],
            )],
        )
    }

    #[test]
    fn a_truncated_cube_map_is_refused_at_the_payload_boundary() {
        let whole = bundle(4, 24, Domain::CubeMap);
        assert!(decode(&whole).is_ok(), "4 × 24 是完整的立方图");
        let truncated = bundle(4, 8, Domain::CubeMap);
        let message = decode(&truncated).expect_err("缺面的立方图必须被拒");
        assert!(
            message.contains("width = 4") && message.contains("height = 8"),
            "错误信息要同时点出两个数：{message}"
        );
    }
}
