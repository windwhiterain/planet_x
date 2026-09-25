use px_field_schema::field::Field;
use px_field_schema::params::RemapParams;

pub trait FieldFn {
    fn value(&self, params: &RemapParams, upstream: f32, uv: [f32; 2], direction: [f32; 3]) -> f32;
}

pub trait Upstream {
    fn uv(&self, x: u32, y: u32) -> [f32; 2];
    fn direction(&self, x: u32, y: u32) -> [f32; 3];
    fn upstream(&self, x: u32, y: u32) -> f32;
}

pub struct Sampled<'a> {
    pub field: &'a Field,
}

impl Upstream for Sampled<'_> {
    fn uv(&self, x: u32, y: u32) -> [f32; 2] {
        let (u, v) = self.field.uv(x, y);
        [u, v]
    }

    fn direction(&self, x: u32, y: u32) -> [f32; 3] {
        self.field.direction(x, y)
    }

    fn upstream(&self, x: u32, y: u32) -> f32 {
        self.field.at(x, y)
    }
}

impl FieldFn for Sampled<'_> {
    fn value(
        &self,
        _params: &RemapParams,
        upstream: f32,
        _uv: [f32; 2],
        _direction: [f32; 3],
    ) -> f32 {
        upstream
    }
}
