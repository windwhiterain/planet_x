use px_field_schema::field::Field;
use px_graph_schema::Cooked;

#[derive(px_derive::PxInputs)]
pub struct MixInput {
    pub a: Cooked<Field>,
    pub b: Cooked<Field>,
    pub mask: Cooked<Field>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct MixParams {
    pub bias: f32,
}

impl Default for MixParams {
    fn default() -> Self {
        Self { bias: 0.0 }
    }
}
