use std::collections::BTreeMap;
use std::path::PathBuf;

use px_protocol::material::ParamKind;
use px_protocol::scene::{
    AlphaMode, DrawSpec, FrameMaterial, MaterialInstance, Member, Object, PassCubeFace,
    PassResource, PassSpec, SceneSpec, Value,
};
use serde::Deserialize;

pub const DEFAULT_FRAME: &str = "default";

pub fn recipe_path(name: &str) -> PathBuf {
    px_graph::workspace_root()
        .join("art")
        .join("frame")
        .join(format!("{name}.toml"))
}

const SELECTS: [&str; 5] = ["opaque", "transparent", "shadow_casters", "skybox", "none"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameFile {
    pub resources: Vec<ResourceFile>,
    #[serde(default)]
    pub before: Vec<EntryFile>,
    #[serde(default)]
    pub after: Vec<EntryFile>,
    #[serde(default)]
    pub chain_color: Vec<String>,
    #[serde(default)]
    pub materials: Vec<MaterialFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialFile {
    pub name: String,
    pub shader: String,
    pub entry: String,
    #[serde(default)]
    pub params: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Copy)]
pub struct Sources {
    pub ambient: f32,
    pub skybox_brightness: f32,
    pub shadow_lights: usize,
}

pub const SOURCES: [&str; 2] = ["environment.ambient", "environment.skybox_brightness"];

pub const CUBE_FACES: u32 = 6;
pub const FACE_NAMES: [&str; 6] = ["+x", "-x", "+y", "-y", "-z", "+z"];

fn source_of(name: &str, sources: &Sources) -> Option<(Value, ParamKind)> {
    match name {
        "environment.ambient" => Some((Value::Num(f64::from(sources.ambient)), ParamKind::F32)),
        "environment.skybox_brightness" => Some((
            Value::Num(f64::from(sources.skybox_brightness)),
            ParamKind::F32,
        )),
        _ => None,
    }
}

fn toml_of(value: &Value) -> toml::Value {
    match value {
        Value::Num(number) => toml::Value::Float(*number),
        Value::Text(text) => toml::Value::String(text.clone()),
        Value::Triple(items) => toml::Value::Array(
            items
                .iter()
                .map(|v| toml::Value::Float(f64::from(*v)))
                .collect(),
        ),
        Value::Quad(items) => toml::Value::Array(
            items
                .iter()
                .map(|v| toml::Value::Float(f64::from(*v)))
                .collect(),
        ),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceFile {
    pub name: String,
    pub format: String,
    pub size: String,
    #[serde(default)]
    pub layers: Option<String>,
    #[serde(default)]
    pub usage: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryFile {
    pub label: String,
    pub kind: String,
    #[serde(default)]
    pub select: Option<String>,
    #[serde(default)]
    pub vertex_shader: Option<String>,
    #[serde(default)]
    pub vertex_entry: String,
    #[serde(default)]
    pub fragment_shader: Option<String>,
    #[serde(default)]
    pub entry: String,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub depth_target: Option<String>,
    #[serde(default, alias = "cube_faces")]
    pub shadow_faces: Option<u32>,
    pub render: String,
    #[serde(default)]
    pub params: BTreeMap<String, toml::Value>,
}

impl EntryFile {
    fn select(&self) -> &str {
        self.select.as_deref().unwrap_or("none")
    }
}

pub fn load(name: &str) -> Result<FrameFile, String> {
    let path = recipe_path(name);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不了帧图配方 {}：{err}", path.display()))?;
    let frame: FrameFile =
        toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))?;
    frame.check()?;
    Ok(frame)
}

impl FrameFile {
    fn check(&self) -> Result<(), String> {
        if self.resources.is_empty() {
            return Err("帧图一个 resources 都没有：中间目标画到哪去？".to_string());
        }
        if self.chain_color.len() != 2 {
            return Err(format!(
                "帧图的 chain_color 有 {} 个：内容链的乒乓对要**恰好两个**（帧图的绘制段写一个，\
                 链的第一笔写另一个）",
                self.chain_color.len()
            ));
        }
        for name in &self.chain_color {
            if !self.resources.iter().any(|r| &r.name == name) {
                return Err(format!(
                    "chain_color 里的 '{name}' 不在 resources 里（声明了的：{}）",
                    self.resources
                        .iter()
                        .map(|r| r.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ));
            }
        }
        for (index, entry) in self.before.iter().chain(self.after.iter()).enumerate() {
            let at = format!("帧图第 {index} 条 '{}'", entry.label);
            if !SELECTS.contains(&entry.select()) {
                return Err(format!(
                    "{at} 的 select 是 '{}'：认 {}",
                    entry.select(),
                    SELECTS.join(" / ")
                ));
            }
            match (&entry.vertex_shader, &entry.fragment_shader) {
                (None, None) if entry.kind == "copy" => {}
                (_, _) if entry.kind == "copy" => {
                    return Err(format!(
                        "{at} 的 kind 是 copy：拷贝不画东西，`vertex_shader` / `vertex_entry` / \
                         `fragment_shader` / `entry` 四栏都该是空的\
                         （顶点阶段是几何 pass 那一栏，片元成员是全屏 pass 那一栏）"
                    ));
                }
                (Some(_), Some(_)) => {
                    return Err(format!(
                        "{at} 同时给了顶点阶段与片元成员：几何 pass 只给顶点阶段\
                         （片元阶段属于材质），全屏 pass 只给片元成员"
                    ));
                }
                (None, None) if entry.kind == "fullscreen" => {
                    return Err(format!("{at} 是 fullscreen，却没给 fragment_shader"));
                }
                (None, None) => {
                    return Err(format!(
                        "{at} 是几何 pass，却没给 vertex_shader（WGSL 文件）"
                    ));
                }
                (Some(_), None) if entry.vertex_entry.trim().is_empty() => {
                    return Err(format!("{at} 给了顶点阶段却没给 vertex_entry"));
                }
                _ => {}
            }
        }
        if let Some(blit) = self.after.iter().find(|entry| entry.kind == "fullscreen") {
            let read = blit.reads.first().map(String::as_str).unwrap_or("");
            if !self.chain_color.iter().any(|name| name == read) {
                return Err(format!(
                    "帧图最后那条全屏 pass '{}' 读的是 '{read}'，它不在 chain_color（{}）里：\
                     内容链接完要读链尾那一个",
                    blit.label,
                    self.chain_color.join(" / ")
                ));
            }
        }
        let mut names: Vec<&str> = Vec::new();
        for material in &self.materials {
            let at = format!("帧图材质 '{}'", material.name);
            if material.name.trim().is_empty() {
                return Err(
                    "帧图有一份 `[[materials]]` 没给 name：draws 是按名字引用它的".to_string(),
                );
            }
            if names.contains(&material.name.as_str()) {
                return Err(format!(
                    "帧图里材质名重了：'{}'（两份材质同名 ⇒ 宿主只能猜一个）",
                    material.name
                ));
            }
            if material.shader.trim().is_empty() {
                return Err(format!("{at} 没给 shader（WGSL 文件路径）"));
            }
            if material.entry.trim().is_empty() {
                return Err(format!("{at} 没给 entry（@fragment 那个函数叫什么）"));
            }
            let full = px_graph::workspace_root().join(&material.shader);
            if !full.is_file() {
                return Err(format!(
                    "{at} 的 shader '{}' 不是一份文件（{}）：它会被**内联**进产物，\
                     所以必须读得到",
                    material.shader,
                    full.display()
                ));
            }
            names.push(&material.name);
        }
        Ok(())
    }
}

pub fn draws_of(objects: &[Object], select: &str) -> Vec<DrawSpec> {
    let draw = |object: &Object| DrawSpec {
        geometry: object.id.clone(),
        material: object.id.clone(),
    };
    match select {
        "opaque" => objects
            .iter()
            .filter(|object| object.material.alpha == AlphaMode::Opaque)
            .map(draw)
            .collect(),
        "transparent" => {
            let mut picked: Vec<&Object> = objects
                .iter()
                .rev()
                .filter(|object| object.material.alpha != AlphaMode::Opaque)
                .collect();
            picked.sort_by(|a, b| a.material.depth_bias.total_cmp(&b.material.depth_bias));
            picked.into_iter().map(draw).collect()
        }
        "shadow_casters" => objects
            .iter()
            .filter(|object| object.cast_shadow)
            .map(draw)
            .collect(),
        "skybox" => vec![DrawSpec {
            geometry: "skybox".to_string(),
            material: "skybox".to_string(),
        }],
        _ => Vec::new(),
    }
}

pub struct Baked {
    pub resources: Vec<PassResource>,
    pub passes: Vec<PassSpec>,
    pub materials: Vec<FrameMaterial>,
    pub material_instances: Vec<MaterialInstance>,
    pub shadow: Option<px_protocol::scene::ShadowPlan>,
}

pub fn build(
    frame: &FrameFile,
    objects: &[Object],
    sources: &Sources,
    lights: &[px_protocol::scene::Light],
    with_graph: bool,
) -> Result<Baked, String> {
    if !with_graph {
        return Ok(Baked {
            resources: Vec::new(),
            passes: Vec::new(),
            materials: Vec::new(),
            material_instances: Vec::new(),
            shadow: None,
        });
    }
    let allocation = shadow_allocation(lights, objects)?;
    let mut resources: Vec<PassResource> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for resource in &frame.resources {
        let Some(source) = &resource.layers else {
            resources.push(PassResource {
                name: resource.name.clone(),
                format: resource.format.clone(),
                size: resource.size.clone(),
                layers: 1,
                usage: resource.usage.clone(),
            });
            continue;
        };
        let layers = match source.as_str() {
            "shadow_faces" => sources.shadow_lights * CUBE_FACES as usize,
            other => {
                return Err(format!(
                    "帧图资源 '{}' 的 layers 来源是 '{other}'：这一版只认 'shadow_faces'\
                     （每盏投影的点光一个 cube，每面一层）",
                    resource.name
                ));
            }
        };
        if layers == 0 {
            dropped.push(resource.name.clone());
            continue;
        }
        let mut size = resource.size.clone();
        if source == "shadow_faces" {
            let allocation = allocation.as_ref().ok_or_else(|| {
                format!(
                    "帧图资源 '{}' 要按分配定尺寸，而这一帧一盏投影灯都没有",
                    resource.name
                )
            })?;
            let level = crate::vshadow::SHADOW_ATLAS_RESOURCES
                .iter()
                .position(|name| *name == resource.name)
                .ok_or_else(|| {
                    format!(
                        "帧图资源 '{}' 的 layers 是 'shadow_faces'，但它不在那几条影子 atlas 的\
                         名字里（{}）—— 名字不对就认不出这是哪一级的 atlas",
                        resource.name,
                        crate::vshadow::SHADOW_ATLAS_RESOURCES.join(" / ")
                    )
                })?;
            let side = allocation.atlas_sides[level];
            size = format!("{}x{}", side, side);
            let declared = parse_fixed_size(&resource.size)?;
            if side > declared.0 || side > declared.1 {
                return Err(format!(
                    "虚拟影图的级 {} atlas 要 {}×{}，而帧图资源 '{}' 声明的是 {}：\
                     把那一栏调大，或者把这一帧的 shadow_density 调小。\
                     这一版**不降精度** —— 降了之后画面照样出得来，只是影比要求糊",
                    level, side, side, resource.name, resource.size
                ));
            }
        }
        resources.push(PassResource {
            name: resource.name.clone(),
            format: resource.format.clone(),
            size,
            layers: layers as u32,
            usage: resource.usage.clone(),
        });
    }
    if !dropped.is_empty() {
        println!(
            "⚠ 这一帧没有投影的点光（{} 盏）⇒ 不烘这些资源：{} —— 也不烘写它们的那几条 pass\
             （oracle 那边同样不渲染影子图）",
            sources.shadow_lights,
            dropped.join(" / ")
        );
    }

    let mut materials = Vec::with_capacity(frame.materials.len());
    if !frame.materials.is_empty() {
        let modules = px_shader::workspace_modules(&px_graph::workspace_root())?;
        for material in &frame.materials {
            materials.push(bake_material(material, sources, &modules)?);
        }
    }

    let mut passes: Vec<PassSpec> = Vec::new();
    let material_instances: Vec<MaterialInstance> = Vec::new();
    for entry in frame.before.iter().chain(frame.after.iter()) {
        let at = format!("帧图 pass '{}'", entry.label);
        if let Some(found) = [entry.depth_target.as_ref()]
            .into_iter()
            .flatten()
            .chain(entry.reads.iter())
            .chain(entry.writes.iter())
            .find(|name| dropped.contains(name))
        {
            println!(
                "⚠ 帧图 pass '{}' 用到没烘出来的 '{}' ⇒ 这一条也不烘",
                entry.label, found
            );
            continue;
        }
        let (vertex_shader, vertex_entry) = match &entry.vertex_shader {
            Some(path) => {
                let full = px_graph::workspace_root().join(path);
                let text = std::fs::read_to_string(&full)
                    .map_err(|err| format!("{at} 读不了顶点阶段 {}：{err}", full.display()))?;
                (text, entry.vertex_entry.clone())
            }
            None => (String::new(), String::new()),
        };
        let mut params: BTreeMap<String, px_protocol::scene::Value> = BTreeMap::new();
        let shader = match &entry.fragment_shader {
            Some(node) => {
                let key = px_graph::manifest_key_of("shaders", node).map_err(|err| {
                    format!("{at} 的片元 shader '{node}'：{err}（先跑 --bin shaders 烘）")
                })?;
                let member = Member::new("shaders", node, &key);
                let layout = crate::contract::schema_of(&member, &px_graph::cache_root())
                    .map_err(|err| format!("{at}：{err}"))?;
                params = crate::contract::merge_named(
                    &format!("帧图 pass '{}'", entry.label),
                    &entry.params,
                    &[],
                    &layout,
                    BTreeMap::new(),
                )
                .map_err(|err| err.to_string())?;
                Some(member)
            }
            None => None,
        };
        let draws = if entry.kind == "geometry" {
            draws_of(objects, entry.select())
        } else {
            Vec::new()
        };
        let Some(faces) = entry.shadow_faces else {
            passes.push(PassSpec {
                kind: entry.kind.clone(),
                shader,
                label: entry.label.clone(),
                entry: entry.entry.clone(),
                reads: entry.reads.clone(),
                writes: entry.writes.clone(),
                params,
                draws,
                vertex_shader,
                vertex_entry,
                render: entry.render.clone(),
                depth_target: entry.depth_target.clone(),
                cube_face: None,
                viewport: None,
            });
            continue;
        };
        if faces != CUBE_FACES {
            return Err(format!(
                "{at} 的 shadow_faces 是 {faces}：cube 只有 {CUBE_FACES} 面\
                 （§109.1：Bevy 就是 6 个单层 pass）"
            ));
        }
        let allocation = allocation.as_ref().ok_or_else(|| {
            format!(
                "{at} 要展开影子 pass，而这一帧没有分配出任何页 —— \
                 帧图里不该出现这一条（一盏投影的灯都没有时它整条都不烘）"
            )
        })?;
        for light in 0..sources.shadow_lights as u32 {
            for face in 0..CUBE_FACES {
                let face_label = format!("{}_{}_{}", entry.label, light, FACE_NAMES[face as usize]);
                let layer = light * CUBE_FACES + face;
                let mut cleared = [false; crate::vshadow::MAX_LEVELS as usize];
                for patch in allocation
                    .patches
                    .iter()
                    .filter(|patch| patch.light == light && patch.face == face)
                {
                    let window = patch.window;
                    if entry.fragment_shader.is_some() {
                        let level = crate::vshadow::SHADOW_ATLAS_RESOURCES
                            .iter()
                            .position(|name| entry.depth_target.as_deref() == Some(*name))
                            .ok_or_else(|| {
                                format!(
                                    "{at} 的降采样 pass '{}' 的 depth_target 是 '{}'：\
                                     那不是某一级的影子 atlas（{}）",
                                    entry.label,
                                    entry.depth_target.as_deref().unwrap_or("(没写)"),
                                    crate::vshadow::SHADOW_ATLAS_RESOURCES.join(" / ")
                                )
                            })? as u32;
                        if level == 0 {
                            return Err(format!(
                                "{at} 的降采样 pass '{}' 往级 0 画：级 0 是最细的那一级，\
                                 没有更细的一级可降（降采样只能 k ≥ 1）",
                                entry.label
                            ));
                        }
                        if patch.level != level {
                            continue;
                        }
                        let page_size = crate::vshadow::PAGE_SIZE;
                        let mut down = std::collections::BTreeMap::new();
                        for (key, value) in [
                            ("level", level),
                            ("light", light),
                            ("face", face),
                            ("page_size", page_size),
                            ("origin_x", patch.page_x * page_size),
                            ("origin_y", patch.page_y * page_size),
                        ] {
                            down.insert(key.to_string(), Value::Num(f64::from(value)));
                        }
                        let mut page = PassSpec {
                            kind: entry.kind.clone(),
                            shader: shader.clone(),
                            label: format!(
                                "{}_down_c{}",
                                page_label(&face_label, patch.level, patch.page_y, patch.page_x),
                                passes.len()
                            ),
                            entry: entry.entry.clone(),
                            reads: entry.reads.clone(),
                            writes: entry.writes.clone(),
                            params: down,
                            draws: Vec::new(),
                            vertex_shader: vertex_shader.clone(),
                            vertex_entry: vertex_entry.clone(),
                            render: entry.render.clone(),
                            depth_target: entry
                                .depth_target
                                .as_ref()
                                .map(|_| crate::vshadow::shadow_atlas_resource(patch.level)),
                            cube_face: Some(PassCubeFace { light, face, layer }),
                            viewport: Some([
                                patch.atlas_x as f32,
                                patch.atlas_y as f32,
                                page_size as f32,
                                page_size as f32,
                            ]),
                        };
                        if cleared[patch.level as usize] {
                            page.render = load_state(&page.render);
                        }
                        cleared[patch.level as usize] = true;
                        passes.push(page);
                        continue;
                    }
                    for caster in patch.casters.iter() {
                        let Some(draw) = draws.iter().find(|draw| &draw.geometry == caster) else {
                            return Err(format!(
                                "{at} 的第 {light} 盏灯第 {} 面页 ({}, {}) 要画 '{}'，\
                                 而这一条的 `select` 里没有它（内部不一致）",
                                FACE_NAMES[face as usize], patch.page_x, patch.page_y, caster
                            ));
                        };
                        let mut page = PassSpec {
                            kind: entry.kind.clone(),
                            shader: shader.clone(),
                            label: page_label(&face_label, patch.level, patch.page_y, patch.page_x),
                            entry: entry.entry.clone(),
                            reads: entry.reads.clone(),
                            writes: entry.writes.clone(),
                            params: params.clone(),
                            draws: Vec::new(),
                            vertex_shader: vertex_shader.clone(),
                            vertex_entry: vertex_entry.clone(),
                            render: entry.render.clone(),
                            depth_target: entry
                                .depth_target
                                .as_ref()
                                .map(|_| crate::vshadow::shadow_atlas_resource(patch.level)),
                            cube_face: Some(PassCubeFace { light, face, layer }),
                            viewport: None,
                        };
                        page.label = format!(
                            "{}_c{}",
                            page_label(&face_label, patch.level, patch.page_y, patch.page_x),
                            passes.len()
                        );
                        page.draws = vec![DrawSpec {
                            geometry: draw.geometry.clone(),
                            material: draw.material.clone(),
                        }];
                        page.viewport = Some([
                            patch.atlas_x as f32,
                            patch.atlas_y as f32,
                            crate::vshadow::PAGE_SIZE as f32,
                            crate::vshadow::PAGE_SIZE as f32,
                        ]);
                        page.params = BTreeMap::from([(
                            "view_page".to_string(),
                            px_protocol::scene::Value::Quad(page_rect_in_face(
                                window,
                                crate::vshadow::pages_at_level(
                                    allocation.lights[light as usize].pages_per_side,
                                    patch.level,
                                ),
                            )),
                        )]);
                        if cleared[patch.level as usize] {
                            page.render = load_state(&page.render);
                        }
                        cleared[patch.level as usize] = true;
                        passes.push(page);
                    }
                }
            }
        }
    }
    Ok(Baked {
        resources,
        passes,
        materials,
        material_instances,
        shadow: allocation.as_ref().map(|allocation| {
            let mut table = Vec::with_capacity(allocation.table.len() * 4);
            for word in &allocation.table {
                table.extend_from_slice(&word.to_le_bytes());
            }
            px_protocol::scene::ShadowPlan {
                table,
                light_offsets: {
                    let mut offsets = Vec::with_capacity(allocation.lights.len());
                    let mut at = 0_u32;
                    for light in &allocation.lights {
                        offsets.push(at);
                        at += crate::vshadow::table_words_per_light(
                            light.pages_per_side,
                            light.levels,
                        );
                    }
                    offsets
                },
                atlas_side: allocation.atlas.0,
                layers: allocation.atlas.2,
                faces: px_protocol::scene::SHADOW_FACE_BASIS
                    .iter()
                    .flat_map(|face| face.iter().copied())
                    .collect(),
            }
        }),
    })
}

fn parse_fixed_size(text: &str) -> Result<(u32, u32), String> {
    let (width, height) = text
        .split_once('x')
        .ok_or_else(|| format!("帧图资源尺寸 '{text}' 不是 '<宽>x<高>'"))?;
    let width = width
        .trim()
        .parse::<u32>()
        .map_err(|err| format!("尺寸 '{text}' 的宽读不出来：{err}"))?;
    let height = height
        .trim()
        .parse::<u32>()
        .map_err(|err| format!("尺寸 '{text}' 的高读不出来：{err}"))?;
    if width == 0 || height == 0 {
        return Err(format!("尺寸 '{text}' 里有 0"));
    }
    Ok((width, height))
}

fn shadow_allocation(
    lights: &[px_protocol::scene::Light],
    objects: &[Object],
) -> Result<Option<crate::vshadow::Allocation>, String> {
    let casting: Vec<&px_protocol::scene::Light> =
        lights.iter().filter(|light| light.shadows).collect();
    if casting.is_empty() {
        return Ok(None);
    }
    let mut per_light: Vec<Vec<crate::vshadow::Caster>> = Vec::with_capacity(casting.len());
    for light in &casting {
        let mut casters = Vec::new();
        for object in objects {
            if !object.cast_shadow || object.shadow_density <= 0.0 {
                continue;
            }
            let radius = object.geometry.bounding_radius().ok_or_else(|| {
                format!(
                    "物体 '{}' 要投影（shadow_density {}），而它的几何没有 `bounding_radius`：\
                     虚拟影图要按包围球分页，而没有半径就算不出它占多少页",
                    object.id, object.shadow_density
                )
            })?;
            casters.push(crate::vshadow::Caster {
                id: object.id.clone(),
                radius: radius * object.transform.scale[0].abs(),
                position: [
                    object.transform.translation[0] - light.position[0],
                    object.transform.translation[1] - light.position[1],
                    object.transform.translation[2] - light.position[2],
                ],
                density: object.shadow_density,
            });
        }
        per_light.push(casters);
    }
    let allocation =
        crate::vshadow::allocate(&per_light).map_err(|err| format!("虚拟影图分配不出来：{err}"))?;
    Ok(Some(allocation))
}

fn page_rect_in_face(window: [u32; 4], pages_per_side: u32) -> [f32; 4] {
    let n = pages_per_side as f32;
    let span = 2.0 / n;
    let x0 = -1.0 + span * (window[0] as f32 / crate::vshadow::PAGE_SIZE as f32);
    let y0 = 1.0 - span * (window[1] as f32 / crate::vshadow::PAGE_SIZE as f32);
    [x0 + span * 0.5, y0 - span * 0.5, span * 0.5, span * 0.5]
}

fn page_label(face_label: &str, level: u32, page_y: u32, page_x: u32) -> String {
    format!("{face_label}_l{level}_p{page_y}_{page_x}")
}

fn page_clear_pass(template: &PassSpec, window: [u32; 4], label: String) -> PassSpec {
    let mut pass = template.clone();
    pass.label = label;
    pass.draws.clear();
    pass.render = clear_state(&template.render);
    pass.viewport = Some([
        window[0] as f32,
        window[1] as f32,
        (window[2] - window[0]) as f32,
        (window[3] - window[1]) as f32,
    ]);
    pass
}

fn clear_state(render: &str) -> String {
    swap_depth(render, "depth=clear(0)")
}

fn load_state(render: &str) -> String {
    swap_depth(render, "depth=load")
}

fn swap_depth(render: &str, want: &str) -> String {
    render
        .split('|')
        .map(|part| {
            if part.starts_with("depth=") {
                want.to_string()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn frame_stubs(symbol: &str) -> Option<&'static str> {
    match symbol {
        "bevy_pbr::mesh_view_bindings::view" => Some(px_shader::assemble::HOST_VIEW_STUB),
        other => px_shader::host_stubs::wgpu_host_stub(other),
    }
}

fn bake_material(
    material: &MaterialFile,
    sources: &Sources,
    modules: &px_shader::ModuleTable,
) -> Result<FrameMaterial, String> {
    let at = format!("帧材质 '{}'", material.name);
    let full = px_graph::workspace_root().join(&material.shader);
    let text = std::fs::read_to_string(&full)
        .map_err(|err| format!("{at} 读不了 {}：{err}", full.display()))?;
    let mut seen = Vec::new();
    let assembled = px_shader::assemble::render_source(&text, modules, frame_stubs, &mut seen);
    let layout = px_shader::reflect::reflect_assembled(&assembled, &at)
        .map_err(|err| format!("{at}（{}）反射不出参数块：{err}", material.shader))?;

    let entries = px_shader::reflect::entry_points(&assembled, &at)?;
    if !entries
        .iter()
        .any(|(name, stage)| name == &material.entry && *stage == "fragment")
    {
        let list = |stage: &str| -> String {
            let found: Vec<&str> = entries
                .iter()
                .filter(|(_, kind)| *kind == stage)
                .map(|(name, _)| name.as_str())
                .collect();
            if found.is_empty() {
                "（一个都没有）".to_string()
            } else {
                found.join(" / ")
            }
        };
        return Err(format!(
            "{at} 要的片元入口 '{}' 在 {} 里不存在。\n  那份 WGSL 的**片元**入口：{}\n  \
             全部入口：{}",
            material.entry,
            material.shader,
            list("fragment"),
            if entries.is_empty() {
                "（一个都没有）".to_string()
            } else {
                entries
                    .iter()
                    .map(|(name, stage)| format!("{name}（{stage}）"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            }
        ));
    }

    let mut given: BTreeMap<String, toml::Value> = BTreeMap::new();
    for name in material.params.keys() {
        if layout.param(name).is_none() {
            return Err(format!(
                "{at} 不认识参数 '{name}'：\n  这份 WGSL 声明的参数：{}\n  \
                 ⇒ 要么名字拼错了，要么得先在 shader 的结构体里声明它\
                 （声明之后按名字透传，不用改 Rust）",
                layout.param_names()
            ));
        }
    }
    for slot in &layout.params {
        let Some(value) = material.params.get(&slot.name) else {
            return Err(format!(
                "{at} 的 WGSL 声明了参数 '{}'（{}），而配方没给它**来源**。\n  \
                 认得的来源：{}\n  ⇒ 在 `[[materials]]` 的 params 里加一行 `{} = \"<来源>\"`",
                slot.name,
                slot.kind.name(),
                SOURCES.join(" / "),
                slot.name
            ));
        };
        let Some(source) = value.as_str() else {
            return Err(format!(
                "{at} 的参数 '{}' 给的是 {value}：参数要说**来源**，不是值 ——\n  \
                 帧图是六个场景共用的一份，写死一个数就等于把内容焊进帧策略\
                 （§133：六份场景的亮度今天恰好都是 900，而内容可以不是）。\n  \
                 认得的来源：{}",
                slot.name,
                SOURCES.join(" / ")
            ));
        };
        let Some((resolved, kind)) = source_of(source, sources) else {
            return Err(format!(
                "{at} 的参数 '{}' 说的来源是 '{source}'：**不认得这个来源**。\n  \
                 认得的来源：{}",
                slot.name,
                SOURCES.join(" / ")
            ));
        };
        if kind != slot.kind {
            return Err(format!(
                "{at} 的参数 '{}'：来源 '{source}' 是 {}，而 WGSL 里声明的是 {} —— 类型不符\n  \
                 ⇒ 要么改 WGSL 那一格，要么换一个类型对得上的来源（认得的：{}）",
                slot.name,
                kind.name(),
                slot.kind.name(),
                SOURCES.join(" / ")
            ));
        }
        given.insert(slot.name.clone(), toml_of(&resolved));
    }

    let params = crate::contract::merge_named(&at, &given, &[], &layout, BTreeMap::new())
        .map_err(|err| err.to_string())?;
    Ok(FrameMaterial {
        name: material.name.clone(),
        shader: text,
        entry: material.entry.clone(),
        params,
    })
}

pub fn frame_labels(frame: &FrameFile) -> Vec<String> {
    frame
        .before
        .iter()
        .chain(frame.after.iter())
        .map(|entry| entry.label.clone())
        .collect()
}

fn expanded_labels(frame: &FrameFile, shadow_lights: usize) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for entry in frame.before.iter().chain(frame.after.iter()) {
        match entry.shadow_faces {
            None => labels.push(entry.label.clone()),
            Some(_) => labels.push(format!(
                "{}_<灯>_<面>_p<页行>_<页列>（× {shadow_lights} 盏灯 × {CUBE_FACES} 面 × 该面分配到的页，\
                 每页一笔 `<...>_c<序号>` 是那一页后续的物体）",
                entry.label
            )),
        }
    }
    labels
}

pub fn verify(spec: &SceneSpec, frame: &FrameFile, name: &str) -> Result<(), String> {
    let found: Vec<&str> = spec.passes.iter().map(|pass| pass.label.as_str()).collect();
    let mut at = 0_usize;
    for entry in frame.before.iter().chain(frame.after.iter()) {
        match entry.shadow_faces {
            None => {
                if found.get(at) != Some(&entry.label.as_str()) {
                    return Err(mismatch(name, frame, &found, at, &entry.label));
                }
                at += 1;
            }
            Some(faces) => {
                if faces != CUBE_FACES {
                    return Err(format!(
                        "帧图 '{name}' 的 '{}' 写了 shadow_faces = {faces}：cube 只有 {CUBE_FACES} 面",
                        entry.label
                    ));
                }
                let mut light = 0_u32;
                loop {
                    let mut matched_faces = 0_u32;
                    for face in 0..CUBE_FACES {
                        let prefix =
                            format!("{}_{}_{}", entry.label, light, FACE_NAMES[face as usize]);
                        let start = at;
                        while found
                            .get(at)
                            .is_some_and(|label| label.starts_with(&prefix))
                        {
                            at += 1;
                        }
                        if at > start {
                            matched_faces += 1;
                        }
                    }
                    if matched_faces == 0 {
                        break;
                    }
                    if matched_faces != CUBE_FACES {
                        return Err(format!(
                            "帧图 '{name}' 的 '{}' 展开到第 {light} 盏灯时只找到 {matched_faces} 面\
                             （应当是 {CUBE_FACES} 面）：要么产物是半截的，要么标签不是这一条烘的。\
                             实际标签：[{}]",
                            entry.label,
                            found.join(" / ")
                        ));
                    }
                    light += 1;
                }
            }
        }
    }
    if at != found.len() {
        return Err(mismatch(name, frame, &found, at, "（帧图的条目已经走完）"));
    }
    Ok(())
}
fn mismatch(name: &str, frame: &FrameFile, found: &[&str], at: usize, want: &str) -> String {
    format!(
        "这份产物不是用帧图 '{name}' 烘的：第 {at} 个标签应当是 '{want}'，实际是 '{}'。\
         期望（配方里那些条目，`cube_faces` 的按灯×面展开）：[{}]；\
         实际：[{}]。要么改用烘它的那张帧图（--frame <名>），要么用 --no-frame-graph 走老形状",
        found.get(at).copied().unwrap_or("（没有更多了）"),
        expanded_labels(frame, 1).join(" / "),
        if found.is_empty() {
            "（空）".to_string()
        } else {
            found.join(" / ")
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::scene::{CullMode, Geometry, Material, Transform};

    fn ensure_shader_graph() {
        if shader_graph_is_ready() {
            return;
        }
        let baked = px_graph::bake_shader_graph().unwrap_or_else(|err| {
            panic!("盘上没有 `shaders` 图的产物，现烘又失败（帧图按名字查它的成员）：{err}")
        });
        assert!(
            shader_graph_is_ready(),
            "现烘了 {} 份入口 shader，清单/产物还是不全",
            baked.len()
        );
    }

    fn shader_graph_is_ready() -> bool {
        let root = px_graph::cache_root();
        let Ok(text) = std::fs::read_to_string(root.join("shaders").join("manifest.json")) else {
            return false;
        };
        let Ok(entries) = serde_json::from_str::<Vec<px_graph::ManifestEntry>>(&text) else {
            return false;
        };
        !entries.is_empty()
            && entries.iter().all(|entry| {
                px_protocol::scene::cas_path(&root, &entry.key)
                    .map(|path| path.is_file())
                    .unwrap_or(false)
            })
    }

    fn object(id: &str, alpha: AlphaMode) -> Object {
        let mut material = Material::new(Member::new("shaders", "surface", &"a".repeat(64)));
        material.alpha = alpha;
        material.cull = CullMode::Back;
        Object {
            id: id.to_string(),
            geometry: Geometry::primitive(
                "icosphere",
                BTreeMap::from([("radius".to_string(), Value::Num(1.0))]),
            )
            .with_bounding_radius(1.0),
            material,
            transform: Transform::default(),
            cast_shadow: true,
            shadow_density: 256.0,
        }
    }

    #[test]
    fn a_select_picks_objects_by_their_material_alpha() {
        let objects = vec![
            object("planet", AlphaMode::Opaque),
            object("atmosphere", AlphaMode::Add),
            object("clouds", AlphaMode::Premultiplied),
            object("rings", AlphaMode::Blend),
        ];
        let opaque = draws_of(&objects, "opaque");
        assert_eq!(
            opaque
                .iter()
                .map(|d| d.geometry.as_str())
                .collect::<Vec<_>>(),
            vec!["planet"]
        );
        let transparent = draws_of(&objects, "transparent");
        assert_eq!(
            transparent
                .iter()
                .map(|d| d.geometry.as_str())
                .collect::<Vec<_>>(),
            vec!["rings", "clouds", "atmosphere"]
        );
        assert_eq!(transparent[0].material, "rings");
        let sky = draws_of(&objects, "skybox");
        assert_eq!(sky.len(), 1);
        assert_eq!(sky[0].geometry, "skybox");
        assert_eq!(draws_of(&objects, "none").len(), 0);
    }

    #[test]
    fn a_transparent_pass_sorts_by_depth_bias_before_the_reversed_order() {
        let mut clouds = object("clouds", AlphaMode::Premultiplied);
        clouds.material.depth_bias = -1.0;
        let objects = vec![
            object("planet", AlphaMode::Opaque),
            object("atmosphere", AlphaMode::Add),
            clouds,
            object("rings", AlphaMode::Blend),
        ];
        assert_eq!(
            draws_of(&objects, "transparent")
                .iter()
                .map(|d| d.geometry.as_str())
                .collect::<Vec<_>>(),
            vec!["clouds", "rings", "atmosphere"]
        );
    }

    fn sources() -> Sources {
        Sources {
            ambient: 80.0,
            skybox_brightness: 900.0,
            shadow_lights: 1,
        }
    }

    fn lights() -> Vec<px_protocol::scene::Light> {
        vec![
            px_protocol::scene::Light::point("sun", [-4.2, 1.15, 2.35], [1.0, 1.0, 1.0], 7.6e5)
                .with_shadows(true),
        ]
    }

    #[test]
    fn the_legacy_switch_emits_nothing_at_all() {
        ensure_shader_graph();
        let frame = load(DEFAULT_FRAME).expect("默认帧图要能读");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), &lights(), false).expect("老形状");
        assert!(baked.resources.is_empty(), "老形状不许有 resources");
        assert!(baked.passes.is_empty(), "老形状不许有 passes");
        assert!(
            baked.materials.is_empty(),
            "老形状不许有 frame_materials（多一节就改产物字节）"
        );
        let baked = build(&frame, &objects, &sources(), &lights(), true).expect("帧图");
        assert!(!baked.resources.is_empty(), "帧图要声明中间目标");
        let mut document = SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: "判据".to_string(),
            environment: px_protocol::scene::Environment::default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            resources: baked.resources.clone(),
            passes: baked.passes.clone(),
            lights: lights(),
            shadow: None,
            objects: objects.clone(),
            frame_materials: Vec::new(),
            material_instances: Vec::new(),
        };
        document.resources = baked.resources.clone();
        document.passes = baked.passes.clone();
        verify(&document, &frame, DEFAULT_FRAME).expect("自己烘出来的标签自己要认");
        let cubes: Vec<(u32, u32, u32)> = baked
            .passes
            .iter()
            .filter_map(|pass| pass.cube_face.map(|c| (c.light, c.face, c.layer)))
            .collect();
        assert!(
            !cubes.is_empty(),
            "一条 `shadow_faces = 6` 的条目要展开成若干页，每页都带 (灯, 面, 层)"
        );
        for (light, face, layer) in &cubes {
            assert_eq!(
                *layer,
                light * CUBE_FACES + face,
                "层号必须是 灯 × 6 + 面（烘图侧那一份算式）"
            );
        }
        let mut faces_seen: Vec<u32> = cubes.iter().map(|(_, face, _)| *face).collect();
        faces_seen.sort_unstable();
        faces_seen.dedup();
        assert_eq!(faces_seen.len(), CUBE_FACES as usize, "六面各要有页");
        assert_eq!(baked.materials.len(), frame.materials.len());
        let skybox = baked
            .materials
            .iter()
            .find(|material| material.name == "skybox")
            .expect("默认帧图里那份天空盒");
        assert_eq!(skybox.entry, "fragment");
        assert!(
            skybox.shader.contains("coords_to_ray_direction"),
            "内联的必须是**文件全文**（不是路径、也不是摘要）：{} 字节",
            skybox.shader.len()
        );
        assert!(
            skybox.shader.contains("#{MATERIAL_BIND_GROUP}"),
            "落进文档的是**组装前**的原文 —— 组装是宿主的事（它有自己的桩表与组号）"
        );
        assert_eq!(
            skybox.params.get("brightness"),
            Some(&Value::Num(900.0)),
            "参数要按反射出来的结构体打包：{:?}",
            skybox.params
        );
    }

    #[test]
    fn a_broken_frame_material_is_refused_at_bake_time() {
        let modules = px_shader::workspace_modules(&px_graph::workspace_root()).expect("模块表");
        let material = |params: &str| -> MaterialFile {
            toml::from_str(&format!(
                "name = \"skybox\"\nshader = \"art/frame/skybox.wgsl\"\nentry = \"fragment\"\nparams = {{ {params} }}\n"
            ))
            .expect("夹具")
        };

        let err = bake_material(
            &material("brightness = \"environment.skyboox_brightness\""),
            &sources(),
            &modules,
        )
        .expect_err("不认识的来源 ⇒ 拒");
        assert!(err.contains("environment.skyboox_brightness"), "{err}");
        assert!(
            err.contains("environment.skybox_brightness"),
            "要把认得的来源列出来：{err}"
        );

        let err = bake_material(&material(""), &sources(), &modules).expect_err("声明了没给 ⇒ 拒");
        assert!(err.contains("brightness"), "{err}");
        assert!(err.contains("来源"), "{err}");

        let err = bake_material(&material("brightness = 900.0"), &sources(), &modules)
            .expect_err("给值 ⇒ 拒");
        assert!(err.contains("brightness"), "{err}");
        assert!(err.contains("来源"), "{err}");

        let dir = px_graph::workspace_root()
            .join("target")
            .join("frame-material-fixture");
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        let fixture = dir.join("vec3_param.wgsl");
        std::fs::write(
            &fixture,
            "struct FixtureParams { brightness: vec3<f32> };\n\
             @group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: FixtureParams;\n\
             @fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(params.brightness, 1.0); }\n",
        )
        .expect("写夹具");
        let mut wrong_type = material("brightness = \"environment.skybox_brightness\"");
        wrong_type.shader = "target/frame-material-fixture/vec3_param.wgsl".to_string();
        wrong_type.entry = "fs_main".to_string();
        let err = bake_material(&wrong_type, &sources(), &modules).expect_err("类型不符 ⇒ 拒");
        assert!(err.contains("类型不符"), "{err}");
        assert!(err.contains("vec3"), "要说清声明的是哪一档：{err}");

        let mut wrong_entry = material("brightness = \"environment.skybox_brightness\"");
        wrong_entry.entry = "fs_main".to_string();
        let err = bake_material(&wrong_entry, &sources(), &modules).expect_err("入口不存在 ⇒ 拒");
        assert!(err.contains("fs_main"), "要点名那个指不到的名字：{err}");
        assert!(
            err.contains("fragment"),
            "要把那份 WGSL 实际的入口列出来：{err}"
        );

        let err = bake_material(
            &material("brightness = \"environment.ambient\", gain = \"environment.ambient\""),
            &sources(),
            &modules,
        )
        .expect_err("多给参数 ⇒ 拒");
        assert!(err.contains("gain"), "{err}");
        assert!(
            err.contains("brightness"),
            "要列出 shader 声明的参数：{err}"
        );

        let dim = Sources {
            ambient: 80.0,
            skybox_brightness: 1200.0,
            shadow_lights: 1,
        };
        let baked = bake_material(
            &material("brightness = \"environment.skybox_brightness\""),
            &dim,
            &modules,
        )
        .expect("换一个亮度也要烘得出来");
        assert_eq!(baked.params["brightness"], Value::Num(1200.0));
    }

    #[test]
    fn a_broken_frame_recipe_is_refused_by_name() {
        fn entry_mut<'a>(frame: &'a mut FrameFile, label: &str) -> &'a mut EntryFile {
            frame
                .before
                .iter_mut()
                .chain(frame.after.iter_mut())
                .find(|entry| entry.label == label)
                .unwrap_or_else(|| panic!("默认帧图里没有 '{label}' 这一条"))
        }

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        let before = frame.chain_color.clone();
        frame.chain_color = vec![before[0].clone()];
        let err = frame.check().expect_err("乒乓对只有一个 ⇒ 拒");
        assert!(err.contains("恰好两个"), "{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        frame.chain_color = vec!["nope".to_string(), before[1].clone()];
        let err = frame.check().expect_err("乒乓对里有没声明的名字 ⇒ 拒");
        assert!(err.contains("nope"), "{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        entry_mut(&mut frame, "prepass").select = Some("clouds".to_string());
        let err = frame.check().expect_err("不认识的 select ⇒ 拒");
        assert!(err.contains("clouds"), "{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        entry_mut(&mut frame, "opaque").fragment_shader = Some("px_grade".to_string());
        let err = frame.check().expect_err("几何 pass 给片元成员 ⇒ 拒");
        assert!(err.contains("属于材质"), "{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        entry_mut(&mut frame, "copy_depth").vertex_shader =
            Some("art/frame/vertex_mesh.wgsl".to_string());
        let err = frame.check().expect_err("copy 给顶点阶段 ⇒ 拒");
        assert!(err.contains("拷贝不画东西"), "{err}");
        assert!(err.contains("vertex_shader"), "要指出是哪一栏：{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        entry_mut(&mut frame, "copy_depth").fragment_shader = Some("blit".to_string());
        let err = frame.check().expect_err("copy 给片元成员 ⇒ 拒");
        assert!(err.contains("拷贝不画东西"), "{err}");
    }

    #[test]
    fn verify_names_the_expected_frame_and_what_it_found() {
        ensure_shader_graph();
        let frame = load(DEFAULT_FRAME).expect("默认帧图");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), &lights(), true).expect("帧图");
        let spec = SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: "夹具".to_string(),
            environment: Default::default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            resources: baked.resources,
            passes: baked.passes,
            lights: Vec::new(),
            shadow: None,
            objects,
            frame_materials: baked.materials,
            material_instances: baked.material_instances,
        };
        verify(&spec, &frame, DEFAULT_FRAME).expect("自己烘的自己认");

        let mut stale = spec.clone();
        stale.passes.truncate(2);
        let err = verify(&stale, &frame, DEFAULT_FRAME).expect_err("少了两条 ⇒ 拒");
        assert!(err.contains(DEFAULT_FRAME), "要说清是哪张帧图：{err}");
        assert!(err.contains("prepass"), "要列出期望的标签：{err}");
    }

    #[test]
    fn the_baked_materials_satisfy_the_document_checks() {
        ensure_shader_graph();
        let frame = load(DEFAULT_FRAME).expect("默认帧图");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), &lights(), true).expect("帧图");
        let spec = SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: "夹具".to_string(),
            environment: Default::default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            resources: baked.resources,
            passes: baked.passes,
            lights: Vec::new(),
            shadow: None,
            objects,
            frame_materials: baked.materials,
            material_instances: baked.material_instances,
        };
        spec.check()
            .unwrap_or_else(|err| panic!("帧图烘出来的文档要自洽：{err}"));
        let mut without = spec.clone();
        without.frame_materials.clear();
        let err = without
            .check()
            .expect_err("少了帧材质，`sky` 的 draw 就没人认领了 ⇒ 必须拒");
        assert!(err.contains("skybox"), "{err}");
    }
}
