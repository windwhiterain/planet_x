use std::collections::BTreeMap;
use std::sync::OnceLock;

use px_protocol::material::MaterialLayout;
#[cfg(test)]
use px_protocol::material::ParamKind;
use px_protocol::scene::Value;

pub const CLOUD_BASE: f32 = 1.01;
pub const CLOUD_TOP: f32 = 1.06;

#[derive(Clone, Copy, Debug)]
pub struct CloudParams {
    pub orientation: [f32; 4],
    pub tint: [f32; 4],
    pub inner: f32,
    pub outer: f32,
    pub density: f32,
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub detail_scale: f32,
    pub detail_strength: f32,
    pub erode: f32,
    pub phase: f32,
    pub shadow: f32,
    pub steps: u32,
    pub bump: f32,
    pub seed: u32,
    pub ablate: u32,
    pub slope_scale: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    pub surface_level: f32,
    pub bound: u32,
    pub gradient: u32,
    pub wind: f32,
    pub wind_skin: f32,
}

impl CloudParams {
    pub fn new(inner: f32, outer: f32, density: f32) -> Self {
        Self {
            orientation: [0.0, 0.0, 0.0, 1.0],
            tint: [1.0, 0.99, 0.97, 1.0],
            inner,
            outer,
            density,
            coverage: 0.35,
            base: 0.06,
            top: 0.62,
            detail_scale: 16.0,
            detail_strength: 0.55,
            erode: 0.0,
            phase: 0.62,
            shadow: 1.0,
            steps: 56,
            bump: 0.85,
            seed: 7,
            ablate: 0,
            slope_scale: 0.12,
            taper: 0.45,
            coverage_gain: 2.6,
            surface_level: 0.20,
            bound: 0,
            gradient: 1,
            wind: 0.0,
            wind_skin: 0.0,
        }
    }

    pub fn to_map(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("orientation".to_string(), Value::Quad(self.orientation)),
            ("tint".to_string(), Value::Quad(self.tint)),
            ("inner".to_string(), Value::Num(f64::from(self.inner))),
            ("outer".to_string(), Value::Num(f64::from(self.outer))),
            ("density".to_string(), Value::Num(f64::from(self.density))),
            ("coverage".to_string(), Value::Num(f64::from(self.coverage))),
            ("base".to_string(), Value::Num(f64::from(self.base))),
            ("top".to_string(), Value::Num(f64::from(self.top))),
            (
                "detail_scale".to_string(),
                Value::Num(f64::from(self.detail_scale)),
            ),
            (
                "detail_strength".to_string(),
                Value::Num(f64::from(self.detail_strength)),
            ),
            ("erode".to_string(), Value::Num(f64::from(self.erode))),
            ("phase".to_string(), Value::Num(f64::from(self.phase))),
            ("shadow".to_string(), Value::Num(f64::from(self.shadow))),
            ("steps".to_string(), Value::Num(f64::from(self.steps))),
            ("bump".to_string(), Value::Num(f64::from(self.bump))),
            ("seed".to_string(), Value::Num(f64::from(self.seed))),
            ("ablate".to_string(), Value::Num(f64::from(self.ablate))),
            (
                "slope_scale".to_string(),
                Value::Num(f64::from(self.slope_scale)),
            ),
            ("taper".to_string(), Value::Num(f64::from(self.taper))),
            (
                "coverage_gain".to_string(),
                Value::Num(f64::from(self.coverage_gain)),
            ),
            (
                "surface_level".to_string(),
                Value::Num(f64::from(self.surface_level)),
            ),
            ("bound".to_string(), Value::Num(f64::from(self.bound))),
            ("gradient".to_string(), Value::Num(f64::from(self.gradient))),
            ("wind".to_string(), Value::Num(f64::from(self.wind))),
            (
                "wind_skin".to_string(),
                Value::Num(f64::from(self.wind_skin)),
            ),
        ])
    }
}

pub fn layout() -> &'static MaterialLayout {
    static LAYOUT: OnceLock<MaterialLayout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        let assembled = crate::common::assemble("clouds.wgsl");
        px_shader::reflect::reflect_assembled(&assembled, "clouds.wgsl")
            .unwrap_or_else(|err| panic!("云的材质契约反射不出来：{err}"))
    })
}

pub fn params_bytes(params: &CloudParams) -> Vec<u8> {
    layout().pack(&params.to_map()).unwrap_or_else(|err| {
        panic!(
            "探针的 CloudParams 与 clouds.wgsl 的契约对不上：{err}\n  \
                 ⇒ 两边漂了：先 cargo test -p px_probe 看是哪一格"
        )
    })
}

#[cfg(test)]
const MIRROR: [(&str, ParamKind); 25] = [
    ("orientation", ParamKind::Vec4),
    ("tint", ParamKind::Vec4),
    ("inner", ParamKind::F32),
    ("outer", ParamKind::F32),
    ("density", ParamKind::F32),
    ("coverage", ParamKind::F32),
    ("base", ParamKind::F32),
    ("top", ParamKind::F32),
    ("detail_scale", ParamKind::F32),
    ("detail_strength", ParamKind::F32),
    ("erode", ParamKind::F32),
    ("phase", ParamKind::F32),
    ("shadow", ParamKind::F32),
    ("steps", ParamKind::U32),
    ("bump", ParamKind::F32),
    ("seed", ParamKind::U32),
    ("ablate", ParamKind::U32),
    ("slope_scale", ParamKind::F32),
    ("taper", ParamKind::F32),
    ("coverage_gain", ParamKind::F32),
    ("surface_level", ParamKind::F32),
    ("bound", ParamKind::U32),
    ("gradient", ParamKind::U32),
    ("wind", ParamKind::F32),
    ("wind_skin", ParamKind::F32),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mirror_matches_the_contract() {
        let layout = layout();
        let names: Vec<(&str, ParamKind)> = layout
            .params
            .iter()
            .map(|slot| (slot.name.as_str(), slot.kind))
            .collect();
        assert_eq!(
            names,
            MIRROR.to_vec(),
            "探针镜像与 shader 声明不一致（名字 / 顺序 / 类型）：\n  契约 {names:?}\n  镜像 {:?}",
            MIRROR
        );
        assert_eq!(layout.params_bytes % 16, 0, "参数块按 16 对齐");
        assert_eq!(
            params_bytes(&CloudParams::new(CLOUD_BASE, CLOUD_TOP, 900.0)).len(),
            layout.params_bytes as usize,
            "打包出来的字节数 = 契约说的字节数"
        );
    }

    #[test]
    fn a_drifted_mirror_is_refused_loudly() {
        let mut map = CloudParams::new(CLOUD_BASE, CLOUD_TOP, 900.0).to_map();
        map.remove("wind_skin");
        let err = layout().pack(&map).expect_err("少一格必须报错");
        assert!(err.contains("wind_skin"), "报错要点名：{err}");
        map.insert("typo".to_string(), Value::Num(1.0));
        map.insert("wind_skin".to_string(), Value::Num(0.0));
        let err = layout().pack(&map).expect_err("多一格也必须报错");
        assert!(err.contains("typo"), "报错要点名：{err}");
    }
}
