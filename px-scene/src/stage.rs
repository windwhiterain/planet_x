//! **pipeline 的 stage 与它要求的 per-pass material 参数** —— 全部**按类型**说，不用字符串当身份。
//!
//! ## 为什么这一层一个 `dyn` 都没有
//!
//! 这一层要回答的两个问题都**是类型问题**：
//!
//! 1. **这一档读材质里的哪几个格** ⇒ [`StageParams`]，按 `(stage 类型, 内容类型)` 两条轴手写；
//! 2. **这份内容参与哪几档** ⇒ [`Content::Stages`]，一个类型级列表。
//!
//! 于是"打错档位 / 漏写某一档的参数表 / 给某份内容登记它不参与的档"都是**编译错误**，
//! 而不是烘图时才红。用字符串当 stage 身份（`"opaque"` 那种）会把这两条都推到运行期，
//! 而且**擦掉类型**：`Vec<(String, ...)>` 里再也看不出登记了哪几档，编译器帮不了任何忙。
//!
//! ## 多种 pipeline
//!
//! [`Stage`] 与 [`Content`] 都是 **trait** ⇒ 另一条 pipeline 自带**它自己的一套 stage 类型**
//! （各自的 per-pass 参数契约、各自的语义），与 `default` 那六档互不干扰：一个新 pipeline
//! 只需要定义自己的零尺寸 stage 类型 + 为它要用的内容写 `StageParams`。
//! `px_pass` 只按产物里那条 pass 的 `kind` 分派，它不认识任何具体档位（§124）。
//!
//! ## 产物那一侧
//!
//! ⚠ 产物里**没有**多档状态，也没有类型：文档里一个物体仍然是"一份材质 + 一袋
//! `BTreeMap<String, Value>`"。那一袋**不是**擦除 —— 它就是 `.pxart` 的数据形状
//! （参数的名字 ↔ 字节由 shader 的反射说了算，见 [`crate::contract`]）。
//! 擦除指的是**作者这一层**：这一层的 API 与模型不许出现 `dyn` / `Box<dyn>` / 字符串身份。

use std::collections::BTreeMap;
use std::marker::PhantomData;

use px_protocol::material::ParamKind;
use px_protocol::scene::Value;

/// 一份 stage。**零尺寸类型**：它的身份就是它的类型本身。
pub trait Stage {
    /// 产物里那条 pass 的 `kind`（`px_pass` 按它分派）。
    const KIND: &'static str;
    /// 给人看的名字（报错用）。
    const LABEL: &'static str;
    /// 产物与标签那一侧的名字（`PassSpec::label` 的形状、帧图配方的 `select`）。
    ///
    /// ⚠ 它**只是**往下写文档时用的字，**不是**这一层用来查表的键 —— 查表一律走类型。
    const STAGE_NAME: &'static str;
}

/// 一个格：**名字 + 类型**。
///
/// ⚠ 名字是 `&'static str` 而不是 const 泛型参数：`&[Given<"a">, Given<"b">]` 里两个元素
/// **类型不同**，凑不成一个数组。格是**元数据**（这一档要查哪几个名字），它的"类型检查"由
/// [`StageParams`] 的实现（谁给这一对组合写表）承担，不靠名字进类型。
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

/// 一份**内容**（一份材质所代表的东西：表面 / 云 / 大气 / 环 / 天空盒 / 未来任何东西）。
///
/// `Stages` 是**类型级列表**：这份内容参与哪几档。它同时是检查清单 ——
/// [`CheckStages`] 会按它逐档展开，于是**漏写某一档的 `StageParams` 是编译错误**。
pub trait Content: Sized {
    type Stages;

    /// 产物那一侧的名字（材质成员的节点名，如 `"surface"`）：**只进报错与文档**。
    fn name() -> &'static str;
}

/// **这一档 + 这份内容**要的那几个格。
///
/// 两条轴都用类型：`S` 是 stage，`M` 是内容。漏写某一对组合 ⇒ **编译期**就报
/// "`StageParams<X, Y>` 没有实现"。
///
/// ⚠ 这一栏说的是"这一档**读**哪几个格"，**不是**"这份材质**有**哪些格"：
/// 一份材质的参数表往往是几档的并集（`surface` 的结构体里就住着云影那几个格），
/// 而某一档只碰其中几个 ⇒ 判据是**子集 + 类型相等**，多出来的格不算错。
pub trait StageParams<S: Stage, M: Content> {
    const GIVEN: &'static [Given];
}

/// [`Registration::check_all`] 那一半：按 `M::Stages` 逐档展开。
///
/// 由 [`impl_check_stages!`] 为每一份内容生成。⚠ 这不是擦除：展开出来的是**每一档一次
/// 具体类型**的调用（`self.check::<$stage>()`），单态化之后每档一份代码。
pub trait CheckStages {
    fn check_stages(&self) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// `default` 那条 pipeline 的六档
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 今天那五份内容 + 每一对的 per-pass 表（**手写**，不从 WGSL 反推）
//
// ⚠ 从反射生成这一栏就等于"shader 声明什么就是什么" —— 那不是格式，那是抄本。
//   反射管的是另一半：参数的**名字 ↔ 字节**（`crate::contract`）。
// ---------------------------------------------------------------------------

/// 一份内容 + 它参与的那几档 + **每一档的格**。
///
/// 写法：`内容, "名字", [档位…]; 档位 => [格…], 档位 => [格…]`。
/// ⚠ `[档位…]` 那张清单**不带重复**（`CheckStages` 要恰好一次），而 `档位 => [..]` 可以按档
/// 出现多次（宏把它们**并起来**成那一档的表）。三件一起写下来 ⇒ "参与哪几档"与"每档要什么"
/// 不会再漂开。
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

    // 一份内容在某一档上的表：同名同档出现多次时，后一条通过 `stage_state!` 追加。
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

/// 一个 `Value` 落在哪一档。与 [`ParamKind`] 一一对应（文本在参数块里非法 ⇒ `None`）。
///
/// ⚠ 只用于报错信息；**判据不许只看它**（见 [`satisfies`]）。
pub fn kind_of(value: &Value) -> Option<ParamKind> {
    match value {
        Value::Num(_) => Some(ParamKind::F32),
        Value::Quad(_) => Some(ParamKind::Vec4),
        Value::Triple(_) => Some(ParamKind::Vec3),
        Value::Text(_) => None,
    }
}

/// 这个值**塞得进**这一档要的那个格吗？
///
/// ⚠ 判据是"**打得进去吗**"，不是"值在 Rust 里是不是那个类型"：参数块的打包（
/// `MaterialLayout::pack`，唯一那份真源）**允许把整数写成 `Value::Num`** 再 `as u32`
/// —— `steps` / `seed` / `ablate` / `bound` / `gradient` 这几个 `u32` 格在既有配方里
/// 就是按数给的，落盘的字节也一直是对的。只看 [`kind_of`] 会把它们判成"类型不符"，
/// 那是**假红**：判据与真源各说一套，最坏的结局是把好端端的内容逼着改。
///
/// ⇒ 整数那一档：`Value::Num` 只要**没有小数部分**且落在范围内就算满足；小数则点名拒。
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

/// 这一袋参数**满足**这一档要的那几个格吗？不满足就把两边的表都列出来。
pub fn check<S: Stage, M: Content>(params: &BTreeMap<String, Value>) -> Result<(), String>
where
    S: StageParams<S, M>,
{
    check_given::<S, M>(<S as StageParams<S, M>>::GIVEN, params)
}

/// [`check`] 的"给定表"那一半 —— 两份调用共用同一段判据（抄第二份就是第二个真相）。
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

/// **运行期那一条路**（TOML 配方适配器专用）。
///
/// ⚠ 它存在的原因必须说清楚：配方是**数据**，一份 `art/scene/*.toml` 里的 `kind` / `shader`
/// 是字符串 ⇒ 走到这里时"这是哪份内容"已经不在类型里了。所以这条桥**故意**是运行期的，
/// 而且**只有它**是：图程序里手写的场景请走 [`Registration<Stage, Content>`]（类型级）。
///
/// 桥的这一侧内容类型独立（[`RuntimeContent`]），于是适配器**不需要**对五份内容各写一遍 ——
/// 那是 `dyn` 之外唯一能把运行期那一维补回来的做法，而它把"两个类型"的代价摆在明面上。
pub mod runtime {
    use super::*;
    use px_protocol::scene::AlphaMode;

    /// 运行期认得出的一档（配方里的 stage 名）。
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
        /// 产物标签与配方里那一个字。
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

    /// 运行期认得出的内容（配方里的 `shader` 名）。
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

    /// 这个物体被**哪一档**读材质参数 —— 与 `frame::draws_of` 的 `opaque` / `transparent`
    /// 两个 `select` 同一套谓词。
    ///
    /// ⚠ 深度-only 的那两档（预通道 / 影子）不读材质参数，所以不在这里。
    pub fn stage_of_alpha(alpha: AlphaMode) -> RuntimeStage {
        if alpha == AlphaMode::Opaque {
            RuntimeStage::Opaque
        } else {
            RuntimeStage::Transparent
        }
    }

    /// 运行期的对账：**表与判据仍然是类型级那两份**（`StageParams` 的实现 + [`check`]），
    /// 这里只做两个 `match` 的分派。
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
            // 这一档 + 这份内容**没有表**：不是错误（这一档压根不读这份材质的参数），
            // 而是"这里没什么可查的"。
            _ => Ok(()),
        };
        checked.map_err(|err| format!("内容 '{}'：{err}", content.name()))
    }
}

// ---------------------------------------------------------------------------
// 登记：`Registration<S, M>` —— **stage 与内容都是类型参数**
// ---------------------------------------------------------------------------

/// 一个物体登记的"某一档的那份材质"。
#[derive(Debug, Clone)]
struct Slot {
    stage: &'static str,
    params: BTreeMap<String, Value>,
}

/// 一个物体的 **stage 材质登记**，按类型带上 stage 与内容：[`Registration<S, M>`]。
///
/// 寻常那一路只有一份材质（几档共用，`Registration::single`）；分档那条路给某一档另注册
/// 一份（[`Registration::stage`]）—— 而**两条路都在类型里说清是谁**。
///
/// ⚠ 内部那张表仍然按名字存（产物与标签那一侧要名字），但**这一层不向外露擦除句柄**：
/// 入口是类型化的 `single` / `stage`，出口是类型化的 `check::<T>()` / `check_all()` / `freeze`。
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

    /// 寻常那一路：**一份材质**，几档 pass 共用。
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

    /// 给某一档**另**注册一份参数（覆盖 `single` 那一份）。
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

    /// 这一档落到产物上的那一袋参数（分档优先，其次 `single`）。
    pub fn params_of(&self, stage: &str) -> Option<&BTreeMap<String, Value>> {
        if let Some(slot) = self.slots.iter().find(|slot| slot.stage == stage) {
            return Some(&slot.params);
        }
        self.default.as_ref().map(|slot| &slot.params)
    }

    /// **这一档的对账**：登记的材质给全了这一档要的那几个格吗。
    ///
    /// 三样都在类型里：`T` 是档位、`M` 是内容、`T: StageParams<T, M>` 这条界保证
    /// **这一对组合的参数表一定存在**（漏写就是编译错误）。
    pub fn check<T: Stage>(&self) -> Result<(), String>
    where
        T: StageParams<T, M>,
    {
        let Some(params) = self.params_of(T::STAGE_NAME) else {
            return Ok(());
        };
        check::<T, M>(params)
    }

    /// 按 `M::Stages` 那张类型级清单**逐档**对账（[`CheckStages`] 由 `contents!` 生成）。
    pub fn check_all(&self) -> Result<(), String>
    where
        Self: CheckStages,
    {
        self.check_stages()
    }

    /// 产物那一份参数：**按这一档解算**（分档优先，其次 `single`）。
    ///
    /// ⚠ 文档里一个物体仍然只有一份材质 ⇒ 这里把多档状态解算掉。今天每个物体只被一档读
    /// 材质参数（不透明 xor 透明），所以落盘的就是那一档那一份。
    pub fn freeze(&self, document_stage: &str) -> BTreeMap<String, Value> {
        self.params_of(document_stage)
            .or_else(|| self.default.as_ref().map(|slot| &slot.params))
            .cloned()
            .unwrap_or_default()
    }

    /// 登记的档位（审计 / 报错）。
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

    /// **正对照**：给全了就不红。
    #[test]
    fn a_complete_material_passes_its_stage() {
        Registration::<Transparent, Atmosphere>::single(atmosphere_params())
            .check_all()
            .expect("五格都给全了");
    }

    /// 缺一格 ⇒ 当场红，而且要点名缺的是哪个、是哪份内容。
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

    /// 类型不符要点名两边（`tint` 是 vec4，给 vec3 不算数）。
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

    /// **多出来的格不算错**：一份材质往往同时被几档 pass 用。
    #[test]
    fn extra_names_are_not_a_failure() {
        let mut extra = atmosphere_params();
        extra.insert("这是别的档要的格".to_string(), Value::Num(1.0));
        Registration::<Transparent, Atmosphere>::single(extra)
            .check_all()
            .expect("子集判据：多给的不是错");
    }

    /// **一个 stage 里住着几份不同的内容**：各自只查自己那一档的表。
    ///
    /// 这条钉的是"按 `(stage, 内容)` 两条轴查"：云那一档要 `steps`，大气那一条不该要它。
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

    /// **分档登记**：某一档另有参数，而没点名的档仍走 `single` 那一份。
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

    /// 天空盒那一份内容只参与 `sky` 一档 —— 类型清单说的，不是运行期查的。
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

    /// 环那一档只要一个 `tint`。
    #[test]
    fn the_ring_content_needs_only_its_tint() {
        Registration::<Transparent, Ring>::single(params(&[(
            "tint",
            Value::Quad([1.0, 1.0, 1.0, 1.0]),
        )]))
        .check_all()
        .expect("一个格");
    }

    /// **整数那一档收 `Value::Num`（整数值）** —— 参数块的打包本来就这么收
    /// （`MaterialLayout::pack` 对 `u32` 的判据是 `number.fract() != 0.0`）。
    ///
    /// ⚠ 这条是**踩过之后补的**：判据原先只看 `kind_of`，于是把 `steps` / `seed` /
    /// `ablate` / `bound` / `gradient` 那五个 `u32` 格判成"类型不符"，而既有配方一直是
    /// 按数给它们的、落盘字节也一直是对的 ⇒ 那是**假红**。云的参数表里这五个格就是夹具。
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

    /// 但**带小数**的给整数格要拒，而且报错要点名那个值。
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

    /// 判据本身就是"塞得进吗"：四数给 vec3、三数给 vec4 都拒；数与向量的分岔也拒。
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

    /// `freeze` 按这一档解算（分档优先、其次 `single`）。
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
