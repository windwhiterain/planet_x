//! **配方**（`art/scene/*.toml`）的形状与它的语义组装。
//!
//! 配方文件的形状**没变**（还是 `[[parts]]` + `kind` + `members` + `params`）：它是艺术内容。
//! 变的是它编译成什么、以及**这份语义住在哪** —— 它从 `px_graphs/src/bin/scene.rs`
//! 搬进了这个库，于是 pcg 的图程序（以及将来的工具）用的是同一份语义。
//!
//! ⚠ 倾斜（[`SYSTEM_TILT`]）只在这里出现一次：文档里的方向与朝向**都是世界系**的。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_ops::generate::{self, Palette};
use px_protocol::art::Camera;
use px_protocol::scene::{
    AlphaMode, CullMode, Environment, Geometry, Light, Material, Object, Sampler, SceneSpec,
    TextureRef, Transform, Value,
};

use crate::baked::Baked;
use crate::contract::{merge_named, schema_of};
use crate::members::{member_of, path_of};
use crate::stage::Registration;
use crate::vocab::{
    self, ATMOSPHERE_KEYS, CLOUD_BASE, CLOUD_SHADOW_GAIN, CLOUD_SHADOW_HEIGHT, CLOUD_TOP,
    CLOUDS_KEYS, CloudShape, PLANET_KEYS, RING_BAND, RING_SEGMENTS, SKYBOX_BRIGHTNESS,
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
    /// **逐 stage 的材质格式对账**要不要在这份场景上做。
    ///
    /// ⚠ 缺省 `false`，而这是一个**必须说清的取舍**：格式表（[`crate::stage::Formats::content`]）
    /// 目前**晚于内容** —— 行星的 surface 材质在 shader 里还带着云影那几个格（`inner` /
    /// `outer` / `coverage` / `shadow` / `height` / `gain`），而表面那一档的格式**故意**不列它们
    /// （它们是云那一侧的量）。⇒ 今天给**既有配方**打开它，`orbit-soft` 那一族会当场红。
    ///
    /// 所以：**新内容请写 `formats = true`**（那时格式与材质一起设计，对账是有意义的）；
    /// 老配方保持 `false`，它们的产物**逐字节不动**（`art/anchor/hashes.txt` §三 那六格）。
    /// 等 surface 的云影参数腾出那个结构体之后，这一栏就该删掉 —— 那时它是恒真的。
    #[serde(default)]
    pub formats: bool,
    pub parts: Vec<PartFile>,
}

/// 配方里的一个 part。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartFile {
    pub id: String,
    pub kind: String,
    pub shader: String,
    /// 这个 part 的成员默认属于哪张图；跨图的成员写 `图名::节点名`。
    #[serde(default)]
    pub graph: Option<String>,
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
    let root = px_ops::cache_root();
    let planet = file
        .parts
        .iter()
        .find(|part| part.kind == "planet")
        .ok_or_else(|| "场景里没有 kind planet 的 part：主体（height / mesh / palette / …）全在它身上".to_string())?;
    let clouds = file.parts.iter().find(|part| part.kind == "clouds");
    let atmosphere = file.parts.iter().find(|part| part.kind == "atmosphere");
    for part in &file.parts {
        if !["planet", "clouds", "atmosphere"].contains(&part.kind.as_str()) {
            return Err(format!(
                "part '{}' 的 kind '{}' 不认识；场景编译器认：planet / clouds / atmosphere",
                part.id, part.kind
            ));
        }
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
    let height_member = planet.member("height")?;
    let height_path = path_of(&height_member, &root)?;
    let height = generate::load_field(&height_path.display().to_string())
        .map_err(|err| format!("读场 {} 失败：{err}", height_path.display()))?;
    let (color, glow, audit) = generate::surface_color(&height, palette, sea_level);
    println!("{audit}");
    let color_member = baked.texture("surface_color", color, "texture.palette")?;
    let glow_member = match glow {
        Some(glow) => Some(baked.texture("surface_glow", glow, "texture.palette")?),
        None => None,
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
    let mut registrations: Vec<Registration> = Vec::new();

    // 行星本体：走自写的 surface 材质。云影那几个参数与云材质同一口径。
    let surface_shader = planet.shader_member()?;
    let cloud_shadow = if coverage_member.is_some() {
        planet.number_or("cloud_shadow", 0.0)
    } else {
        0.0
    };
    let has_glow = glow_member.is_some();
    // 行星的材质参数**全是算出来的**（云影那几个量必须与云材质同口径）⇒ `computed` 就是全部；
    // 但配方里仍然可以按名字**补**这份 shader 声明过的其它参数（§80 第 2 步的透传）。
    let surface_params = material_params(
        planet,
        &surface_shader,
        &PLANET_KEYS,
        BTreeMap::from([
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
        ]),
        &root,
    )?;
    let mut surface = Material::new(surface_shader).with_params(surface_params);
    surface = surface.with_texture(
        "albedo",
        TextureRef::new(1, color_member.clone(), Sampler::repeat()),
    );
    if let Some(glow) = &glow_member {
        surface = surface.with_texture("glow", TextureRef::new(3, glow.clone(), Sampler::repeat()));
    }
    if let Some(coverage) = &coverage_member {
        surface = surface.with_texture(
            "coverage",
            TextureRef::new(5, coverage.clone(), Sampler::clamped()),
        );
    }
    registrations.push(Registration::single(&surface));
    objects.push(Object {
        id: "planet".to_string(),
        geometry: Geometry::mesh(planet.member("mesh")?),
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
        registrations.push(Registration::single(&material));
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
        registrations.push(Registration::single(&material));
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
        let ring_params = BTreeMap::from([("tint".to_string(), Value::Quad([1.0, 1.0, 1.0, 1.0]))]);
        validate_material("环", &ring_shader, &ring_params, &root)?;
        let mut material = Material::new(ring_shader).with_params(ring_params);
        material.alpha = AlphaMode::Blend;
        material.cull = CullMode::None;
        material = material.with_texture("color", TextureRef::new(1, band, Sampler::clamped()));
        registrations.push(Registration::single(&material));
        objects.push(Object {
            id: "rings".to_string(),
            geometry: Geometry::mesh(mesh),
            material,
            transform: Transform::rotated(world),
            cast_shadow: true,
        });
    }

    // ---- stage 的登记与**格式对账**（`px-scene` 的高层语义那一半）----
    //
    // 一个物体可以注册多个 stage 的材质；寻常那一路是 `Registration::single`（一份材质，
    // 几档 pass 共用）。这里对每一档问一遍"它要的格式满足了吗" —— 不满足就**烘图时**红。
    // ⚠ 只有 `formats = true` 的配方走这一道（见 `SceneFile::formats` 里那段取舍）。
    if file.formats {
        check_registrations(&file.name, &objects, &registrations)?;
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

    // ---- 相机：局部方向 → 世界系 ----
    let cameras: Vec<Camera> = match file.cameras.as_deref() {
        Some("review") | None => px_ops::cameras::review(),
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
        lights: vec![sun],
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

/// **逐 stage 的格式对账**：每个物体登记的那些 stage 材质，必须满足那一档的格式。
///
/// ⚠ 今天是**声明式**的：内容那一侧按 `kind` 给出每一档要的格式（[`stage::content_formats`]）。
/// "哪一档画哪些物体"由帧图的 `select` 决定（`opaque` / `transparent` / `shadow_casters` /
/// `skybox`），而这里用的判据与 [`crate::frame::draws_of`] **同一套谓词** —— 两处各写一遍
/// 就是"同一件事两个答案"，漂开的那天变成"自己烘的自己不认"。
fn check_registrations(
    scene: &str,
    objects: &[Object],
    registrations: &[Registration],
) -> Result<(), String> {
    let table = stage::StageParamsTable::content();
    for (object, registration) in objects.iter().zip(registrations) {
        for stage_name in stage::stages_of(&object.material, object.cast_shadow) {
            // 登记的那份材质说"这一档用哪份内容" ⇒ per-pass 表按 **(stage, 那份材质)** 查。
            let Some(material) = registration.material_for(stage_name) else {
                continue;
            };
            let Some(given) = table.of(stage_name, &material.of) else {
                continue;
            };
            if let Some(params) = registration.params_for(stage_name) {
                stage::check(stage_name, &material.of, given, params)
                    .map_err(|err| format!("场景 '{scene}' 的物体 '{}'：{err}", object.id))?;
            }
        }
    }
    Ok(())
}

/// 配方参数表 + 编译器算出来的那些 → 材质参数表，并在**烘图时**按契约校验一遍。
///
/// 三档，逐条都当场说清楚（§79 的 W1：原来「配方里能写什么」是一张写死的白名单，
/// 加一个 shader 参数就得改那张表 —— 也就是改 Rust）：
/// · 名字在 `structural` 里 ⇒ 编译器自己要用它（半径、色板、灯、消融档……），**不进**参数表；
/// · 名字在这份 shader 的参数表里 ⇒ 按它声明的类型透传（**这就是「加一个参数不用改 Rust」**）；
/// · 两边都不是 ⇒ 报错，并把两张表都列出来。
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
    let path = px_ops::workspace_root().join("art/shaders/ring.wgsl");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
    let modules = px_shader::workspace_modules(&px_ops::workspace_root())?;
    let closure = px_shader::closure(&text, &modules);
    let (key, _path, _bytes) = px_ops::write_shader("ring", &text, &closure, &modules)
        .map_err(|err| format!("写环 shader 失败：{err}"))?;
    println!("环 shader {}｜{}", px_ops::hex_short(&key), closure.summary());
    Ok(px_protocol::scene::Member::new(
        "shaders",
        "ring",
        &px_ops::hex(&key),
    ))
}

/// 配方文件的位置（`art/scene/<名>.toml`）。
pub fn recipe_path(name: &str) -> PathBuf {
    px_ops::workspace_root()
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
