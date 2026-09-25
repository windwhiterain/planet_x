use px_field_schema::field::Field;
use px_graph_schema::Cooked;

#[derive(px_derive::PxInputs)]
pub struct FuseInput {
    pub a: Cooked<Field>,
    pub b: Cooked<Field>,
    pub mask: Cooked<Field>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct FuseParams {
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub smooth: bool,
    pub gamma: f32,
    pub bias: f32,
}

impl Default for FuseParams {
    fn default() -> Self {
        Self {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
            gamma: 1.0,
            bias: 0.0,
        }
    }
}
