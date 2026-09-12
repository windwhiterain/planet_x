pub fn same_signature(a: f32, b: f32) -> bool {
    a >= 0.0 && b >= 0.0 || a <= 0.0 && b <= 0.0
}

pub fn conditional_swap<T>(a: T, b: T, condition: bool) -> (T, T) {
    if condition { (b, a) } else { (a, b) }
}

pub fn cosine_similarity(a: f32, b: f32) -> f32 {
    2.0 * a * b / (a * a + b * b)
}

pub fn conditional_signature(condition: bool) -> f32 {
    if condition { 1.0 } else { -1.0 }
}

pub fn abgebraic_average(a: f32, b: f32) -> f32 {
    f32::sqrt(a * b)
}
