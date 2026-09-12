pub struct Departments {
    pub departments: Vec<Department>,
}

pub struct Department {
    pub currency: f32,
    pub policies: Vec<Policy>,
    pub policy_choice: usize,
}

pub struct Policy {
    pub consumptions: Vec<f32>,
    pub motive: f32,
    price_potential: f32,
    distribution: f32,
}
