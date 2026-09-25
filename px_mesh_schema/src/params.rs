pub mod cubesphere {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default)]
    pub struct Params {
        pub subdivisions: u32,
        pub radius: f32,
        pub displace: f32,
        pub sea_level: f32,
        pub flat_sea: bool,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                subdivisions: 128,
                radius: 1.0,
                displace: 0.075,
                sea_level: 0.52,
                flat_sea: false,
            }
        }
    }
}

pub mod proxy {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub level: f32,
        pub depth: u32,
        pub weld: f32,
        #[serde(skip_serializing_if = "is_zero")]
        pub offset: f32,
    }

    fn is_zero(value: &f32) -> bool {
        *value == 0.0
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                level: 0.0,
                depth: 6,
                weld: 1e-4,
                offset: 0.0,
            }
        }
    }
}
