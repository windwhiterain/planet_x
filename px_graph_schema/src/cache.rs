use crate::Key;
use crate::payload::PayloadBundle;

pub trait Cache {
    fn params_text(&self, name: &str) -> Option<String>;
    fn record_params(&self, node: &str, op: &str, params_json: &str, from_file: bool);
    fn fetch(&self, key: Key) -> Option<PayloadBundle>;
    fn store(&self, report: Report<'_>, payload: &PayloadBundle) -> Result<(), String>;
}

pub struct Report<'a> {
    pub node: &'a str,
    pub op: &'static str,
    pub interface: u64,
    pub key: Key,
    pub hit: bool,
    pub millis: u64,
    pub detail: String,
}
