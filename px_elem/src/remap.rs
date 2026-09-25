use px_field_schema::field::Field;
use px_graph_schema::Cooked;

#[derive(px_derive::PxInputs)]
pub struct RemapInput {
    pub field: Cooked<Field>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct RemapParams {
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub smooth: bool,
    pub gamma: f32,
}

impl Default for RemapParams {
    fn default() -> Self {
        Self {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
            gamma: 1.0,
        }
    }
}
