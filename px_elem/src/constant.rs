use px_field_schema::params::Shape;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct ConstantParams {
    pub shape: Shape,
    pub value: f32,
}

impl Default for ConstantParams {
    fn default() -> Self {
        Self {
            shape: Shape::default(),
            value: 0.5,
        }
    }
}
