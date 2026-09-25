use px_field_schema::field::Field;
use px_verify::proxy;

pub type CoverCloud = px_verify::cloud_field::CloudFieldParams;

pub trait FieldFn {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32;
}

pub struct SampleField<'a> {
    pub field: &'a Field,
}

impl FieldFn for SampleField<'_> {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32 {
        proxy::cover_at(cloud, self.field, direction)
    }
}
