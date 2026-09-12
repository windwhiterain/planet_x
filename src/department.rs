pub struct Departments {
    pub departments: Vec<Department>,
}

/// index with [`crate::warehouse::Warehouse`]
pub struct Department {
    pub currency: f32,
    /// index with [`crate::warehouse::Stock`]
    pub productions: Vec<f32>,
    pub policies: Vec<Policy>,
    /// index [`Self::policies`]
    policy_choice: usize,
    policy_execution: f32,
}

pub struct Policy {
    /// index with [`crate::warehouse::Stock`]
    pub consumptions: Vec<f32>,
    pub motive: f32,
    price_potential: f32,
    /// normalize to 1
    distribution: f32,
}
