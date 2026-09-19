use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoodView {
    pub name: String,
    pub price: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepartmentView {
    pub name: String,
    pub execution: f32,
    pub revenue: f32,
    pub payment: f32,
    pub holdings: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    pub cpi: f32,
    pub output: f32,
    pub consumption: f32,
    pub turnover: f32,
    pub granted: f32,
    pub treasury: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldView {
    pub schema_version: u32,
    pub round: u32,
    pub goods: Vec<GoodView>,
    pub departments: Vec<DepartmentView>,
    pub totals: Totals,
}
