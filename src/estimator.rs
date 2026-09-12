#[cfg(test)]
mod tests;

pub trait Estimator {
    fn update(&mut self, x: f32, y: f32);
    fn get(&self, x: f32) -> f32;
}

pub struct Scale {
    a: f32,
}

impl Scale {
    pub fn new(a: f32) -> Self {
        Self { a }
    }
}

impl Estimator for Scale {
    fn update(&mut self, x: f32, y: f32) {
        self.a = y / x;
    }

    fn get(&self, x: f32) -> f32 {
        x * self.a
    }
}

pub struct Linear {
    a: f32,
    b: f32,
}

impl Estimator for Linear {
    fn update(&mut self, _x: f32, _y: f32) {
        todo!()
    }

    fn get(&self, _x: f32) -> f32 {
        todo!()
    }
}
