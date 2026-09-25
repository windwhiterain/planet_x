pub mod curve {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct CircleParams {
        pub radius: f64,
        pub plane: String,
        pub center: [f64; 3],
    }

    impl Default for CircleParams {
        fn default() -> Self {
            Self {
                radius: 1.0,
                plane: "xy".to_string(),
                center: [0.0; 3],
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct SphereParams {
        pub radius: f64,
        pub center: [f64; 3],
        pub rings: u32,
    }

    impl Default for SphereParams {
        fn default() -> Self {
            Self {
                radius: 1.0,
                center: [0.0; 3],
                rings: 2,
            }
        }
    }
}

pub mod eval {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct EvalParams {
        pub u: f64,
        pub v: f64,
        pub tangent: bool,
    }

    impl Default for EvalParams {
        fn default() -> Self {
            Self {
                u: 0.5,
                v: 0.5,
                tangent: false,
            }
        }
    }
}

pub mod insert {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct InsertParams {
        pub t: f64,
        pub times: u32,
        pub along: String,
    }

    impl Default for InsertParams {
        fn default() -> Self {
            Self {
                t: 0.5,
                times: 1,
                along: "u".to_string(),
            }
        }
    }
}

pub mod elevate {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct ElevateParams {
        pub degree: u32,
    }

    impl Default for ElevateParams {
        fn default() -> Self {
            Self { degree: 3 }
        }
    }
}

pub mod tessellate {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct TessellateParams {
        pub tolerance: f64,
        pub depth: u32,
        pub segments: u32,
    }

    impl Default for TessellateParams {
        fn default() -> Self {
            Self {
                tolerance: 1e-3,
                depth: 6,
                segments: 4,
            }
        }
    }
}
