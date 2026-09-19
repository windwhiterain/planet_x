//! **pipeline 的 stage 与它要求的 per-pass material 参数**。
//!
//! 这一篇说的是**一件事**，分两半：
//!
//! 1. **一个物体可以注册多个 stage 的材质**（[`Registration`]）—— 寻常引擎的形状：
//!    prepass 只写深度、影子那一档 depth-only、主 pass 要 color + depth，同一个物体在这几档里
//!    各有一份材质（或者共用同一份）。
//! 2. **每个 stage 只要求 material 里那些 *per pass* 的参数的类型**（[`StageParams`]）——
//!    ⚠ **不是**要求整份 material 的格式：一份材质的参数表往往是几档的并集（`surface` 的
//!    结构体里就有云影那几个格），而某一档只碰其中几个。所以判据是
//!    **"这一档要的那些格，在这份 material 的参数表里存在、且类型相等"**（子集 + 类型相等）。
//!
//! ```ignore
//! // px_graphs/src/bin/scene.rs（那条图的脚本）
//! /// `opaque` 那一档从材质里读的那几格。
//! #[derive(Debug, Clone, Copy)]
//! pub struct OpaquePass;
//! impl StageParams<Opaque> for OpaquePass {
//!     const GIVEN: &'static [Given] = &[
//!         Given::vec4("orientation"), Given::num("inner"), Given::num("outer"),
//!     ];
//! }
//! ```
//!
//! ⚠ 这里**故意不**从 WGSL 反推：这一栏说的是"**这一档**读哪几个格"，那是**意图**。
//! 反射管的是另一件事 —— 参数的**名字 ↔ 字节**（[`crate::contract`]），两者各有一半：
//!
//! · **本模块**：这一档要哪几个格、各是什么类型 ⇒ 意图，编译时逐项对账；
//! · **契约**：那一格落在第几字节 ⇒ 真源，打包时用。
//!
//! ⚠ 产物里**没有**多档状态：编译时按 stage 解算成"一个物体一份材质"
//! （[`Registration::params_for`]）—— 文档形状不变（`SCENE_SCHEMA = 2`），老产物逐字节仍然可比。

use std::collections::BTreeMap;

use px_protocol::material::ParamKind;
use px_protocol::scene::{Material, Value};

/// 一份 stage。它唯一要说清的是**产物里那条 pass 的 `kind`**。
pub trait Stage {
    /// 产物里那条 pass 的 `kind`（`px_pass` 按它分派）。
    const KIND: &'static str;
    /// 给人看的名字（报错用）。
    const LABEL: &'static str;
    /// 登记用的名字（[`Registration::stage`] 认的那个字符串）。
    const STAGE_NAME: &'static str;
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

// 今天的 pipeline（`art/frame/default.toml` 那条）里的档。
stages! {
    Prepass => ("prepass", "geometry", "预通道（只写深度）"),
    PointShadow => ("point_shadow", "geometry", "点光 cube 影子（六面各一条）"),
    Opaque => ("opaque", "geometry", "主 pass：不透明"),
    Transparent => ("transparent", "geometry", "主 pass：透明"),
    Sky => ("sky", "geometry", "天空盒"),
    Fullscreen => ("fullscreen", "fullscreen", "全屏后处理"),
}

/// 一份 stage 从 material 里要的**一个格**：名字 + 类型。
///
/// 「类型」用的是产物那一档词表（[`ParamKind`]）—— 与 `MaterialLayout` 同一份，
/// 所以"这一档要 vec4、shader 声明 vec3"这种分岔在编译时就红。
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

/// **某个 stage 要的 per-pass 参数** —— 图程序里手写这个 impl（一行常量）。
///
/// 判据（[`check`]）：`GIVEN` 里每一个格，都要在这份 material 的参数表里**存在**、
/// 且**类型相等**。多出来的格（这份材质给别的档用的那些）**不算错**。
pub trait StageParams<S: Stage> {
    const GIVEN: &'static [Given];
}

/// 这一袋参数**满足**这一档要的那几个格吗？不满足就把两边的表都列出来。
pub fn check(
    stage: &str,
    material: &str,
    given: &'static [Given],
    params: &BTreeMap<String, Value>,
) -> Result<(), String> {
    let mut missing: Vec<String> = Vec::new();
    let mut wrong: Vec<String> = Vec::new();
    for field in given {
        match params.get(field.name) {
            None => missing.push(format!("{}（{}）", field.name, field.kind.name())),
            Some(value) => {
                let got = kind_of(value);
                if got != Some(field.kind) {
                    wrong.push(format!(
                        "{}：这一档要 {}，材质给的是 {}",
                        field.name,
                        field.kind.name(),
                        got.map(ParamKind::name).unwrap_or("文本"),
                    ));
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
        "stage '{stage}' 要 material '{material}' 给的 per-pass 参数没给全：{why}\n  \
         这一档要的格：{}\n  \
         材质实际给的：{}",
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

/// 一个 `Value` 落在哪一档。与 `ParamKind` 一一对应（文本在参数块里非法 ⇒ `None`）。
pub fn kind_of(value: &Value) -> Option<ParamKind> {
    match value {
        Value::Num(_) => Some(ParamKind::F32),
        Value::Quad(_) => Some(ParamKind::Vec4),
        Value::Triple(_) => Some(ParamKind::Vec3),
        Value::Text(_) => None,
    }
}

/// 一份 stage 的材质：**哪份内容 + 值那一袋**。
///
/// ⚠ 值就是 [`Material::params`] 那一袋（按名字给，打包由 `MaterialLayout` 管），
/// 这里不另造一层参数模型：参数的名字 ↔ 字节只有一份真源（shader 的反射）。
#[derive(Debug, Clone)]
pub struct StageMaterial {
    /// 哪份内容（`"surface"` / `"clouds"` …）：按它查这一档的 per-pass 参数表。
    pub of: String,
    pub params: BTreeMap<String, Value>,
}

impl StageMaterial {
    pub fn new(of: &str, params: BTreeMap<String, Value>) -> Self {
        Self {
            of: of.to_string(),
            params,
        }
    }

    /// 从一份**材质**取（图程序那一层的寻常写法）。
    pub fn of_material(material: &Material) -> Self {
        Self {
            of: material.shader.node.clone(),
            params: material.params.clone(),
        }
    }
}

/// **一个物体注册的那些 stage 材质** + 寻常那一路的"一份材质"包装。
///
/// 两种写法，同一份产物：
///
/// ```ignore
/// // ① 寻常引擎那样：一份材质，几档 pass 共用
/// Registration::single(&material)
/// // ② 分档：影子那一档与主 pass 各不相同
/// Registration::single(&surface).stage("opaque", &emissive_variant)
/// ```
#[derive(Debug, Clone, Default)]
pub struct Registration {
    /// 分档登记：stage 名 → 那一档的材质。
    stages: Vec<(String, StageMaterial)>,
    /// 寻常那一路：没分档的 stage 都用它。
    single: Option<StageMaterial>,
}

impl Registration {
    /// 寻常那一路：**一份材质**，所有 stage 共用（"用起来就像寻常 game engine"）。
    pub fn single(material: &Material) -> Self {
        Self {
            stages: Vec::new(),
            single: Some(StageMaterial::of_material(material)),
        }
    }

    /// 给某一档**另**注册一份材质（覆盖 `single` 那一份）。
    pub fn stage(mut self, stage: &str, material: &Material) -> Self {
        let entry = StageMaterial::of_material(material);
        match self.stages.iter_mut().find(|(name, _)| name == stage) {
            Some(slot) => slot.1 = entry,
            None => self.stages.push((stage.to_string(), entry)),
        }
        self
    }

    /// 手写 per-pass 表的那条路：按 `S` 把这一档登记上。
    pub fn with<S: Stage>(mut self, material: StageMaterial) -> Self {
        self.stages.push((S::STAGE_NAME.to_string(), material));
        self
    }

    /// 这一档落到产物上的那一份材质（分档优先，其次 `single`）。
    pub fn material_for(&self, stage: &str) -> Option<&StageMaterial> {
        if let Some((_, material)) = self.stages.iter().find(|(name, _)| name == stage) {
            return Some(material);
        }
        self.single.as_ref()
    }

    /// 这一档落到产物上的那一袋参数。
    ///
    /// 这一步就是"多档状态只在内存里"那条口径的落点：文档里一个物体仍然只有一份材质，
    /// 而**哪一份**由它被哪一档画决定（分档优先，其次 `single`）。
    pub fn params_for(&self, stage: &str) -> Option<&BTreeMap<String, Value>> {
        self.material_for(stage).map(|material| &material.params)
    }

    /// 这一档的 per-pass 参数给全了吗（[`check`]）。
    pub fn check(
        &self,
        stage: &str,
        given: &'static [Given],
    ) -> Result<(), String> {
        let Some(material) = self.material_for(stage) else {
            return Ok(());
        };
        check(stage, &material.of, given, &material.params)
    }

    /// 登记了哪几档（出现次序；报错与审计用）。
    pub fn registered_stages(&self) -> Vec<&str> {
        self.stages
            .iter()
            .map(|(name, _)| name.as_str())
            .collect()
    }

    pub fn is_single(&self) -> bool {
        self.stages.is_empty() && self.single.is_some()
    }
}

// ---------------------------------------------------------------------------
// 内容材质在**每一档**给的那几个 per-pass 参数
//
// ⚠ 这一栏说的是"这一档 **读**哪几个格"，不是"这份材质**有**哪些格"：
//   `surface` 的参数表里同时住着行星自己的那几格与云影那几个格，而 `opaque` 那一档
//   到底读哪几个由这里说 —— 那份意图抄不下来，只能写。
// ---------------------------------------------------------------------------

/// 行星表面在 `opaque` 那一档给的格。
#[derive(Debug, Clone, Copy)]
pub struct SurfaceOpaque;

impl StageParams<Opaque> for SurfaceOpaque {
    const GIVEN: &'static [Given] = &[
        Given::vec4("orientation"),
        Given::vec4("emissive"),
        Given::num("inner"),
        Given::num("outer"),
        Given::num("coverage"),
        Given::num("shadow"),
        Given::num("height"),
        Given::num("gain"),
    ];
}

/// 云在 `transparent` 那一档给的格（`clouds.wgsl` 的 `CloudParams` 全表）。
#[derive(Debug, Clone, Copy)]
pub struct CloudsTransparent;

impl StageParams<Transparent> for CloudsTransparent {
    const GIVEN: &'static [Given] = &[
        Given::vec4("orientation"),
        Given::vec4("tint"),
        Given::num("inner"),
        Given::num("outer"),
        Given::num("density"),
        Given::num("coverage"),
        Given::num("base"),
        Given::num("top"),
        Given::num("detail_scale"),
        Given::num("detail_strength"),
        Given::num("erode"),
        Given::num("phase"),
        Given::num("shadow"),
        Given::uint("steps"),
        Given::num("bump"),
        Given::uint("seed"),
        Given::uint("ablate"),
        Given::num("slope_scale"),
        Given::num("taper"),
        Given::num("coverage_gain"),
        Given::num("surface_level"),
        Given::uint("bound"),
        Given::uint("gradient"),
        Given::num("wind"),
        Given::num("wind_skin"),
    ];
}

/// 大气在 `transparent` 那一档给的格。
#[derive(Debug, Clone, Copy)]
pub struct AtmosphereTransparent;

impl StageParams<Transparent> for AtmosphereTransparent {
    const GIVEN: &'static [Given] = &[
        Given::num("inner"),
        Given::num("outer"),
        Given::num("density"),
        Given::num("softness"),
        Given::vec4("tint"),
    ];
}

/// 环在 `transparent` 那一档给的格。
#[derive(Debug, Clone, Copy)]
pub struct RingTransparent;

impl StageParams<Transparent> for RingTransparent {
    const GIVEN: &'static [Given] = &[Given::vec4("tint")];
}

/// 天空盒在 `sky` 那一档给的格（`art/frame/skybox.wgsl`，帧自有材质）。
#[derive(Debug, Clone, Copy)]
pub struct SkyboxSky;

impl StageParams<Sky> for SkyboxSky {
    const GIVEN: &'static [Given] = &[Given::num("brightness")];
}

/// 一张表：`(stage, 哪份材质) → 这一档要的那几个格`。
///
/// ⚠ 键必须是**两半**：一个 stage 里往往住着几份**不同的**内容材质（今天 `transparent`
/// 一档里就有云 / 大气 / 环三份，各自读的格完全不同）—— 按 stage 合成一份并集是错的。
#[derive(Debug, Clone, Default)]
pub struct StageParamsTable {
    entries: Vec<(&'static str, &'static str, &'static [Given])>,
}

impl StageParamsTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// 今天那五份内容材质的表。
    pub fn content() -> Self {
        Self::new()
            .with::<Opaque, SurfaceOpaque>("surface")
            .with::<Transparent, CloudsTransparent>("clouds")
            .with::<Transparent, AtmosphereTransparent>("atmosphere")
            .with::<Transparent, RingTransparent>("ring")
            .with::<Sky, SkyboxSky>("skybox")
    }

    /// 登记一份（`material` 是[`StageMaterial::of`] 那个名字）。
    pub fn with<S: Stage, P: StageParams<S>>(mut self, material: &'static str) -> Self {
        self.entries.push((S::STAGE_NAME, material, P::GIVEN));
        self
    }

    /// 这一档 + 这份材质要哪几个格。没登记 = 不查。
    pub fn of(&self, stage: &str, material: &str) -> Option<&'static [Given]> {
        self.entries
            .iter()
            .find(|(name, of, _)| *name == stage && *of == material)
            .map(|(_, _, given)| *given)
    }

    pub fn pairs(&self) -> Vec<(&'static str, &'static str)> {
        self.entries
            .iter()
            .map(|(stage, of, _)| (*stage, *of))
            .collect()
    }
}

/// 一个物体**带着材质**落在哪几档 pass 里 —— 判据与 `frame::draws_of` 的 `select`
/// **同一套谓词**。
///
/// ⚠ 只列**会读它材质参数**的那两档：深度-only 的那两档（`prepass` / `point_shadow`）
/// 的顶点阶段由帧图自带，内容的材质在那两档里一个参数都不参与。
pub fn stages_of(material: &Material, _cast_shadow: bool) -> Vec<&'static str> {
    if material.alpha == px_protocol::scene::AlphaMode::Opaque {
        vec![Opaque::STAGE_NAME]
    } else {
        vec![Transparent::STAGE_NAME]
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

    /// 缺一个格 ⇒ 当场红，而且把缺的那个与这一档要的表都列出来。
    #[test]
    fn a_missing_per_pass_field_names_what_is_missing() {
        let given = <AtmosphereTransparent as StageParams<Transparent>>::GIVEN;
        let err = check(
            Transparent::STAGE_NAME,
            "atmosphere",
            given,
            &params(&[
                ("inner", Value::Num(1.0)),
                ("outer", Value::Num(1.14)),
                ("density", Value::Num(0.3)),
                ("tint", Value::Quad([0.0, 0.0, 0.0, 1.0])),
            ]),
        )
        .expect_err("少了 softness");
        assert!(err.contains("softness"), "{err}");
        assert!(err.contains("缺参数"), "{err}");
    }

    /// 类型不符要点名两边（`tint` 是 vec4，给 vec3 不算数）。
    #[test]
    fn a_wrong_kind_names_both_sides() {
        let given = <RingTransparent as StageParams<Transparent>>::GIVEN;
        let err = check(
            Transparent::STAGE_NAME,
            "ring",
            given,
            &params(&[("tint", Value::Triple([1.0, 1.0, 1.0]))]),
        )
        .expect_err("vec3 ≠ vec4");
        assert!(err.contains("tint"), "{err}");
        assert!(err.contains("类型不符"), "{err}");
    }

    /// **多出来的格不算错**：一份材质往往同时被几档 pass 用，判据是"这一档要的那几个格"。
    #[test]
    fn extra_names_are_not_a_failure() {
        let given = <RingTransparent as StageParams<Transparent>>::GIVEN;
        check(
            Transparent::STAGE_NAME,
            "ring",
            given,
            &params(&[
                ("tint", Value::Quad([1.0, 1.0, 1.0, 1.0])),
                ("这是别的档要的格", Value::Num(1.0)),
            ]),
        )
        .expect("子集判据：多给的不是错");
    }

    /// **一个 stage 住着几份不同的材质** ⇒ 表必须按 `(stage, 材质)` 查，不能按 stage 合并。
    #[test]
    fn a_stage_with_several_materials_keeps_their_tables_apart() {
        let table = StageParamsTable::content();
        let clouds = table.of(Transparent::STAGE_NAME, "clouds").expect("云的");
        let atmosphere = table.of(Transparent::STAGE_NAME, "atmosphere").expect("大气的");
        assert!(clouds.iter().any(|field| field.name == "steps"), "云那一档要 steps");
        assert!(
            !atmosphere.iter().any(|field| field.name == "steps"),
            "大气那一档不该要云的那些格（并入就是错的）"
        );
        assert!(table.of(Transparent::STAGE_NAME, "没这个名字").is_none());
    }

    /// 寻常那一路：**一份材质**，几档 pass 共用 —— 解出来的就是那一袋。
    #[test]
    fn a_single_registration_resolves_for_every_stage() {
        let mut material =
            Material::new(px_protocol::scene::Member::new("shaders", "atmosphere", "00"));
        material.params = params(&[("density", Value::Num(0.3))]);
        let registration = Registration::single(&material);
        assert!(registration.is_single());
        assert_eq!(
            registration.params_for(Transparent::STAGE_NAME).unwrap()["density"],
            Value::Num(0.3)
        );
        assert_eq!(
            registration.params_for(Opaque::STAGE_NAME).unwrap()["density"],
            Value::Num(0.3),
            "没分档 ⇒ 每一档都用那一份"
        );
    }

    /// 分档：影子那一档可以另有一份材质，而不透明那一档仍是原来那份。
    #[test]
    fn a_stage_specific_registration_overrides_the_single_one() {
        let mut base = Material::new(px_protocol::scene::Member::new("shaders", "surface", "00"));
        base.params = params(&[("coverage", Value::Num(0.1))]);
        let mut shadow = Material::new(px_protocol::scene::Member::new("shaders", "surface", "00"));
        shadow.params = params(&[("coverage", Value::Num(0.9))]);

        let registration = Registration::single(&base).stage(PointShadow::STAGE_NAME, &shadow);
        assert!(!registration.is_single());
        assert_eq!(
            registration.params_for(PointShadow::STAGE_NAME).unwrap()["coverage"],
            Value::Num(0.9)
        );
        assert_eq!(
            registration.params_for(Opaque::STAGE_NAME).unwrap()["coverage"],
            Value::Num(0.1),
            "没点名的档仍走 single 那一份"
        );
    }

    /// 手写表那条路：`with::<S>` 把这一档登记上，而材质名字取自那份材质。
    #[test]
    fn the_typed_registration_keeps_the_material_identity() {
        let mut material = Material::new(px_protocol::scene::Member::new("shaders", "ring", "00"));
        material.params = params(&[("tint", Value::Quad([1.0, 1.0, 1.0, 1.0]))]);
        let registration =
            Registration::default().with::<Transparent>(StageMaterial::of_material(&material));
        let registered = registration
            .material_for(Transparent::STAGE_NAME)
            .expect("登记上了");
        assert_eq!(registered.of, "ring");
        assert!(
            registration
                .check(Transparent::STAGE_NAME, <RingTransparent as StageParams<Transparent>>::GIVEN)
                .is_ok()
        );
    }

    /// 一个物体落在哪一档：不透明 → `opaque`，透明 → `transparent`；
    /// 深度-only 的那两档**不在**这张表里（它们不读材质参数）。
    #[test]
    fn the_stages_of_an_object_are_the_ones_that_read_its_material() {
        let mut material = Material::new(px_protocol::scene::Member::new("shaders", "surface", "00"));
        assert_eq!(stages_of(&material, true), vec!["opaque"]);
        material.alpha = px_protocol::scene::AlphaMode::Add;
        assert_eq!(stages_of(&material, false), vec!["transparent"]);
    }
}