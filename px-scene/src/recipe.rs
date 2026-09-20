//! **配方**（`art/scene/*.toml`）的形状与它的语义组装。
//!
//! 配方文件的形状**没变**（还是 `[[parts]]` + `kind` + `members` + `params`）：它是艺术内容。
//! 变的是它编译成什么、以及**这份语义住在哪** —— 它从 `px_graphs/src/bin/scene.rs`
//! 搬进了这个库，于是 pcg 的图程序（以及将来的工具）用的是同一份语义。
//!
//! ⚠ 倾斜（[`SYSTEM_TILT`]）只在这里出现一次：文档里的方向与朝向**都是世界系**的。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_graph::generate::{self, Palette};
use px_protocol::art::Camera;
use px_protocol::scene::{
    AlphaMode, CullMode, Environment, Geometry, Light, Material, Object, Sampler, SceneSpec,
    TextureRef, Transform, Value,
};

use crate::baked::Baked;
use crate::contract::{merge_named, schema_of};
use crate::members::{member_of, path_of};
use crate::vocab::{
    self, ATMOSPHERE_KEYS, CLOUD_BASE, CLOUD_SHADOW_GAIN, CLOUD_SHADOW_HEIGHT, CLOUD_TOP,
    CLOUDS_KEYS, CloudShape, MOON_KEYS, PLANET_KEYS, RING_BAND, RING_SEGMENTS, SKYBOX_BRIGHTNESS,
    STARS_FACE, SUN_RANGE_FACTOR,
};
use crate::{math, stage};

use serde::Deserialize;

/// 场景配方：这次要渲什么、用什么参数。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneFile {
    pub name: String,
    #[serde(default)]
    pub ambient: f32,
    /// `"review"` = 评审相机表；缺省也用它。
    #[serde(default)]
    pub cameras: Option<String>,
    /// 用哪张**帧图**（`art/frame/<名>.toml`，§128）。不写 = `default`。
    #[serde(default)]
    pub frame: Option<String>,
    pub parts: Vec<PartFile>,
}

/// 配方里的一个 part。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartFile {
    pub id: String,
    pub kind: String,
    /// 这一 part 用哪份 shader（成员名）。⚠ **可选**：`kind = "light"` 那一档**没有材质**
    /// （灯不是物体）—— 2026-09-20 加第二光源（地球反照）时发现的：原先它是必填，
    /// 于是"灯"这种 part 在配方里根本写不出来。要材质的那几档在装配时自己会报"没给 shader"。
    #[serde(default)]
    pub shader: String,
    /// 这个 part 的成员默认属于哪张图；跨图的成员写 `图名::节点名`。
    #[serde(default)]
    pub graph: Option<String>,
    /// **本体的几何**（只对 `kind = "planet"` 有意义）：不写 = 用 `members` 里那个网格；
    /// 写 `"icosphere"` = 用内建球（气态巨行星那种"可见面就是光球层"的天体）。
    /// ⚠ 内建球之后 `mesh` 成员就不再需要 —— 见 `compile` 里那一段的理由。
    #[serde(default)]
    pub primitive: Option<String>,
    #[serde(default)]
    pub members: BTreeMap<String, String>,
    #[serde(default)]
    pub params: BTreeMap<String, toml::Value>,
}

impl PartFile {
    pub fn number(&self, key: &str) -> Result<f64, String> {
        match self.params.get(key) {
            Some(toml::Value::Integer(value)) => Ok(*value as f64),
            Some(toml::Value::Float(value)) => Ok(*value),
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要一个数，实际是 {other:?}",
                self.id
            )),
            None => Err(format!(
                "part '{}' 缺参数 '{key}'；它有的参数：{}",
                self.id,
                if self.params.is_empty() {
                    "（空）".to_string()
                } else {
                    self.params.keys().cloned().collect::<Vec<_>>().join(" / ")
                }
            )),
        }
    }

    pub fn number_or(&self, key: &str, fallback: f32) -> f32 {
        match self.params.get(key) {
            Some(toml::Value::Integer(value)) => *value as f32,
            Some(toml::Value::Float(value)) => *value as f32,
            _ => fallback,
        }
    }

    pub fn integer_or(&self, key: &str, fallback: u32) -> u32 {
        match self.params.get(key) {
            Some(toml::Value::Integer(value)) => (*value).max(0) as u32,
            Some(toml::Value::Float(value)) => (*value).max(0.0) as u32,
            _ => fallback,
        }
    }

    pub fn text(&self, key: &str) -> Result<&str, String> {
        match self.params.get(key) {
            Some(toml::Value::String(value)) => Ok(value),
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要一段文本，实际是 {other:?}",
                self.id
            )),
            None => Err(format!("part '{}' 缺参数 '{key}'", self.id)),
        }
    }

    pub fn text_or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        match self.params.get(key) {
            Some(toml::Value::String(value)) => value,
            _ => fallback,
        }
    }

    pub fn triple(&self, key: &str) -> Result<[f32; 3], String> {
        match self.params.get(key) {
            Some(toml::Value::Array(items)) if items.len() == 3 => {
                let mut out = [0.0_f32; 3];
                for (slot, item) in out.iter_mut().zip(items.iter()) {
                    let number = item
                        .as_float()
                        .or_else(|| item.as_integer().map(|value| value as f64))
                        .ok_or_else(|| {
                            format!("part '{}' 参数 '{key}' 里有一个不是数：{item:?}", self.id)
                        })?;
                    *slot = number as f32;
                }
                Ok(out)
            }
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要三个数，实际是 {other:?}",
                self.id
            )),
            None => Err(format!("part '{}' 缺参数 '{key}'", self.id)),
        }
    }

    /// 云的一个形状档。
    pub fn cloud_shape(&self) -> CloudShape {
        let default = CloudShape::default();
        CloudShape {
            coverage: self.number_or("coverage", default.coverage),
            base: self.number_or("base", default.base),
            top: self.number_or("top", default.top),
            detail_scale: self.number_or("detail_scale", default.detail_scale),
            detail_strength: self.number_or("detail_strength", default.detail_strength),
            erode: self.number_or("erode", default.erode),
            phase: self.number_or("phase", default.phase),
            shadow: self.number_or("shadow", default.shadow),
            steps: self.integer_or("steps", default.steps),
            bump: self.number_or("bump", default.bump),
            seed: self.integer_or("seed", default.seed),
            slope_scale: self.number_or("slope_scale", default.slope_scale),
            taper: self.number_or("taper", default.taper),
            coverage_gain: self.number_or("coverage_gain", default.coverage_gain),
            surface_level: self.number_or("surface_level", default.surface_level),
            bound: self.integer_or("bound", default.bound),
            gradient: self.integer_or("gradient", default.gradient),
            wind: self.number_or("wind", default.wind),
            wind_skin: self.number_or("wind_skin", default.wind_skin),
        }
    }

    /// 按角色取一个成员（配方里 `members` 那张表）。
    pub fn member(&self, role: &str) -> Result<px_protocol::scene::Member, String> {
        let reference = self.members.get(role).ok_or_else(|| {
            format!(
                "part '{}'（kind {}）要成员 '{role}'；这份有：{}",
                self.id,
                self.kind,
                if self.members.is_empty() {
                    "（空）".to_string()
                } else {
                    self.members.keys().cloned().collect::<Vec<_>>().join(" / ")
                }
            )
        })?;
        let what = format!("part '{}' 的成员 '{role}'", self.id);
        crate::members::reference(&what, reference, self.graph.as_deref())
    }

    /// 可选成员。
    pub fn optional_member(&self, role: &str) -> Result<Option<px_protocol::scene::Member>, String> {
        match self.members.get(role) {
            None => Ok(None),
            Some(reference) => {
                let what = format!("part '{}' 的成员 '{role}'", self.id);
                crate::members::reference(&what, reference, self.graph.as_deref()).map(Some)
            }
        }
    }

    /// part 用的那份 WGSL。配方里 `shader = "surface"` 是给人看的名字，真本由
    /// `members.shader` 指 —— 两个对不上就报错：否则改了一处、另一处还写着老名字，
    /// 读配方的人会以为换的是另一个 shader。
    pub fn shader_member(&self) -> Result<px_protocol::scene::Member, String> {
        let member = self.member("shader")?;
        if member.node != self.shader {
            return Err(format!(
                "part '{}'：`shader = \"{}\"` 与成员 `shader = \"{}\"` 指的不是同一份 WGSL",
                self.id, self.shader, member.node
            ));
        }
        Ok(member)
    }
}

/// 一份已经组装好、准备进帧图的场景。
pub struct Compiled {
    pub document: SceneSpec,
    pub name: String,
}

/// **编译**：配方 → 通用渲染文档（不含帧图那三节）。
///
/// `with_graph = false` 是**兼容逃生门**（`--no-frame-graph`）：帧图那三节两栏都空，
/// 产物因此与没有帧图时**逐字节相同**（六份冻产物的 sha256 是这条的判据）。
/// ⚠ 它不是"另一种受支持的烘法" —— 它存在的唯一目的是证明老产物还能逐字节复现。
pub fn compile(
    file: &SceneFile,
    baked: &mut Baked,
    with_graph: bool,
) -> Result<Compiled, String> {
    let root = px_graph::cache_root();
    let planet = file
        .parts
        .iter()
        .find(|part| part.kind == "planet")
        .ok_or_else(|| "场景里没有 kind planet 的 part：主体（height / mesh / palette / …）全在它身上".to_string())?;
    let clouds = file.parts.iter().find(|part| part.kind == "clouds");
    let atmosphere = file.parts.iter().find(|part| part.kind == "atmosphere");
    for part in &file.parts {
        check_kind(part)?;
    }

    // 配方的参数名不再在这里按白名单查：判据换成这份 shader 的契约（见 `merge`）。
    let palette_name = planet.text("palette")?;
    let palette = Palette::parse(palette_name).ok_or_else(|| {
        format!(
            "part '{}' 参数 'palette' 是 '{palette_name}'，认不出来；可用：{}",
            planet.id,
            Palette::NAMES.join(" / ")
        )
    })?;
    let radius = planet.number("radius")? as f32;
    let spin = planet.number_or("spin", 0.0);
    let sea_level = planet.number("sea_level")? as f32;
    // `displace` 只对「从场现造球面网格」那条路有意义 —— 那条路随渲染器里的程序化网格
    // 一起没了（网格现在是 `planet` 图的产物）。配方里留着它是因为它本来就是场那一侧的数。
    let _displace = planet.number_or("displace", 0.075);
    let rings = planet.number_or("rings", 0.0);
    let world = math::orientation(spin, vocab::SYSTEM_TILT);

    // ---- 生成物：色板贴图 + 星空 ----
    // ⚠ 本体那份 shader 决定**要烘哪些生成物**（`contract::schema_of` 读它的贴图格）：
    //   自写的 surface 要 `albedo@1` / `glow@3`，而气态巨行星那份（`gasgiant`）要的是
    //   一张**条带立方图**（`bands@7`，成员由配方给、图烘出来）。
    //   ⇒ "生成物只为消费者烘"：没有消费者的贴图不再白烘（也不再多出几 MB 的产物字节）。
    let surface_shader = planet.shader_member()?;
    let surface_layout = crate::contract::schema_of(&surface_shader, &root)?;
    let wants_texture = |binding: u32| {
        surface_layout
            .textures
            .iter()
            .any(|slot| slot.binding == binding)
    };

    // ⚠ 高度场成员**只在色板那条路上要**：本体 shader 声明了 `@binding(1)`（albedo）才读它。
    //   气态巨行星那份（次表面散射）没有 albedo 贴图 ⇒ 配方不必给 `height` —— 于是
    //   "一份生成物只有一个消费者"这条也落到了成员上。
    let height = match planet.optional_member("height")? {
        Some(member) => {
            let path = path_of(&member, &root)?;
            Some(
                generate::load_field(&path.display().to_string())
                    .map_err(|err| format!("读场 {} 失败：{err}", path.display()))?,
            )
        }
        None => None,
    };
    let (color_member, glow_member) = if wants_texture(1) {
        let height = height.as_ref().ok_or_else(|| {
            format!(
                "part '{}' 的本体 shader 声明了 albedo 贴图（@binding(1)）⇒ 配方的 members \
                 里要给 `height`（色板贴图是从那张场烘的）",
                planet.id
            )
        })?;
        let (color, glow, audit) = generate::surface_color(height, palette, sea_level);
        println!("{audit}");
        let color_member = Some(baked.texture("surface_color", color, "texture.palette")?);
        let glow_member = match glow {
            Some(glow) if wants_texture(3) => {
                Some(baked.texture("surface_glow", glow, "texture.palette")?)
            }
            _ => None,
        };
        (color_member, glow_member)
    } else {
        (None, None)
    };
    let stars_member = baked.texture("stars", generate::stars(STARS_FACE), "texture.stars")?;

    // ---- 云：壳、覆盖度立方图、形状档 ----
    let (cloud_inner, cloud_outer, cloud_shape, coverage_member) = match clouds {
        Some(clouds) => {
            let shape = clouds.cloud_shape();
            let inner = radius * clouds.number_or("inner", CLOUD_BASE) as f32;
            let outer = radius * clouds.number_or("outer", CLOUD_TOP) as f32;
            let mask_member = clouds.member("field")?;
            let mask = generate::load_field(&path_of(&mask_member, &root)?.display().to_string())
                .map_err(|err| format!("读云场失败：{err}"))?;
            let mut slopes = Vec::new();
            for role in ["slope_x", "slope_y", "slope_z"] {
                let member = clouds.member(role)?;
                slopes.push(
                    generate::load_field(&path_of(&member, &root)?.display().to_string())
                        .map_err(|err| format!("读云场 {role} 失败：{err}"))?,
                );
            }
            let cube = generate::coverage_cube(&mask, [&slopes[0], &slopes[1], &slopes[2]])
                .map_err(|err| format!("覆盖度立方图：{err}"))?;
            let member = baked.texture("cloud_coverage", cube, "texture.coverage")?;
            (inner, outer, shape, Some(member))
        }
        None => {
            let default = CloudShape::default();
            (radius * CLOUD_BASE, radius * CLOUD_TOP, default, None)
        }
    };

    // ---- 物体 ----
    let mut objects: Vec<Object> = Vec::new();

    // 行星本体：走自写材质（`surface` 或别的自写本体 shader，如 `gasgiant`）。
    // 云影那几个参数与云材质同一口径。
    let cloud_shadow = if coverage_member.is_some() {
        planet.number_or("cloud_shadow", 0.0)
    } else {
        0.0
    };
    let has_glow = glow_member.is_some();
    // 行星的材质参数**全是算出来的**（云影那几个量必须与云材质同口径）⇒ `computed` 就是全部；
    // 但配方里仍然可以按名字**补**这份 shader 声明过的其它参数（§80 第 2 步的透传）。
    // ⚠ **算出来的那些按这份 shader 声明了什么过滤**：本体 shader 不只有 `surface` 一种
    //   （气态巨行星那份 `gasgiant` 要的是 `wrap` / `absorption` / `thickness` 这一族，
    //   没有 `inner` / `coverage` / `emissive`）—— 把用不到的名字塞进参数表就是"多给参数"，
    //   会在 `pack` 那一关当场红。过滤掉不等于放过：**shader 声明了而没人给** 仍然会红。
    let mut computed = BTreeMap::from([
        ("orientation".to_string(), Value::Quad(world)),
        (
            "emissive".to_string(),
            Value::Quad(if has_glow {
                [3.0, 3.0, 3.0, 1.0]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            }),
        ),
        ("inner".to_string(), Value::Num(f64::from(cloud_inner))),
        ("outer".to_string(), Value::Num(f64::from(cloud_outer))),
        (
            "coverage".to_string(),
            Value::Num(f64::from(cloud_shape.coverage)),
        ),
        (
            "shadow".to_string(),
            Value::Num(f64::from(cloud_shadow)),
        ),
        (
            "height".to_string(),
            Value::Num(planet.number_or("shadow_height", CLOUD_SHADOW_HEIGHT) as f64),
        ),
        ("gain".to_string(), Value::Num(f64::from(CLOUD_SHADOW_GAIN))),
    ]);
    computed.retain(|name, _| surface_layout.param(name).is_some());
    let surface_params = material_params(
        planet,
        &surface_shader,
        &PLANET_KEYS,
        computed,
        &root,
    )?;
    let mut surface = Material::new(surface_shader).with_params(surface_params);
    if let Some(color) = &color_member {
        surface = surface.with_texture(
            "albedo",
            TextureRef::new(1, color.clone(), Sampler::repeat()),
        );
    }
    if let Some(glow) = &glow_member {
        surface = surface.with_texture("glow", TextureRef::new(3, glow.clone(), Sampler::repeat()));
    }
    if let Some(coverage) = &coverage_member {
        surface = surface.with_texture(
            "coverage",
            TextureRef::new(5, coverage.clone(), Sampler::clamped()),
        );
    }
    // 条带立方图（气态巨行星那一档）：配方给一个 **CubeMap 场成员**，这里把它烘成
    // 一张立方贴图挂到 `@binding(7)`（`TEXTURE_SLOTS` 里第二格 cube）。
    // ⚠ 与覆盖度那张的差别：不掺梯度 —— 它是"把一张场当数据贴图"，不是云的细节场。
    // 第三层次（第 10 轮）：**第二张立方图**。成员名 `detail`，进 21 号格
    // （`TEXTURE_SLOTS` 里那一对空着的 cube）。⚠ 可选：不给成员就吃渲染器的兜底贴图，
    // 于是"没有这一层的场景"产物与从前逐字节相同。
    if let Some(detail_member) = planet.optional_member("detail")? {
        let field = generate::load_field(&path_of(&detail_member, &root)?.display().to_string())
            .map_err(|err| format!("读细丝场失败：{err}"))?;
        let cube = generate::field_cube(&field).map_err(|err| format!("细丝立方图：{err}"))?;
        let member = baked.texture("gas_filaments", cube, "texture.filaments")?;
        surface = surface.with_texture("detail", TextureRef::new(21, member, Sampler::clamped()));
    }
    if let Some(band_member) = planet.optional_member("bands")? {
        let field = generate::load_field(&path_of(&band_member, &root)?.display().to_string())
            .map_err(|err| format!("读条带场失败：{err}"))?;
        let cube = generate::field_cube(&field).map_err(|err| format!("条带立方图：{err}"))?;
        let member = baked.texture("gas_bands", cube, "texture.bands")?;
        surface = surface.with_texture("bands", TextureRef::new(7, member, Sampler::clamped()));
    }
    check_stage(&file.name, "planet", &surface)?;
    // 本体几何：默认是**图烘出来的网格**（`planet` / `moon` 那两张图的立方球）；
    // 配方写 `primitive = "icosphere"` 时改用**内建球**。
    // ⚠ 气态巨行星必须走这一支：它的可见面是**光球层**（一个球），不是有起伏的地形网格
    //   —— 挂上岩石那张位移网格，地形起伏会从次表面散射材质里透出来（实测：一版渲染图上
    //   那几块"海岸线"其实就是 `planet` 图的陆地，法线带着它 ⇒ 条带被采样歪了）。
    let geometry = match planet.primitive.as_deref() {
        None => Geometry::mesh(planet.member("mesh")?),
        Some("icosphere") => Geometry::primitive(
            "icosphere",
            BTreeMap::from([
                ("radius".to_string(), Value::Num(f64::from(radius))),
                (
                    "subdivisions".to_string(),
                    Value::Num(f64::from(planet.number_or("subdivisions", 64.0))),
                ),
            ]),
        ),
        Some(other) => {
            return Err(format!(
                "part '{}' 的 primitive '{other}' 不认识（今天只有 icosphere；\
                 不写这一栏就用 members 里那个网格）",
                planet.id
            ))
        }
    };
    objects.push(Object {
        id: "planet".to_string(),
        geometry,
        material: surface,
        transform: Transform::rotated(world),
        cast_shadow: true,
    });

    // ⚠ 物体的次序**就是文档的次序**，而透明物体（大气、云）都摆在原点 ⇒ 深度排序分不出
    // 先后，谁先画谁后画完全由这张表决定。次序照迁移前的渲染器摆：行星 → 大气 → 云。
    // 反过来的话，云壳薄的像素会差 1~2 个色阶（实测 33/614400 个像素、最大差 2）。
    if let Some(atmosphere) = atmosphere {
        let inner = atmosphere.number_or("inner", radius);
        if (inner - radius).abs() > 1e-3 {
            return Err(format!(
                "大气 part 的 'inner' 是 {inner}，而行星半径是 {radius}：大气壳的内半径就是行星半径，\
                 这两个对不上"
            ));
        }
        let outer_factor = atmosphere.number("outer")? as f32;
        let outer = radius * outer_factor;
        let density = atmosphere.number("density")? as f32 * planet.number_or("atmo", 1.0);
        let tint = atmosphere.triple("tint")?;
        let atmosphere_shader = atmosphere.shader_member()?;
        let params = material_params(
            atmosphere,
            &atmosphere_shader,
            &ATMOSPHERE_KEYS,
            BTreeMap::from([
                ("inner".to_string(), Value::Num(f64::from(radius))),
                ("outer".to_string(), Value::Num(f64::from(outer))),
                ("density".to_string(), Value::Num(f64::from(density))),
                (
                    "softness".to_string(),
                    Value::Num(atmosphere.number("softness")?),
                ),
                (
                    "tint".to_string(),
                    Value::Quad([tint[0], tint[1], tint[2], 1.0]),
                ),
            ]),
            &root,
        )?;
        let material = Material::new(atmosphere_shader).with_params(params);
        let mut material = material;
        material.alpha = AlphaMode::Add;
        check_stage(&file.name, "atmosphere", &material)?;
        objects.push(Object {
            id: "atmosphere".to_string(),
            geometry: Geometry::primitive(
                "icosphere",
                BTreeMap::from([
                    ("radius".to_string(), Value::Num(f64::from(outer))),
                    ("subdivisions".to_string(), Value::Num(64.0)),
                ]),
            ),
            material,
            // 大气壳是**球对称**的：迁移前它挂在根上（不带倾斜），这里也就给单位变换。
            transform: Transform::default(),
            cast_shadow: false,
        });
    }

    if let Some(clouds) = clouds {
        let shape = clouds.cloud_shape();
        let clouds_shader = clouds.shader_member()?;
        let ablate = vocab::ablate_code(clouds.text_or("ablate", "none"))?;
        let tint = match clouds.params.get("tint") {
            Some(_) => clouds.triple("tint")?,
            None => [1.0, 0.99, 0.97],
        };
        let computed = vocab::cloud_params(
            shape,
            ablate,
            tint,
            clouds.number("extinction")? as f32,
            cloud_inner,
            cloud_outer,
            world,
        );
        let params = material_params(clouds, &clouds_shader, &CLOUDS_KEYS, computed, &root)?;
        let mut material = Material::new(clouds_shader).with_params(params);
        material.alpha = AlphaMode::Premultiplied;
        // 云壳压一点深度：它整颗球都盖在行星上。
        material.depth_bias = -1.0;
        material = material.with_texture(
            "coverage",
            TextureRef::new(
                5,
                coverage_member
                    .clone()
                    .ok_or_else(|| "云 part 没有覆盖度成员".to_string())?,
                Sampler::clamped(),
            ),
        );
        check_stage(&file.name, "clouds", &material)?;
        // 几何：有代理 mesh 就用它（空区域在光栅阶段就被剔除），没有就是一个细分球壳。
        let geometry = match clouds.optional_member("proxy")? {
            Some(proxy) => Geometry::mesh(proxy),
            None => Geometry::primitive(
                "icosphere",
                BTreeMap::from([
                    ("radius".to_string(), Value::Num(f64::from(cloud_outer))),
                    ("subdivisions".to_string(), Value::Num(64.0)),
                ]),
            ),
        };
        objects.push(Object {
            id: "clouds".to_string(),
            geometry,
            material,
            transform: Transform::rotated(world),
            // 云壳**不投**阴影：它是一整颗球，进 shadow map 就是一颗球形硬影。
            cast_shadow: false,
        });
    }

    if rings > 0.0 {
        let inner = radius * 1.30;
        let outer = radius * rings.max(1.45);
        let mesh = baked.mesh(
            "ring_mesh",
            &generate::ring_mesh(inner, outer, RING_SEGMENTS),
            "mesh.ring",
        )?;
        let band = baked.texture(
            "ring_band",
            generate::ring_band(RING_BAND.0, RING_BAND.1),
            "texture.ring",
        )?;
        let ring_shader = ring_shader()?;
        // ⚠ `ambient`（2026-09-20 加）：环**受光也接受影**之后，"影里剩多少"是一栏观感旋钮。
        //   0.18 = 影里留一点星光/天光（参考图里行星投在环上的那道影是深灰，不是纯黑）。
        let ring_params = BTreeMap::from([
            ("tint".to_string(), Value::Quad([1.0, 1.0, 1.0, 1.0])),
            ("ambient".to_string(), Value::Num(0.18)),
        ]);
        validate_material("环", &ring_shader, &ring_params, &root)?;
        let mut material = Material::new(ring_shader).with_params(ring_params);
        material.alpha = AlphaMode::Blend;
        material.cull = CullMode::None;
        material = material.with_texture("color", TextureRef::new(1, band, Sampler::clamped()));
        check_stage(&file.name, "rings", &material)?;
        objects.push(Object {
            id: "rings".to_string(),
            geometry: Geometry::mesh(mesh),
            material,
            transform: Transform::rotated(world),
            cast_shadow: true,
        });
    }

    // ---- 卫星：天上**另外一颗小球**（`kind = "moon"`，2026-09-20 加）----
    //
    // ⚠ 它与 planet 的区别只有三条：① 不产环、不产云、不当光源；② 几何**只**走内建球
    //   （一颗小球不需要图烘网格）；③ 自己带 `position`（世界系里的偏移）—— 参考图上那颗
    //   凌日的卫星就是"球 + 一个位置"。
    //   ⚠ 材质那一侧与 planet **同一条口径**：参数按本 part shader 的契约判，声明了贴图
    //   而配方没给成员的格子就吃渲染器的兜底（`fallback_slots`）—— 气态巨行星那份材质
    //   在没有条带图时退化成"一颗均匀的气球"，正好是参考图上那颗土黄小球的样子。
    for part in file.parts.iter().filter(|part| part.kind == "moon") {
        let moon_shader = part.shader_member()?;
        let (geometry, transform, moon_rotation) = moon_body(part)?;
        // ⚠ `orientation` 与行星同口径：由 `spin × SYSTEM_TILT` 算出来（不是配方写的），
        //   所以这里要当**算好的那一栏**递进去，否则契约会判"shader 声明的 orientation 没给"。
        let moon_params = material_params(
            part,
            &moon_shader,
            &MOON_KEYS,
            BTreeMap::from([("orientation".to_string(), Value::Quad(moon_rotation))]),
            &root,
        )?;
        let material = Material::new(moon_shader).with_params(moon_params);
        check_stage(&file.name, &part.id, &material)?;
        objects.push(Object {
            id: part.id.clone(),
            geometry,
            material,
            transform,
            cast_shadow: true,
        });
    }

    // ---- 灯：那盏太阳（点光源，§60）----
    let position = match planet.params.get("light_position") {
        Some(_) => planet.triple("light_position")?,
        None => [-4.2, 1.15, 2.35],
    };
    let color = match planet.params.get("light_color") {
        Some(_) => planet.triple("light_color")?,
        None => [1.0, 1.0, 1.0],
    };
    let intensity = planet.number_or("light_intensity", 7.6e5);
    // 射程 = `|position| × 2.5`（迁移前是渲染器里的 `SUN_RANGE_FACTOR`）。
    let reach = planet.number_or("light_range", math::length(position) * SUN_RANGE_FACTOR);
    let mut sun = Light::point("sun", position, color, intensity as f32).with_range(reach as f32);
    sun.shadows = planet.number_or("shadows", 0.0) > 0.5;
    // ---- 灯：**配方里额外声明的那些**（`kind = "light"`）----
    // ⚠ 为什么需要：地球反照（行星反照到月球暗面那一层光）是**第二光源**，只有一盏灯时
    //   根本表达不出来。它们按配方顺序跟在太阳后面；**默认不投影**（省掉一整张 cube 影图，
    //   也不动帧图里 `shadow_cubes` 的数量）。
    // ⚠ 灯**没有材质** ⇒ 不走"参数对 shader 契约"那条路；结构键是 `vocab::LIGHT_KEYS`。
    let mut extra_lights: Vec<Light> = Vec::new();
    for part in &file.parts {
        if part.kind != "light" {
            continue;
        }
        let position = part.triple("position")?;
        let color = match part.params.get("color") {
            Some(_) => part.triple("color")?,
            None => [1.0, 1.0, 1.0],
        };
        let intensity = part.number_or("intensity", 1.0e4);
        let reach = part.number_or("range", math::length(position) * SUN_RANGE_FACTOR);
        let mut light = Light::point(&part.id, position, color, intensity as f32)
            .with_range(reach as f32);
        light.shadows = part.number_or("shadows", 0.0) > 0.5;
        extra_lights.push(light);
    }

    // ---- 相机：局部方向 → 世界系 ----
    let cameras: Vec<Camera> = match file.cameras.as_deref() {
        Some("review") | None => px_graph::cameras::review(),
        Some(other) => {
            return Err(format!(
                "不认识的相机表 '{other}'（现在只有 review）"
            ))
        }
    }
    .into_iter()
    .map(|camera| {
        let world_direction = math::rotate(
            math::quat_x(vocab::SYSTEM_TILT),
            camera.direction,
        );
        Camera::new(world_direction, camera.distance, &camera.tag)
    })
    .collect();

    // ---- 帧图（§128）：**渲染器的形状**烘进文档 ----
    let frame_name = file
        .frame
        .clone()
        .unwrap_or_else(|| crate::frame::DEFAULT_FRAME.to_string());
    // ⚠ 逃生门那条路**连帧图配方都不读**：它是"证明老产物还能逐字节复现"的仪器，
    //    不该因为帧图配方坏了就一起坏掉（那正是它要保的东西）。
    let framebaked = if with_graph {
        let frame = crate::frame::load(&frame_name)?;
        // 帧材质的参数写的是**来源**，这里把**内容**那一边的值给它 ——
        // 帧配方里一个内容值都不许写死（§133）。
        let sources = crate::frame::Sources {
            ambient: file.ambient,
            skybox_brightness: SKYBOX_BRIGHTNESS,
            // 投影的点光有几盏：今天这张场景表就一盏（`sun`，`shadows` 由 planet 那一格给）。
            shadow_lights: usize::from(sun.shadows),
        };
        crate::frame::build(&frame, &objects, &sources, true)?
    } else {
        crate::frame::Baked {
            resources: Vec::new(),
            passes: Vec::new(),
            materials: Vec::new(),
            material_instances: Vec::new(),
        }
    };

    let document = SceneSpec {
        schema: px_protocol::SCENE_SCHEMA,
        name: file.name.clone(),
        environment: Environment {
            ambient: file.ambient,
            skybox: Some(stars_member),
            skybox_brightness: SKYBOX_BRIGHTNESS,
        },
        cameras,
        expects: if clouds.is_some() {
            vec!["clouds".to_string()]
        } else {
            Vec::new()
        },
        resources: framebaked.resources,
        passes: framebaked.passes,
        lights: {
            let mut all = vec![sun];
            all.extend(extra_lights);
            all
        },
        objects,
        frame_materials: framebaked.materials,
        material_instances: framebaked.material_instances,
    };
    document.check()?;
    Ok(Compiled {
        name: file.name.clone(),
        document,
    })
}

/// **逐 stage 的 per-pass 对账**（运行期那一条路，配方适配器专用）。
///
/// ⚠ 这一支是**唯一**的运行期入口：配方是数据，`kind` / `shader` 是字符串 ⇒ 走到这里
/// "哪份内容"已经不在类型里。表与判据**仍然是类型级那两份**（`StageParams` 的实现 +
/// [`stage::check`]），这里只做分派（[`stage::runtime`]）。
///
/// ⚠ **无条件跑**：每份材质做完就查，没有开关、没有"先放过以后再说"的档。
/// 认不出的内容（自写 shader）这一档没有表可查 ⇒ 不算错，但那是**没有表**，不是"跳过检查"。
///
/// 判据用的是**产物那一档**（`stage_of_alpha`）—— 与 `frame::draws_of` 的 `opaque` /
/// `transparent` 两个 `select` 同一套谓词：两处各写一遍就是"同一件事两个答案"。
fn check_stage(scene: &str, id: &str, material: &Material) -> Result<(), String> {
    let Some(content) = stage::runtime::RuntimeContent::parse(&material.shader.node) else {
        // 认不出的内容（自写 shader 之类）：这一档没有表可查，不是错误。
        return Ok(());
    };
    let stage_name = stage::runtime::stage_of_alpha(material.alpha);
    stage::runtime::check(stage_name, content, &material.params)
        .map_err(|err| format!("场景 '{scene}' 的物体 '{id}'：{err}"))
}

/// 配方参数表 + 编译器算出来的那些 → 材质参数表，并在**烘图时**按契约校验一遍。
///
/// 三档，逐条都当场说清楚（§79 的 W1：原来「配方里能写什么」是一张写死的白名单，
/// 加一个 shader 参数就得改那张表 —— 也就是改 Rust）：
/// · 名字在 `structural` 里 ⇒ 编译器自己要用它（半径、色板、灯、消融档……），**不进**参数表；
/// · 名字在这份 shader 的参数表里 ⇒ 按它声明的类型透传（**这就是「加一个参数不用改 Rust」**）；
/// · 两边都不是 ⇒ 报错，并把两张表都列出来。
/// **part 的种类白名单**（2026-09-20 第 7 轮把 `moon` 加进来）。
///
/// ⚠ 抽成函数不是为了好看：它是"场景里能放什么"这件事**唯一的门**，
///   一个 part 种类加进来而这里没放行，报错必须**点名它认哪些**（否则用户只能猜）。
fn check_kind(part: &PartFile) -> Result<(), String> {
    const KINDS: [&str; 5] = ["planet", "clouds", "atmosphere", "moon", "light"];
    if KINDS.contains(&part.kind.as_str()) {
        return Ok(());
    }
    Err(format!(
        "part '{}' 的 kind '{}' 不认识；场景编译器认：{}",
        part.id,
        part.kind,
        KINDS.join(" / ")
    ))
}

/// 卫星 part 的**几何与变换**（`kind = "moon"`）。
///
/// ⚠ 抽出来是为了**能单独判**：`position` 进 `translation`、`radius` 进内建球、
///   `subdivisions` 有默认值、`spin` 与行星同口径 —— 这四件都不需要烘任何产物，
///   于是可以在一个 unit 测试里钉住（场景级那条判据要烘图，代价大得多）。
fn moon_body(part: &PartFile) -> Result<(Geometry, Transform, [f32; 4]), String> {
    // ⚠ 这两栏**不许走 `number_or`**（它给 f32）：几何参数在产物里是 f64（`Value::Num`），
    //   绕一趟 f32 会把配方里的 `0.093` 写成 `0.09300000220537186` —— 那是**静默改产物字节**。
    //   （测试 `a_moon_part_carries_its_position_and_radius_into_the_object` 抓的就是这一条。）
    let number = |key: &str, fallback: f64| -> Result<f64, String> {
        match part.params.get(key) {
            Some(_) => part.number(key),
            None => Ok(fallback),
        }
    };
    let radius = number("radius", 0.08)?;
    let subdivisions = number("subdivisions", 48.0)?;
    let rotation = math::orientation(part.number_or("spin", 0.0), vocab::SYSTEM_TILT);
    let geometry = Geometry::primitive(
        "icosphere",
        BTreeMap::from([
            ("radius".to_string(), Value::Num(f64::from(radius))),
            ("subdivisions".to_string(), Value::Num(subdivisions)),
        ]),
    );
    let position = match part.params.get("position") {
        Some(_) => part.triple("position")?,
        None => [0.0, 0.0, 0.0],
    };
    let transform = Transform {
        translation: position,
        rotation,
        scale: [1.0, 1.0, 1.0],
    };
    Ok((geometry, transform, rotation))
}

pub fn material_params(
    part: &PartFile,
    shader: &px_protocol::scene::Member,
    structural: &[&str],
    computed: BTreeMap<String, Value>,
    root: &Path,
) -> Result<BTreeMap<String, Value>, String> {
    let layout = schema_of(shader, root)?;
    merge_named(
        &format!("part '{}'（kind {}）", part.id, part.kind),
        &part.params,
        structural,
        &layout,
        computed,
    )
    .map_err(|err| format!("{err}\n  shader 成员：{}", shader.node))
}

/// 烘图时的最后一道：这份参数表**打得进这份契约吗**（缺参 / 多参 / 类型不符都在这里红）。
pub fn validate_material(
    label: &str,
    shader: &px_protocol::scene::Member,
    params: &BTreeMap<String, Value>,
    root: &Path,
) -> Result<(), String> {
    let layout = schema_of(shader, root)?;
    layout
        .pack(params)
        .map(|_| ())
        .map_err(|err| format!("{label} 的材质参数对不上 {} 的契约：{err}", shader.node))
}

/// 环的自写材质（原来是 Bevy 的 `StandardMaterial { unlit: true, blend, cull: none }`）。
/// 和 `shaders` 图里那三份同规矩：**include 闭包进键**（§17.1、§52.3）。
///
/// ⚠ 入口文本走**工作区根**拼绝对路径，不靠当前目录：`cargo run` 与 `cargo test`
/// 的工作目录是**两个**（调用目录 / 包目录）—— 相对路径会让同一份配方指向两个地方，
/// 而那一处的症状是"环的键变了"（读不到文件就换一个 shader）。
pub fn ring_shader() -> Result<px_protocol::scene::Member, String> {
    let path = px_graph::workspace_root().join("art/shaders/ring.wgsl");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
    let modules = px_shader::workspace_modules(&px_graph::workspace_root())?;
    let closure = px_shader::closure(&text, &modules);
    let (key, _path, _bytes) = px_graph::write_shader("ring", &text, &closure, &modules)
        .map_err(|err| format!("写环 shader 失败：{err}"))?;
    println!("环 shader {}｜{}", px_graph::hex_short(&key), closure.summary());
    Ok(px_protocol::scene::Member::new(
        "shaders",
        "ring",
        &px_graph::hex(&key),
    ))
}

/// 配方文件的位置（`art/scene/<名>.toml`）。
pub fn recipe_path(name: &str) -> PathBuf {
    px_graph::workspace_root()
        .join("art")
        .join("scene")
        .join(format!("{name}.toml"))
}

/// 读一份配方。
pub fn load(name: &str) -> Result<SceneFile, String> {
    let path = recipe_path(name);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
    toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))
}

/// 环的 shader 成员在文档里的角色名（`shaders::ring`）。
pub const RING_SHADER_NODE: &str = "ring";

/// 一个占位，让 `member_of` 在本模块里也有一条直路（`shaders` 图）。
pub fn shader_member(node: &str) -> Result<px_protocol::scene::Member, String> {
    member_of("shaders", node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::runtime::{RuntimeContent, RuntimeStage};
    use crate::stage::{Opaque, Surface};
    use px_protocol::scene::{AlphaMode, Member, Value};

    fn member(node: &str) -> Member {
        Member::new("shaders", node, &"0".repeat(64))
    }

    fn material(node: &str, pairs: &[(&str, Value)]) -> Material {
        let mut material = Material::new(member(node));
        material.params = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect();
        material
    }

    fn full_surface() -> Material {
        material(
            "surface",
            &[
                ("orientation", Value::Quad([0.0, 0.0, 0.0, 1.0])),
                ("emissive", Value::Quad([0.0, 0.0, 0.0, 0.0])),
                ("inner", Value::Num(1.01)),
                ("outer", Value::Num(1.06)),
                ("coverage", Value::Num(0.35)),
                ("shadow", Value::Num(0.0)),
                ("height", Value::Num(0.5)),
                ("gain", Value::Num(2.0)),
                // ⚠ 多出来的格**不算错**：一份材质往往同时被几档 pass 用。
                ("这一格是别的档要的", Value::Num(1.0)),
            ],
        )
    }

    /// 卫星 part：`position` 进 `translation`、`radius` 进内建球、`subdivisions` 有默认值。
    ///
    /// ⚠ 判的是**装配**（这条新路唯一容易写错的那几栏），不烘任何产物。
    #[test]
    fn a_moon_part_carries_its_position_and_radius_into_the_object() {
        let part: PartFile = toml::from_str(
            r#"
id = "satellite"
kind = "moon"
shader = "gasgiant"
params = { radius = 0.093, position = [-0.62, 0.30, 1.32] }
"#,
        )
        .expect("这份 part 应当解得开");
        let (geometry, transform, _) = moon_body(&part).expect("卫星的几何/变换应当装得出来");
        assert_eq!(transform.translation, [-0.62, 0.30, 1.32]);
        assert_eq!(transform.scale, [1.0, 1.0, 1.0]);
        match geometry {
            Geometry::Primitive { name, params } => {
                assert_eq!(name, "icosphere");
                assert_eq!(params.get("radius"), Some(&Value::Num(0.093)));
                // ⚠ 默认细分：不写这一栏也该有一颗球（不是 0 个三角形）。
                assert_eq!(params.get("subdivisions"), Some(&Value::Num(48.0)));
            }
            other => panic!("卫星的几何必须是内建球，实际是 {other:?}"),
        }
    }

    /// **种类白名单**：`moon` 放行；不认识的种类当场点名"我认哪些"。
    ///
    /// ⚠ 报错信息里必须出现 `moon` —— 新加一个 part 种类时，这条会替他回答用户
    ///   "为什么我写的 kind 不被认"（否则只能去读源码）。
    #[test]
    fn an_unknown_part_kind_is_named_against_the_list_it_knows() {
        let known: PartFile = toml::from_str(
            "id = \"satellite\"
kind = \"moon\"
shader = \"gasgiant\"
",
        )
        .expect("解得开");
        assert!(check_kind(&known).is_ok(), "`moon` 必须放行");
        let unknown: PartFile =
            toml::from_str("id = \"x\"
kind = \"asteroid\"
shader = \"surface\"
")
                .expect("解得开");
        let err = check_kind(&unknown).expect_err("不认识的 kind 必须被拒");
        assert!(err.contains("asteroid"), "要点名写错的那个：{err}");
        assert!(err.contains("moon"), "要点名它认哪些（含 moon）：{err}");
    }

    /// **对账真的会红**：surface 那一档要 8 格，只给 2 格 ⇒ 当场点名缺的是哪几个。
    ///
    /// ⚠ 这是**行为的**判据（不是"这个函数存在"）：它走的是配方编译时那条路
    /// （[`check_stage`] → `stage::runtime::check` → 类型级那份 `StageParams` 表）。
    #[test]
    fn the_stage_param_check_refuses_a_material_missing_per_pass_fields() {
        let thin = material(
            "surface",
            &[
                ("orientation", Value::Quad([0.0, 0.0, 0.0, 1.0])),
                ("emissive", Value::Quad([0.0, 0.0, 0.0, 0.0])),
            ],
        );
        let err = check_stage("夹具", "planet", &thin).expect_err("缺六格");
        for want in ["inner", "outer", "coverage", "shadow", "height", "gain"] {
            assert!(err.contains(want), "报错要点名 '{want}'：{err}");
        }
        assert!(err.contains("夹具") && err.contains("planet"), "{err}");
        assert!(err.contains("surface"), "要点名是哪份内容：{err}");
    }

    /// 给全了就不红（同一条路的**正对照** —— 少了它，上面那条可能只是"永远红"）。
    #[test]
    fn the_stage_param_check_accepts_a_complete_material() {
        check_stage("夹具", "planet", &full_surface()).expect("给全了就不该红");
    }

    /// **认不出的内容不查**（自写 shader 之类）：这一档没有表可查，不是错误。
    #[test]
    fn an_unknown_content_is_not_an_error() {
        check_stage("夹具", "custom", &material("custom", &[])).expect("没有表就不查");
        assert!(RuntimeContent::parse("custom").is_none());
    }

    /// 透明的物体走**透明**那一档：同样一份大气材质，在不透明那一档下没有表 ⇒ 不查；
    /// 在透明那一档下按大气那五格查。
    #[test]
    fn the_stage_comes_from_the_alpha_mode() {
        let mut atmosphere = material("atmosphere", &[("density", Value::Num(0.3))]);
        atmosphere.alpha = AlphaMode::Add;
        assert_eq!(
            crate::stage::runtime::stage_of_alpha(atmosphere.alpha).name(),
            RuntimeStage::Transparent.name()
        );
        let err = check_stage("夹具", "atmosphere", &atmosphere).expect_err("少四格");
        assert!(err.contains("softness"), "{err}");
    }

    /// 分派表与类型级那张表**说的是同一件事**：`opaque` + surface 这一对在两边都存在。
    #[test]
    fn the_runtime_dispatch_agrees_with_the_typed_table() {
        let typed = <Opaque as crate::stage::StageParams<Opaque, Surface>>::GIVEN;
        let via_runtime = crate::stage::runtime::check(
            RuntimeStage::Opaque,
            RuntimeContent::Surface,
            &BTreeMap::new(),
        )
        .expect_err("空参数必然缺格");
        for field in typed {
            assert!(
                via_runtime.contains(field.name),
                "运行期那条分派漏了 '{}'：{via_runtime}",
                field.name
            );
        }
    }
}
