use std::collections::BTreeMap;
use std::marker::PhantomData;

use px_protocol::material::ParamKind;
use px_protocol::scene::Value;

pub trait Stage {
    const KIND: &'static str;
    const LABEL: &'static str;
    const STAGE_NAME: &'static str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Given {
    pub name: &'static str,
    pub kind: ParamKind,
}

impl Given {
    pub const fn num(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::F32,
        }
    }

    pub const fn uint(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::U32,
        }
    }

    pub const fn sint(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::I32,
        }
    }

    pub const fn vec3(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::Vec3,
        }
    }

    pub const fn vec4(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::Vec4,
        }
    }
}

pub trait Content: Sized {
    type Stages;

    fn name() -> &'static str;
}

pub trait StageParams<S: Stage, M: Content> {
    const GIVEN: &'static [Given];
}

pub trait CheckStages {
    fn check_stages(&self) -> Result<(), String>;
}

macro_rules! stages {
    ($($name:ident => ($stage:literal, $kind:literal, $label:literal)),* $(,)?) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $name;
            impl Stage for $name {
                const KIND: &'static str = $kind;
                const LABEL: &'static str = $label;
                const STAGE_NAME: &'static str = $stage;
            }
        )*
    };
}

stages! {
    Prepass => ("prepass", "geometry", "预通道（只写深度）"),
    PointShadow => ("point_shadow", "geometry", "点光 cube 影子（六面各一条）"),
    Opaque => ("opaque", "geometry", "主 pass：不透明"),
    Transparent => ("transparent", "geometry", "主 pass：透明"),
    Sky => ("sky", "geometry", "天空盒"),
    Fullscreen => ("fullscreen", "fullscreen", "全屏后处理"),
}

macro_rules! contents {
    ($(
        $content:ident, $name:literal, [$($stage:ident),+ $(,)?];
        $($entry_stage:ident => [$($entry:literal : $kind:ident),* $(,)?]),* $(,)?
    );* $(;)?) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $content;

            impl Content for $content {
                type Stages = ($($stage,)+);
                fn name() -> &'static str { $name }
            }

            impl<S: Stage> CheckStages for Registration<S, $content> {
                fn check_stages(&self) -> Result<(), String> {
                    $( self.check::<$stage>()?; )+
                    Ok(())
                }
            }

            $( contents!(@impl $content, $entry_stage, [$($entry : $kind),*]); )*
        )*
    };

    (@impl $content:ident, $stage:ident, [$($entry:literal : $kind:ident),*]) => {
        impl StageParams<$stage, $content> for $stage {
            const GIVEN: &'static [Given] = &[
                $( contents!(@given $entry, $kind) ),*
            ];
        }
    };

    (@given $name:literal, num) => { Given::num($name) };
    (@given $name:literal, uint) => { Given::uint($name) };
    (@given $name:literal, sint) => { Given::sint($name) };
    (@given $name:literal, vec3) => { Given::vec3($name) };
    (@given $name:literal, vec4) => { Given::vec4($name) };
}

contents! {
    Surface, "surface", [Opaque];
    Opaque => [
        "orientation" : vec4, "emissive" : vec4,
        "inner" : num, "outer" : num,
        "coverage" : num, "shadow" : num,
        "height" : num, "gain" : num,
    ];

    Clouds, "clouds", [Transparent];
    Transparent => [
        "orientation" : vec4, "tint" : vec4,
        "inner" : num, "outer" : num,
        "density" : num, "coverage" : num,
        "base" : num, "top" : num,
        "detail_scale" : num, "detail_strength" : num,
        "erode" : num, "phase" : num,
        "shadow" : num, "steps" : uint,
        "bump" : num, "seed" : uint,
        "ablate" : uint, "slope_scale" : num,
        "taper" : num, "coverage_gain" : num,
        "surface_level" : num, "bound" : uint,
        "gradient" : uint, "wind" : num,
        "wind_skin" : num,
    ];

    Atmosphere, "atmosphere", [Transparent];
    Transparent => [
        "inner" : num, "outer" : num,
        "density" : num, "softness" : num,
        "tint" : vec4,
    ];

    Ring, "ring", [Transparent];
    Transparent => ["tint" : vec4];

    Skybox, "skybox", [Sky];
    Sky => ["brightness" : num];
}

pub fn kind_of(value: &Value) -> Option<ParamKind> {
    match value {
        Value::Num(_) => Some(ParamKind::F32),
        Value::Quad(_) => Some(ParamKind::Vec4),
        Value::Triple(_) => Some(ParamKind::Vec3),
        Value::Text(_) => None,
    }
}

pub fn satisfies(value: &Value, want: ParamKind) -> Result<(), String> {
    match (value, want) {
        (Value::Quad(_), ParamKind::Vec4) | (Value::Triple(_), ParamKind::Vec3) => Ok(()),
        (Value::Num(_), ParamKind::F32) => Ok(()),
        (Value::Num(number), ParamKind::U32 | ParamKind::I32) => {
            if number.fract() != 0.0 {
                return Err(format!(
                    "给的是 {number}（有小数部分），这一档要的是{}",
                    if want == ParamKind::U32 { "u32" } else { "i32" }
                ));
            }
            let range = match want {
                ParamKind::U32 => (0.0, u32::MAX as f64),
                _ => (i32::MIN as f64, i32::MAX as f64),
            };
            if !(range.0..=range.1).contains(number) {
                return Err(format!("给的是 {number}，超出这一档的取值范围"));
            }
            Ok(())
        }
        (value, want) => Err(format!(
            "这一档要 {}，给的是 {}",
            want.name(),
            kind_of(value).map(ParamKind::name).unwrap_or("文本"),
        )),
    }
}

pub fn check<S: Stage, M: Content>(params: &BTreeMap<String, Value>) -> Result<(), String>
where
    S: StageParams<S, M>,
{
    check_given::<S, M>(<S as StageParams<S, M>>::GIVEN, params)
}

fn check_given<S: Stage, M: Content>(
    given: &'static [Given],
    params: &BTreeMap<String, Value>,
) -> Result<(), String> {
    let mut missing: Vec<String> = Vec::new();
    let mut wrong: Vec<String> = Vec::new();
    for field in given {
        match params.get(field.name) {
            None => missing.push(format!("{}（{}）", field.name, field.kind.name())),
            Some(value) => {
                if let Err(why) = satisfies(value, field.kind) {
                    wrong.push(format!("{}：{why}", field.name));
                }
            }
        }
    }
    if missing.is_empty() && wrong.is_empty() {
        return Ok(());
    }
    let mut why = String::new();
    if !missing.is_empty() {
        why.push_str(&format!("\n  缺参数：{}", missing.join(" / ")));
    }
    if !wrong.is_empty() {
        why.push_str(&format!("\n  类型不符：{}", wrong.join("；")));
    }
    Err(format!(
        "stage '{}'（{}）要 material '{}' 给的 per-pass 参数没给全：{why}\n  \
         这一档要的格：{}\n  \
         材质实际给的：{}",
        S::STAGE_NAME,
        S::LABEL,
        M::name(),
        given
            .iter()
            .map(|field| format!("{}（{}）", field.name, field.kind.name()))
            .collect::<Vec<_>>()
            .join(" / "),
        if params.is_empty() {
            "（空）".to_string()
        } else {
            params.keys().cloned().collect::<Vec<_>>().join(" / ")
        }
    ))
}

pub mod runtime {
    use super::*;
    use px_protocol::scene::AlphaMode;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RuntimeStage {
        Prepass,
        PointShadow,
        Opaque,
        Transparent,
        Sky,
        Fullscreen,
    }

    impl RuntimeStage {
        pub fn name(self) -> &'static str {
            match self {
                Self::Prepass => Prepass::STAGE_NAME,
                Self::PointShadow => PointShadow::STAGE_NAME,
                Self::Opaque => Opaque::STAGE_NAME,
                Self::Transparent => Transparent::STAGE_NAME,
                Self::Sky => Sky::STAGE_NAME,
                Self::Fullscreen => Fullscreen::STAGE_NAME,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RuntimeContent {
        Surface,
        Clouds,
        Atmosphere,
        Ring,
        Skybox,
    }

    impl RuntimeContent {
        pub fn parse(name: &str) -> Option<Self> {
            match name {
                "surface" => Some(Self::Surface),
                "clouds" => Some(Self::Clouds),
                "atmosphere" => Some(Self::Atmosphere),
                "ring" => Some(Self::Ring),
                "skybox" => Some(Self::Skybox),
                _ => None,
            }
        }

        pub fn name(self) -> &'static str {
            match self {
                Self::Surface => Surface::name(),
                Self::Clouds => Clouds::name(),
                Self::Atmosphere => Atmosphere::name(),
                Self::Ring => Ring::name(),
                Self::Skybox => Skybox::name(),
            }
        }
    }

    pub fn stage_of_alpha(alpha: AlphaMode) -> RuntimeStage {
        if alpha == AlphaMode::Opaque {
            RuntimeStage::Opaque
        } else {
            RuntimeStage::Transparent
        }
    }

    pub fn check(
        stage: RuntimeStage,
        content: RuntimeContent,
        params: &BTreeMap<String, Value>,
    ) -> Result<(), String> {
        use RuntimeContent as C;
        use RuntimeStage as S;
        let checked = match (stage, content) {
            (S::Opaque, C::Surface) => check_given::<Opaque, Surface>(
                <Opaque as StageParams<Opaque, Surface>>::GIVEN,
                params,
            ),
            (S::Transparent, C::Clouds) => check_given::<Transparent, Clouds>(
                <Transparent as StageParams<Transparent, Clouds>>::GIVEN,
                params,
            ),
            (S::Transparent, C::Atmosphere) => check_given::<Transparent, Atmosphere>(
                <Transparent as StageParams<Transparent, Atmosphere>>::GIVEN,
                params,
            ),
            (S::Transparent, C::Ring) => check_given::<Transparent, Ring>(
                <Transparent as StageParams<Transparent, Ring>>::GIVEN,
                params,
            ),
            (S::Sky, C::Skybox) => {
                check_given::<Sky, Skybox>(<Sky as StageParams<Sky, Skybox>>::GIVEN, params)
            }
            _ => Ok(()),
        };
        checked.map_err(|err| format!("内容 '{}'：{err}", content.name()))
    }
}

#[derive(Debug, Clone)]
struct Slot {
    stage: &'static str,
    params: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct Registration<S, M> {
    slots: Vec<Slot>,
    default: Option<Slot>,
    stage: PhantomData<S>,
    content: PhantomData<M>,
}

impl<S: Stage, M: Content> Default for Registration<S, M> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            default: None,
            stage: PhantomData,
            content: PhantomData,
        }
    }
}

impl<S: Stage, M: Content> Registration<S, M> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn single(params: BTreeMap<String, Value>) -> Self {
        Self {
            slots: Vec::new(),
            default: Some(Slot {
                stage: S::STAGE_NAME,
                params,
            }),
            stage: PhantomData,
            content: PhantomData,
        }
    }

    pub fn stage<T: Stage>(mut self, params: BTreeMap<String, Value>) -> Self {
        let slot = Slot {
            stage: T::STAGE_NAME,
            params,
        };
        match self
            .slots
            .iter_mut()
            .find(|known| known.stage == slot.stage)
        {
            Some(existing) => *existing = slot,
            None => self.slots.push(slot),
        }
        self
    }

    pub fn params_of(&self, stage: &str) -> Option<&BTreeMap<String, Value>> {
        if let Some(slot) = self.slots.iter().find(|slot| slot.stage == stage) {
            return Some(&slot.params);
        }
        self.default.as_ref().map(|slot| &slot.params)
    }

    pub fn check<T: Stage>(&self) -> Result<(), String>
    where
        T: StageParams<T, M>,
    {
        let Some(params) = self.params_of(T::STAGE_NAME) else {
            return Ok(());
        };
        check::<T, M>(params)
    }

    pub fn check_all(&self) -> Result<(), String>
    where
        Self: CheckStages,
    {
        self.check_stages()
    }

    pub fn freeze(&self, document_stage: &str) -> BTreeMap<String, Value> {
        self.params_of(document_stage)
            .or_else(|| self.default.as_ref().map(|slot| &slot.params))
            .cloned()
            .unwrap_or_default()
    }

    pub fn stages(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self.slots.iter().map(|slot| slot.stage).collect();
        if out.is_empty() {
            if let Some(slot) = &self.default {
                out.push(slot.stage);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect()
    }

    fn atmosphere_params() -> BTreeMap<String, Value> {
        params(&[
            ("inner", Value::Num(1.0)),
            ("outer", Value::Num(1.14)),
            ("density", Value::Num(0.3)),
            ("softness", Value::Num(0.5)),
            ("tint", Value::Quad([0.0, 0.0, 0.0, 1.0])),
        ])
    }

    #[test]
    fn a_complete_material_passes_its_stage() {
        Registration::<Transparent, Atmosphere>::single(atmosphere_params())
            .check_all()
            .expect("五格都给全了");
    }

    #[test]
    fn a_missing_per_pass_field_names_what_is_missing() {
        let mut thin = atmosphere_params();
        thin.remove("softness");
        let err = Registration::<Transparent, Atmosphere>::single(thin)
            .check_all()
            .expect_err("少了 softness");
        assert!(err.contains("softness"), "{err}");
        assert!(err.contains("缺参数"), "{err}");
        assert!(err.contains("atmosphere"), "要点名是哪份内容：{err}");
    }

    #[test]
    fn a_wrong_kind_names_both_sides() {
        let mut wrong = atmosphere_params();
        wrong.insert("tint".to_string(), Value::Triple([1.0, 1.0, 1.0]));
        let err = Registration::<Transparent, Atmosphere>::single(wrong)
            .check_all()
            .expect_err("vec3 ≠ vec4");
        assert!(err.contains("tint"), "{err}");
        assert!(err.contains("类型不符"), "{err}");
    }

    #[test]
    fn extra_names_are_not_a_failure() {
        let mut extra = atmosphere_params();
        extra.insert("这是别的档要的格".to_string(), Value::Num(1.0));
        Registration::<Transparent, Atmosphere>::single(extra)
            .check_all()
            .expect("子集判据：多给的不是错");
    }

    #[test]
    fn one_stage_several_contents_keep_their_own_tables() {
        let clouds = <Transparent as StageParams<Transparent, Clouds>>::GIVEN;
        let atmosphere = <Transparent as StageParams<Transparent, Atmosphere>>::GIVEN;
        assert!(
            clouds.iter().any(|field| field.name == "steps"),
            "云要 steps"
        );
        assert!(
            !atmosphere.iter().any(|field| field.name == "steps"),
            "大气那一档不该要云的那些格"
        );
    }

    #[test]
    fn a_stage_specific_registration_overrides_the_single_one() {
        let mut base = atmosphere_params();
        base.insert("density".to_string(), Value::Num(0.1));
        let mut shadow = atmosphere_params();
        shadow.insert("density".to_string(), Value::Num(0.9));

        let registration =
            Registration::<Transparent, Atmosphere>::single(base).stage::<PointShadow>(shadow);
        assert_eq!(
            registration.params_of(PointShadow::STAGE_NAME).unwrap()["density"],
            Value::Num(0.9)
        );
        assert_eq!(
            registration.params_of(Transparent::STAGE_NAME).unwrap()["density"],
            Value::Num(0.1),
            "没点名的档仍走 single 那一份"
        );
    }

    #[test]
    fn a_content_participates_in_the_stages_its_type_list_says() {
        Registration::<Sky, Skybox>::single(params(&[("brightness", Value::Num(900.0))]))
            .check_all()
            .expect("skybox 只要 brightness");
        let err =
            Registration::<Sky, Skybox>::single(params(&[("brightness", Value::Triple([0.0; 3]))]))
                .check_all()
                .expect_err("vec3 不是 f32");
        assert!(err.contains("brightness"), "{err}");
    }

    #[test]
    fn the_ring_content_needs_only_its_tint() {
        Registration::<Transparent, Ring>::single(params(&[(
            "tint",
            Value::Quad([1.0, 1.0, 1.0, 1.0]),
        )]))
        .check_all()
        .expect("一个格");
    }

    #[test]
    fn an_integral_number_satisfies_an_integer_slot() {
        let mut clouds = params(&[
            ("orientation", Value::Quad([0.0, 0.0, 0.0, 1.0])),
            ("tint", Value::Quad([1.0, 1.0, 1.0, 1.0])),
        ]);
        for name in [
            "inner",
            "outer",
            "density",
            "coverage",
            "base",
            "top",
            "detail_scale",
            "detail_strength",
            "erode",
            "phase",
            "shadow",
            "bump",
            "slope_scale",
            "taper",
            "coverage_gain",
            "surface_level",
            "wind",
            "wind_skin",
        ] {
            clouds.insert(name.to_string(), Value::Num(1.0));
        }
        for name in ["steps", "seed", "ablate", "bound", "gradient"] {
            clouds.insert(name.to_string(), Value::Num(64.0));
        }
        Registration::<Transparent, Clouds>::single(clouds)
            .check_all()
            .expect("整数格按数给也算满足");
    }

    #[test]
    fn a_fractional_number_is_refused_for_an_integer_slot() {
        let mut clouds = params(&[
            ("orientation", Value::Quad([0.0, 0.0, 0.0, 1.0])),
            ("tint", Value::Quad([1.0, 1.0, 1.0, 1.0])),
        ]);
        for name in [
            "inner",
            "outer",
            "density",
            "coverage",
            "base",
            "top",
            "detail_scale",
            "detail_strength",
            "erode",
            "phase",
            "shadow",
            "bump",
            "slope_scale",
            "taper",
            "coverage_gain",
            "surface_level",
            "wind",
            "wind_skin",
        ] {
            clouds.insert(name.to_string(), Value::Num(1.0));
        }
        for name in ["steps", "seed", "ablate", "bound", "gradient"] {
            clouds.insert(name.to_string(), Value::Num(64.0));
        }
        clouds.insert("steps".to_string(), Value::Num(1.5));
        let err = Registration::<Transparent, Clouds>::single(clouds)
            .check_all()
            .expect_err("1.5 塞不进 u32");
        assert!(err.contains("steps"), "{err}");
        assert!(err.contains("小数"), "{err}");
    }

    #[test]
    fn satisfies_judges_packability_not_the_rust_type() {
        assert!(satisfies(&Value::Quad([0.0; 4]), ParamKind::Vec4).is_ok());
        assert!(satisfies(&Value::Triple([0.0; 3]), ParamKind::Vec3).is_ok());
        assert!(satisfies(&Value::Num(2.0), ParamKind::U32).is_ok());
        assert!(
            satisfies(&Value::Num(-1.0), ParamKind::U32).is_err(),
            "u32 不收负数"
        );
        assert!(satisfies(&Value::Triple([0.0; 3]), ParamKind::Vec4).is_err());
        assert!(satisfies(&Value::Quad([0.0; 4]), ParamKind::Vec3).is_err());
        assert!(satisfies(&Value::Text("x".to_string()), ParamKind::F32).is_err());
        assert!(satisfies(&Value::Num(1.0), ParamKind::Vec4).is_err());
    }

    #[test]
    fn freeze_resolves_by_the_document_stage() {
        let mut base = atmosphere_params();
        base.insert("density".to_string(), Value::Num(0.1));
        let mut shadow = atmosphere_params();
        shadow.insert("density".to_string(), Value::Num(0.9));
        let registration =
            Registration::<Transparent, Atmosphere>::single(base).stage::<PointShadow>(shadow);
        assert_eq!(
            registration.freeze(PointShadow::STAGE_NAME)["density"],
            Value::Num(0.9)
        );
        assert_eq!(
            registration.freeze(Transparent::STAGE_NAME)["density"],
            Value::Num(0.1)
        );
    }
}
