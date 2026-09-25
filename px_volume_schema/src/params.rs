use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    #[default]
    Coarse,
    Final,
}

#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub field: FieldKind,
    pub res: u32,
    pub layers: u32,
    pub inner: f32,
    pub outer: f32,
    pub tau: f32,
    pub scale: f32,
    pub reach: u32,
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    pub erode: f32,
    pub orientation: [f32; 4],
}

impl Default for Params {
    fn default() -> Self {
        Self {
            field: FieldKind::Coarse,
            res: 65,
            layers: 65,
            inner: 1.01,
            outer: 1.06,
            tau: 0.20,
            scale: 240.0,
            reach: 1,
            coverage: 0.35,
            base: 0.06,
            top: 0.62,
            taper: 0.45,
            coverage_gain: 2.6,
            erode: 0.0,
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl Params {
    pub fn span(&self) -> f32 {
        self.outer - self.inner
    }
}

impl px_graph_schema::HashField for FieldKind {
    fn hash_field(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&[*self as u8]);
    }
}

pub mod emission {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct EmissionParams {
        pub starlight_gain: f32,
        pub starlight_soft: f32,
        pub starlight_radius: f32,
        pub starlight_steps: u32,
        pub starlight_max: u32,
        pub light: [f32; 3],
        pub light_radius: f32,
        pub shadow_steps: u32,
        pub shadow_gain: f32,
        pub emission_power: f32,
        pub emission_gain: f32,
        pub extinction: [f32; 3],
        pub extinction_power: f32,
        pub dust_bias: f32,
        pub dust_threshold: f32,
        pub glow_gain: f32,
        pub glow_power: f32,
        pub glow_tint: [f32; 3],
        pub scatter_tint: [f32; 3],
        pub glow_threshold: f32,
    }

    impl Default for EmissionParams {
        fn default() -> Self {
            Self {
                light: [0.3, 0.5, 0.8],
                light_radius: 0.25,
                starlight_gain: 0.0,
                starlight_soft: 0.05,
                starlight_radius: 0.20,
                starlight_steps: 12,
                starlight_max: 8,
                shadow_steps: 24,
                shadow_gain: 1.6,
                emission_power: 2.2,
                emission_gain: 1.0,
                glow_gain: 0.0,
                glow_power: 1.0,
                glow_tint: [1.0, 1.0, 1.0],
                scatter_tint: [1.0, 1.0, 1.0],
                glow_threshold: 0.0,
                extinction: [1.6, 2.4, 3.4],
                extinction_power: 1.0,
                dust_bias: 2.0,
                dust_threshold: 0.35,
            }
        }
    }
}

pub mod sky {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct SkyParams {
        pub face: u32,
        pub steps: u32,
        pub jitter: f32,
        pub star_gain: f32,
        pub star_tint: [f32; 3],
        pub star_core: f32,
        pub star_halo: f32,
        pub star_halo_gain: f32,
        pub background: [f32; 3],
    }

    impl Default for SkyParams {
        fn default() -> Self {
            Self {
                face: 256,
                steps: 96,
                jitter: 1.0,
                star_gain: 1.0,
                star_tint: [1.0, 1.0, 1.0],
                star_core: 0.0029,
                star_halo: 0.010,
                star_halo_gain: 0.035,
                background: [0.0, 0.0, 0.0],
            }
        }
    }
}

pub mod stars {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct StarsParams {
        pub count: u32,
        pub seed: u32,
        pub inner: f32,
        pub outer: f32,
        pub cell: f32,
        pub brightness_power: f32,
        pub max_brightness: f32,
        pub min_apparent: f32,
        pub cluster_count: u32,
        pub cluster: [f32; 3],
        pub cluster_radius: f32,
        pub cluster_gain: f32,
        pub cluster_tint: [f32; 3],
        pub clump_count: u32,
        pub clump_radius: f32,
        pub clump_share: f32,
        pub star_tint: [f32; 3],
        pub sky_biased: bool,
        pub sky_frequency: f32,
        pub sky_octaves: u32,
        pub sky_lacunarity: f32,
        pub sky_gain: f32,
        pub sky_seed: u32,
        pub sky_zonal: f32,
        pub sky_contrast: f32,
        pub gas_biased: bool,
        pub gas_contrast: f32,
        pub gas_floor: f32,
        pub gas_depth: f32,
    }

    impl Default for StarsParams {
        fn default() -> Self {
            Self {
                count: 180_000,
                seed: 60613,
                inner: 1.0,
                outer: 3.0,
                cell: 0.05,
                brightness_power: 1.1,
                max_brightness: 64.0,
                min_apparent: 0.0,
                clump_count: 0,
                clump_radius: 0.25,
                clump_share: 0.0,
                star_tint: [1.0, 1.0, 1.0],
                sky_biased: false,
                sky_frequency: 0.35,
                sky_octaves: 3,
                sky_lacunarity: 2.0,
                sky_gain: 0.5,
                sky_seed: 9173,
                sky_zonal: 0.9,
                sky_contrast: 2.0,
                gas_biased: false,
                gas_contrast: 1.5,
                gas_floor: 0.0,
                gas_depth: 0.0,
                cluster_count: 4,
                cluster: [0.50, 0.70, 1.80],
                cluster_radius: 0.35,
                cluster_gain: 1.2,
                cluster_tint: [0.72, 0.86, 1.0],
            }
        }
    }
}

pub mod density {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct DensityParams {
        pub res: u32,
        pub layers: u32,
        pub inner: f32,
        pub outer: f32,
        pub reach: u32,
    }

    impl Default for DensityParams {
        fn default() -> Self {
            Self {
                res: 64,
                layers: 64,
                inner: 1.0,
                outer: 1.6,
                reach: 1,
            }
        }
    }

    impl DensityParams {
        pub fn span(&self) -> f32 {
            self.outer - self.inner
        }

        pub fn shape_of(&self) -> (u32, u32) {
            (self.res.max(2), self.layers.max(2))
        }
    }
}
