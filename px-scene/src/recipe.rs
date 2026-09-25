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
    CLOUDS_KEYS, CloudShape, DEFAULT_SHADOW_DENSITY, MOON_KEYS, PLANET_KEYS, RING_BAND,
    RING_SEGMENTS, SKYBOX_BRIGHTNESS, STARS_FACE, SUN_RANGE_FACTOR,
};
use crate::{math, stage};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneFile {
    pub name: String,
    #[serde(default)]
    pub ambient: f32,
    #[serde(default)]
    pub cameras: Option<String>,
    #[serde(default)]
    pub frame: Option<String>,
    #[serde(default)]
    pub skybox: Option<String>,
    #[serde(default)]
    pub skybox_graph: Option<String>,
    pub parts: Vec<PartFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartFile {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub shader: String,
    #[serde(default)]
    pub graph: Option<String>,
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

    pub fn optional_member(
        &self,
        role: &str,
    ) -> Result<Option<px_protocol::scene::Member>, String> {
        match self.members.get(role) {
            None => Ok(None),
            Some(reference) => {
                let what = format!("part '{}' 的成员 '{role}'", self.id);
                crate::members::reference(&what, reference, self.graph.as_deref()).map(Some)
            }
        }
    }

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

pub struct Compiled {
    pub document: SceneSpec,
    pub name: String,
}

pub fn compile(file: &SceneFile, baked: &mut Baked, with_graph: bool) -> Result<Compiled, String> {
    let root = px_graph::cache_root();
    let planet = file
        .parts
        .iter()
        .find(|part| part.kind == "planet")
        .ok_or_else(|| {
            "场景里没有 kind planet 的 part：主体（height / mesh / palette / …）全在它身上"
                .to_string()
        })?;
    let clouds = file.parts.iter().find(|part| part.kind == "clouds");
    let atmosphere = file.parts.iter().find(|part| part.kind == "atmosphere");
    for part in &file.parts {
        check_kind(part)?;
    }

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
    let _displace = planet.number_or("displace", 0.075);
    let rings = planet.number_or("rings", 0.0);
    let world = math::orientation(spin, vocab::SYSTEM_TILT);

    let surface_shader = planet.shader_member()?;
    let surface_layout = crate::contract::schema_of(&surface_shader, &root)?;
    let wants_texture = |binding: u32| {
        surface_layout
            .textures
            .iter()
            .any(|slot| slot.binding == binding)
    };

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
    let stars_member = match (&file.skybox, &file.skybox_graph) {
        (Some(reference), graph) => {
            crate::members::reference("场景的 skybox", reference, graph.as_deref())?
        }
        (None, _) => baked.texture("stars", generate::stars(STARS_FACE), "texture.stars")?,
    };

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

    let mut objects: Vec<Object> = Vec::new();

    let cloud_shadow = if coverage_member.is_some() {
        planet.number_or("cloud_shadow", 0.0)
    } else {
        0.0
    };
    let has_glow = glow_member.is_some();
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
        ("shadow".to_string(), Value::Num(f64::from(cloud_shadow))),
        (
            "height".to_string(),
            Value::Num(planet.number_or("shadow_height", CLOUD_SHADOW_HEIGHT) as f64),
        ),
        ("gain".to_string(), Value::Num(f64::from(CLOUD_SHADOW_GAIN))),
    ]);
    computed.retain(|name, _| surface_layout.param(name).is_some());
    let surface_params = material_params(planet, &surface_shader, &PLANET_KEYS, computed, &root)?;
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
            ));
        }
    };
    let shape_radius = measure_radius(&geometry, &root)?;
    let geometry = geometry.with_bounding_radius(shape_radius);
    objects.push(Object {
        id: "planet".to_string(),
        geometry,
        material: surface,
        transform: Transform::rotated(world),
        cast_shadow: true,
        shadow_density: shadow_density_of(planet)?,
    });

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
            )
            .with_bounding_radius(outer),
            material,
            transform: Transform::default(),
            cast_shadow: false,
            shadow_density: 0.0,
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
        let cloud_radius = measure_radius(&geometry, &root)?;
        let geometry = geometry.with_bounding_radius(cloud_radius);
        objects.push(Object {
            id: "clouds".to_string(),
            geometry,
            material,
            transform: Transform::rotated(world),
            cast_shadow: false,
            shadow_density: 0.0,
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
            geometry: Geometry::mesh(mesh).with_bounding_radius(outer),
            material,
            transform: Transform::rotated(world),
            cast_shadow: true,
            shadow_density: shadow_density_of(planet)?,
        });
    }

    for part in file.parts.iter().filter(|part| part.kind == "moon") {
        let moon_shader = part.shader_member()?;
        let (geometry, transform, moon_rotation) = moon_body(part)?;
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
            shadow_density: shadow_density_of(part)?,
        });
    }

    let position = match planet.params.get("light_position") {
        Some(_) => planet.triple("light_position")?,
        None => [-4.2, 1.15, 2.35],
    };
    let color = match planet.params.get("light_color") {
        Some(_) => planet.triple("light_color")?,
        None => [1.0, 1.0, 1.0],
    };
    let intensity = planet.number_or("light_intensity", 7.6e5);
    let reach = planet.number_or("light_range", math::length(position) * SUN_RANGE_FACTOR);
    let mut sun = Light::point("sun", position, color, intensity as f32).with_range(reach as f32);
    sun.shadows = planet.number_or("shadows", 0.0) > 0.5;
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
        let mut light =
            Light::point(&part.id, position, color, intensity as f32).with_range(reach as f32);
        light.shadows = part.number_or("shadows", 0.0) > 0.5;
        extra_lights.push(light);
    }

    let cameras: Vec<Camera> = match file.cameras.as_deref() {
        Some("review") | None => crate::cameras::review(),
        Some(other) => return Err(format!("不认识的相机表 '{other}'（现在只有 review）")),
    }
    .into_iter()
    .map(|camera| {
        let world_direction = math::rotate(math::quat_x(vocab::SYSTEM_TILT), camera.direction);
        Camera::new(world_direction, camera.distance, &camera.tag)
    })
    .collect();

    let lights: Vec<px_protocol::scene::Light> = {
        let mut all = vec![sun];
        all.extend(extra_lights);
        all
    };

    let frame_name = file
        .frame
        .clone()
        .unwrap_or_else(|| crate::frame::DEFAULT_FRAME.to_string());
    let framebaked = if with_graph {
        let frame = crate::frame::load(&frame_name)?;
        let sources = crate::frame::Sources {
            ambient: file.ambient,
            skybox_brightness: SKYBOX_BRIGHTNESS,
            shadow_lights: lights.iter().filter(|light| light.shadows).count(),
        };
        crate::frame::build(&frame, &objects, &sources, &lights, true)?
    } else {
        crate::frame::Baked {
            resources: Vec::new(),
            passes: Vec::new(),
            materials: Vec::new(),
            material_instances: Vec::new(),
            shadow: None,
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
        lights,
        shadow: framebaked.shadow,
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

fn check_stage(scene: &str, id: &str, material: &Material) -> Result<(), String> {
    let Some(content) = stage::runtime::RuntimeContent::parse(&material.shader.node) else {
        return Ok(());
    };
    let stage_name = stage::runtime::stage_of_alpha(material.alpha);
    stage::runtime::check(stage_name, content, &material.params)
        .map_err(|err| format!("场景 '{scene}' 的物体 '{id}'：{err}"))
}

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

fn moon_body(part: &PartFile) -> Result<(Geometry, Transform, [f32; 4]), String> {
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
    )
    .with_bounding_radius(radius as f32);
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

fn primitive_radius(geometry: &Geometry) -> Option<f32> {
    let Geometry::Primitive { name, params, .. } = geometry else {
        return None;
    };
    if name != "icosphere" {
        return None;
    }
    match params.get("radius") {
        Some(Value::Num(value)) => Some(*value as f32),
        _ => None,
    }
}

fn mesh_radius(member: &px_protocol::scene::Member, root: &Path) -> Result<f32, String> {
    let path = member
        .resolve(root)
        .map_err(|err| format!("网格成员 {member} 的路径：{err}"))?;
    let bytes = std::fs::read(&path).map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let frames = px_protocol::stream::read_stream(&mut bytes.as_slice())
        .map_err(|err| format!("解 {} 的流：{err}", path.display()))?;
    let blobs: Vec<&px_protocol::wire::Blob> = frames
        .iter()
        .filter_map(|frame| match frame {
            px_protocol::stream::Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .collect();
    let mesh = px_protocol::art::MeshData::from_blobs(&blobs)
        .map_err(|err| format!("解 {} 的网格载荷：{err}", path.display()))?;
    let mut radius = 0.0_f32;
    for point in mesh.positions.chunks_exact(3) {
        let length = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
        radius = radius.max(length);
    }
    if radius <= 0.0 || !radius.is_finite() {
        return Err(format!(
            "网格产物 {} 的包围球半径量出来是 {radius}：顶点都在原点或者数值不合法",
            path.display()
        ));
    }
    Ok(radius)
}

fn measure_radius(geometry: &Geometry, root: &Path) -> Result<f32, String> {
    if let Some(radius) = primitive_radius(geometry) {
        return Ok(radius);
    }
    match geometry.member() {
        Some(member) => mesh_radius(member, root),
        None => Err("这份几何既不是内建图元、也没有网格成员：量不出包围球".to_string()),
    }
}

fn shadow_density_of(part: &PartFile) -> Result<f32, String> {
    if part.number_or("shadows", 0.0) <= 0.5 {
        return Ok(0.0);
    }
    Ok(part.number_or("shadow_density", DEFAULT_SHADOW_DENSITY))
}

pub fn ring_shader() -> Result<px_protocol::scene::Member, String> {
    let path = px_graph::workspace_root().join("art/shaders/ring.wgsl");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
    let modules = px_shader::workspace_modules(&px_graph::workspace_root())?;
    let closure = px_shader::closure(&text, &modules);
    let (key, _path, _bytes) = px_graph::write_shader("ring", &text, &closure, &modules)
        .map_err(|err| format!("写环 shader 失败：{err}"))?;
    println!(
        "环 shader {}｜{}",
        px_graph::hex_short(&key),
        closure.summary()
    );
    Ok(px_protocol::scene::Member::new(
        "shaders",
        "ring",
        &px_graph::hex(&key),
    ))
}

pub fn recipe_path(name: &str) -> PathBuf {
    px_graph::workspace_root()
        .join("art")
        .join("scene")
        .join(format!("{name}.toml"))
}

pub fn load(name: &str) -> Result<SceneFile, String> {
    let path = recipe_path(name);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
    toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))
}

pub const RING_SHADER_NODE: &str = "ring";

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
                ("这一格是别的档要的", Value::Num(1.0)),
            ],
        )
    }

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
            Geometry::Primitive { name, params, .. } => {
                assert_eq!(name, "icosphere");
                assert_eq!(params.get("radius"), Some(&Value::Num(0.093)));
                assert_eq!(params.get("subdivisions"), Some(&Value::Num(48.0)));
            }
            other => panic!("卫星的几何必须是内建球，实际是 {other:?}"),
        }
    }

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
        let unknown: PartFile = toml::from_str(
            "id = \"x\"
kind = \"asteroid\"
shader = \"surface\"
",
        )
        .expect("解得开");
        let err = check_kind(&unknown).expect_err("不认识的 kind 必须被拒");
        assert!(err.contains("asteroid"), "要点名写错的那个：{err}");
        assert!(err.contains("moon"), "要点名它认哪些（含 moon）：{err}");
    }

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

    #[test]
    fn the_stage_param_check_accepts_a_complete_material() {
        check_stage("夹具", "planet", &full_surface()).expect("给全了就不该红");
    }

    #[test]
    fn an_unknown_content_is_not_an_error() {
        check_stage("夹具", "custom", &material("custom", &[])).expect("没有表就不查");
        assert!(RuntimeContent::parse("custom").is_none());
    }

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
