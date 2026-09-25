use serde::{Deserialize, Serialize};

use crate::field::{Field, Projection};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Shape {
    pub width: u32,
    pub height: u32,
    pub projection: Projection,
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            width: 512,
            height: 256,
            projection: Projection::CubeMap,
        }
    }
}

impl px_graph_schema::HashField for Shape {
    fn hash_field(&self, hasher: &mut px_graph_schema::blake3::Hasher) {
        for (label, value) in [
            ("width", u64::from(self.width)),
            ("height", u64::from(self.height)),
            ("projection", u64::from(self.projection.code())),
        ] {
            hasher.update(label.as_bytes());
            hasher.update(&value.to_le_bytes());
        }
    }
}

impl Shape {
    pub fn filled(&self, value: f32) -> Field {
        Field::filled_with(self.width, self.height, value, self.projection)
    }

    pub fn direction(&self, x: u32, y: u32) -> [f32; 3] {
        crate::field::direction_at(self.width, self.height, self.projection, x, y)
    }

    pub fn volume_shape(&self) -> Option<crate::volume::VolumeShape> {
        crate::volume::VolumeShape::of(self.width, self.height, self.projection)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct RemapParams {
    pub gain: f32,
    pub bias: f32,
    pub bands: f32,
}

impl Default for RemapParams {
    fn default() -> Self {
        Self {
            gain: 0.65,
            bias: 0.0,
            bands: 8.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Fbm3Params {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub seed: u32,
    pub zonal: f32,
    pub shape: Shape,
}

impl Default for Fbm3Params {
    fn default() -> Self {
        Self {
            shape: Shape::default(),
            frequency: 3.0,
            octaves: 6,
            lacunarity: 2.0,
            gain: 0.5,
            seed: 7,
            zonal: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Ridged3Params {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub seed: u32,
    pub sharpness: f32,
    pub zonal: f32,
    pub shape: Shape,
}

impl Default for Ridged3Params {
    fn default() -> Self {
        Self {
            shape: Shape::default(),
            frequency: 6.0,
            octaves: 5,
            lacunarity: 2.1,
            gain: 0.55,
            seed: 21,
            sharpness: 2.0,
            zonal: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Warp3Params {
    pub strength: f32,
    pub axial: f32,
}

impl Default for Warp3Params {
    fn default() -> Self {
        Self {
            strength: 0.35,
            axial: 1.0,
        }
    }
}

pub mod fbm {
    use crate::params::Shape;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub frequency: f32,
        pub octaves: u32,
        pub lacunarity: f32,
        pub gain: f32,
        pub seed: u32,
        pub aspect: f32,
        pub spherical: bool,
        pub zonal: f32,
        pub shape: Shape,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                shape: Shape::default(),
                frequency: 4.0,
                octaves: 6,
                lacunarity: 2.0,
                gain: 0.5,
                seed: 7,
                aspect: 2.0,
                spherical: true,
                zonal: 1.0,
            }
        }
    }
}

pub mod gradient {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub component: u32,
        pub epsilon: f32,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                component: 0,
                epsilon: 0.0,
            }
        }
    }
}

pub fn bend(value: f32, gamma: f32) -> f32 {
    if !(gamma > 0.0) || (gamma - 1.0).abs() < f32::EPSILON {
        return value;
    }
    if value <= 0.0 {
        return 0.0;
    }
    value.powf(gamma)
}

pub mod ridged {
    use crate::params::Shape;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub frequency: f32,
        pub octaves: u32,
        pub lacunarity: f32,
        pub gain: f32,
        pub seed: u32,
        pub aspect: f32,
        pub sharpness: f32,
        pub spherical: bool,
        pub shape: Shape,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                shape: Shape::default(),
                frequency: 9.0,
                octaves: 5,
                lacunarity: 2.0,
                gain: 0.5,
                seed: 21,
                aspect: 2.0,
                sharpness: 1.6,
                spherical: true,
            }
        }
    }
}

pub mod warp {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub strength: f32,
        pub lateral: f32,
        pub probe: f32,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                strength: 0.40,
                lateral: 0.50,
                probe: 0.07,
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct CratersParams {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub jitter: f32,
    pub seed: u32,
    pub aspect: f32,
    pub spherical: bool,
    pub radius: f32,
    pub rim: f32,
    pub depth: f32,
    pub height: f32,
}

impl Default for CratersParams {
    fn default() -> Self {
        Self {
            frequency: 6.0,
            octaves: 3,
            lacunarity: 2.15,
            gain: 0.55,
            jitter: 0.85,
            seed: 31,
            aspect: 2.0,
            spherical: true,
            radius: 0.62,
            rim: 0.18,
            depth: 0.35,
            height: 0.16,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct StampsParams {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub jitter: f32,
    pub seed: u32,
    pub aspect: f32,
    pub spherical: bool,
    pub max_radius: f32,
    pub min_radius: f32,
    pub power: f32,
    pub depth: f32,
    pub height: f32,
    pub rim: f32,
    pub excavate: f32,
    pub degrade: f32,
    pub mask_lo: f32,
    pub mask_hi: f32,
}

impl Default for StampsParams {
    fn default() -> Self {
        Self {
            frequency: 6.0,
            octaves: 3,
            lacunarity: 2.4,
            gain: 0.7,
            jitter: 0.9,
            seed: 31,
            aspect: 2.0,
            spherical: true,
            max_radius: 0.55,
            min_radius: 0.12,
            power: 2.2,
            depth: 0.22,
            height: 0.22,
            rim: 0.35,
            excavate: 0.85,
            degrade: 0.45,
            mask_lo: 0.0,
            mask_hi: 1.0,
        }
    }
}
