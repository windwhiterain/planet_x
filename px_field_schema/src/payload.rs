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
