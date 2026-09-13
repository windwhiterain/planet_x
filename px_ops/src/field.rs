use px_protocol::wire::{Blob, DType, WireError};

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub min: f32,
    pub max: f32,
    pub mean: f32,
}

impl Field {
    pub fn new(width: u32, height: u32, data: Vec<f32>) -> Self {
        assert_eq!(
            data.len(),
            width as usize * height as usize,
            "场的像素数与尺寸不符"
        );
        Self {
            width,
            height,
            data,
        }
    }

    pub fn filled(width: u32, height: u32, value: f32) -> Self {
        Self {
            width,
            height,
            data: vec![value; width as usize * height as usize],
        }
    }

    pub fn at(&self, x: u32, y: u32) -> f32 {
        self.data[y as usize * self.width as usize + x as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, value: f32) {
        self.data[y as usize * self.width as usize + x as usize] = value;
    }

    pub fn uv(&self, x: u32, y: u32) -> (f32, f32) {
        (
            x as f32 / self.width.max(1) as f32,
            y as f32 / self.height.max(1) as f32,
        )
    }

    pub fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let x = x.rem_euclid(width as f32);
        let y = y.clamp(0.0, height as f32 - 1.0);

        let x0 = x.floor() as u32 % width;
        let x1 = (x0 + 1) % width;
        let y0 = (y.floor() as u32).min(height - 1);
        let y1 = (y0 + 1).min(height - 1);
        let tx = x - x.floor();
        let ty = y - y.floor();

        let top = self.at(x0, y0) * (1.0 - tx) + self.at(x1, y0) * tx;
        let bottom = self.at(x0, y1) * (1.0 - tx) + self.at(x1, y1) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    pub fn stats(&self) -> Stats {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut sum = 0.0_f64;
        for value in &self.data {
            min = min.min(*value);
            max = max.max(*value);
            sum += *value as f64;
        }
        Stats {
            min,
            max,
            mean: if self.data.is_empty() {
                0.0
            } else {
                (sum / self.data.len() as f64) as f32
            },
        }
    }

    pub fn to_blob(&self) -> Blob {
        Blob::from_f32(vec![self.height, self.width], &self.data)
    }

    pub fn from_blob(blob: &Blob) -> Result<Self, WireError> {
        if blob.header.shape.len() != 2 {
            return Err(WireError::NotF32(blob.header.dtype));
        }
        let height = blob.header.shape[0];
        let width = blob.header.shape[1];
        let data = blob.f32s()?;
        Ok(Self::new(width, height, data))
    }

    pub fn is_f32_blob(blob: &Blob) -> bool {
        blob.header.dtype == DType::F32 && blob.header.shape.len() == 2
    }
}
