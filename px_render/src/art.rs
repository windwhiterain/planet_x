use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_protocol::art::{self, TextureFormat, TextureShape};
use px_protocol::material::{MaterialLayout, TextureDimension};
use px_protocol::scene::{
    AlphaMode, CullMode, FrameMaterial, Geometry, Member, Object, Sampler, SceneSpec, Transform,
    Value,
};
use px_protocol::wire::DType;

use crate::material::version_of;
use crate::mesh::Mesh;
use crate::shader;

#[derive(Clone, Debug)]
pub struct LoadedTexture {
    pub member: Member,
    pub shape: TextureShape,
    pub bytes: Vec<u8>,
    pub image: image::RgbaImage,
}

impl LoadedTexture {
    pub fn label(&self) -> String {
        self.member.to_string()
    }

    pub fn base_level_bytes(&self) -> usize {
        self.shape.width as usize
            * self.shape.height as usize
            * self.shape.layers as usize
            * self.shape.format.texel_bytes()
    }
}

#[derive(Clone, Debug)]
pub struct BoundTexture {
    pub binding: u32,
    pub sampler: Sampler,
    pub texture: LoadedTexture,
}

#[derive(Clone, Debug)]
pub struct Skybox {
    pub sampler: Sampler,
    pub texture: LoadedTexture,
}

impl Skybox {
    pub fn bound(&self, binding: u32) -> BoundTexture {
        BoundTexture {
            binding,
            sampler: self.sampler,
            texture: self.texture.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoadedShader {
    pub member: Member,
    pub source: String,
    pub assembled: String,
    pub layout: MaterialLayout,
    pub closure: String,
}

#[derive(Clone, Debug)]
pub enum LoadedGeometry {
    Mesh {
        member: Member,
        mesh: Mesh,
    },
    Primitive {
        name: String,
        params: BTreeMap<String, Value>,
        mesh: Mesh,
    },
}

impl LoadedGeometry {
    pub fn mesh(&self) -> &Mesh {
        match self {
            LoadedGeometry::Mesh { mesh, .. } => mesh,
            LoadedGeometry::Primitive { mesh, .. } => mesh,
        }
    }

    pub fn member(&self) -> Option<&Member> {
        match self {
            LoadedGeometry::Mesh { member, .. } => Some(member),
            LoadedGeometry::Primitive { .. } => None,
        }
    }

    pub fn primitive(&self) -> Option<(&str, &BTreeMap<String, Value>)> {
        match self {
            LoadedGeometry::Mesh { .. } => None,
            LoadedGeometry::Primitive { name, params, .. } => Some((name.as_str(), params)),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            LoadedGeometry::Mesh { member, mesh } => format!(
                "网格 {member}（{} 顶点 / {} 三角形）",
                mesh.vertex_count(),
                mesh.triangle_count()
            ),
            LoadedGeometry::Primitive { name, mesh, .. } => format!(
                "图元 {name}（{} 顶点 / {} 三角形）",
                mesh.vertex_count(),
                mesh.triangle_count()
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoadedObject {
    pub id: String,
    pub geometry: LoadedGeometry,
    pub shader: LoadedShader,
    pub params: Vec<u8>,
    pub textures: Vec<BoundTexture>,
    pub alpha: AlphaMode,
    pub cull: CullMode,
    pub depth_bias: f32,
    pub cast_shadow: bool,
    pub transform: Transform,
}

impl LoadedObject {
    pub fn texture(&self, binding: u32) -> Option<&BoundTexture> {
        self.textures.iter().find(|bound| bound.binding == binding)
    }

    pub fn param_offset(&self, name: &str) -> Option<u32> {
        self.shader.layout.param(name).map(|slot| slot.offset)
    }

    pub fn f32_at(&self, name: &str) -> Option<f32> {
        let slot = self.shader.layout.param(name)?;
        if slot.kind != px_protocol::material::ParamKind::F32 {
            return None;
        }
        let start = slot.offset as usize;
        let bytes = self.params.get(start..start + 4)?;
        Some(f32::from_le_bytes(bytes.try_into().ok()?))
    }
}

#[derive(Clone, Debug)]
pub struct LoadedScene {
    pub name: String,
    pub ambient: f32,
    pub skybox: Option<Skybox>,
    pub skybox_brightness: f32,
    pub objects: Vec<LoadedObject>,
    pub shadow: Option<px_protocol::scene::ShadowPlan>,
}

impl LoadedScene {
    pub fn object(&self, id: &str) -> Option<&LoadedObject> {
        self.objects.iter().find(|object| object.id == id)
    }

    pub fn audit(&self) -> String {
        let mut lines = vec![format!(
            "渲染文档 {}｜物体 {} 个｜环境光 {}｜天空盒 {}（亮度 {}）",
            self.name,
            self.objects.len(),
            self.ambient,
            match &self.skybox {
                Some(skybox) => format!(
                    "{}｜{}×{}×{} 层｜{} 级 mip",
                    skybox.texture.label(),
                    skybox.texture.shape.width,
                    skybox.texture.shape.height,
                    skybox.texture.shape.layers,
                    skybox.texture.shape.levels
                ),
                None => "无".to_string(),
            },
            self.skybox_brightness
        )];
        for object in &self.objects {
            lines.push(format!(
                "  {}：{}｜shader {}｜参数 {} 字节｜贴图 {} 张｜{:?}/{:?}{}",
                object.id,
                object.geometry.describe(),
                object.shader.member,
                object.params.len(),
                object.textures.len(),
                object.alpha,
                object.cull,
                if object.cast_shadow {
                    ""
                } else {
                    "｜不投影"
                }
            ));
            for bound in &object.textures {
                lines.push(format!(
                    "    第 {} 格：{}｜{}×{}×{} 层｜{} 级 mip｜{}",
                    bound.binding,
                    bound.texture.label(),
                    bound.texture.shape.width,
                    bound.texture.shape.height,
                    bound.texture.shape.layers,
                    bound.texture.shape.levels,
                    bound.texture.shape.format.name()
                ));
            }
        }
        lines.join("\n")
    }
}

pub fn default_pcg_root() -> PathBuf {
    shader::workspace().join("target/pcg")
}

pub fn read_spec(scene_path: &Path) -> Result<SceneSpec, String> {
    let spec = px_protocol::scene::read_scene(scene_path)?;
    spec.check()?;
    Ok(spec)
}

pub fn load_scene(scene_path: &Path, pcg_root: &Path) -> Result<LoadedScene, String> {
    let spec = read_spec(scene_path)?;
    let modules = shader::modules();
    let skybox = match &spec.environment.skybox {
        Some(member) => Some(Skybox {
            sampler: Sampler::clamped(),
            texture: load_texture(member, pcg_root, "天空盒")?,
        }),
        None => None,
    };
    let mut objects = Vec::with_capacity(spec.objects.len());
    for object in &spec.objects {
        objects.push(load_object(object, pcg_root, &modules)?);
    }
    Ok(LoadedScene {
        name: spec.name.clone(),
        ambient: spec.environment.ambient,
        skybox,
        skybox_brightness: spec.environment.skybox_brightness,
        objects,
        shadow: spec.shadow.clone(),
    })
}

fn load_object(
    object: &Object,
    pcg_root: &Path,
    modules: &px_shader::ModuleTable,
) -> Result<LoadedObject, String> {
    let id = object.id.as_str();
    let shader = load_shader(&object.material.shader, pcg_root, modules)?;
    let params = shader
        .layout
        .pack(&object.material.params)
        .map_err(|err| format!("物体 '{id}' 的材质参数：{err}"))?;

    let mut textures: Vec<BoundTexture> = Vec::with_capacity(object.material.textures.len());
    for (role, reference) in &object.material.textures {
        let slot = shader.layout.texture(reference.binding).ok_or_else(|| {
            format!(
                "物体 '{id}' 的贴图 '{role}' 要绑在第 {} 格，但 shader {}/{} 没在那里声明贴图",
                reference.binding, shader.member.graph, shader.member.node
            )
        })?;
        let texture = load_texture(
            &reference.member,
            pcg_root,
            &format!("物体 '{id}' 的贴图 '{role}'"),
        )?;
        if texture.shape.layers != slot.dimension.layers() {
            return Err(format!(
                "物体 '{id}' 的贴图 '{role}' 是 {} 层，而 shader 第 {} 格声明的是 {}（{} 层）",
                texture.shape.layers,
                reference.binding,
                slot.dimension.name(),
                slot.dimension.layers()
            ));
        }
        textures.push(BoundTexture {
            binding: reference.binding,
            sampler: reference.sampler,
            texture,
        });
    }
    textures.sort_by_key(|bound| bound.binding);

    Ok(LoadedObject {
        id: object.id.clone(),
        geometry: load_geometry(&object.geometry, id, pcg_root)?,
        shader,
        params,
        textures,
        alpha: object.material.alpha,
        cull: object.material.cull,
        depth_bias: object.material.depth_bias,
        cast_shadow: object.cast_shadow,
        transform: object.transform,
    })
}

pub(crate) struct Reflected {
    pub assembled: String,
    pub layout: MaterialLayout,
    pub module: naga::Module,
}

pub(crate) fn reflect_source(
    name: &str,
    source: &str,
    modules: &px_shader::ModuleTable,
) -> Result<Reflected, String> {
    let assembled = shader::assemble(source, modules, crate::stubs::stubs);
    let module = shader::validate(name, &assembled)?;
    let layout = px_shader::reflect::reflect_assembled(&assembled, name)?;
    Ok(Reflected {
        assembled,
        layout,
        module,
    })
}

pub(crate) fn fragment_entry(module: &naga::Module, entry: &str, at: &str) -> Result<(), String> {
    let names = |stage: Option<naga::ShaderStage>| -> String {
        let found: Vec<&str> = module
            .entry_points
            .iter()
            .filter(|point| stage.is_none_or(|wanted| point.stage == wanted))
            .map(|point| point.name.as_str())
            .collect();
        if found.is_empty() {
            "（一个都没有）".to_string()
        } else {
            found.join(" / ")
        }
    };
    if module
        .entry_points
        .iter()
        .any(|point| point.name == entry && point.stage == naga::ShaderStage::Fragment)
    {
        return Ok(());
    }
    Err(format!(
        "{at} 要的片元入口 '{entry}' 在它自己的 WGSL 里不存在。\n  \
         那份 WGSL 的**片元**入口：{}\n  全部入口：{}",
        names(Some(naga::ShaderStage::Fragment)),
        names(None)
    ))
}

fn content_key(assembled: &str) -> Result<u64, String> {
    version_of(&crate::digest::sha256_hex(assembled.as_bytes()))
}

#[derive(Clone, Debug)]
pub struct LoadedFrameMaterial {
    pub name: String,
    pub entry: String,
    pub assembled: String,
    pub params: Vec<u8>,
    pub textures: Vec<(u32, TextureDimension)>,
    pub version: u64,
}

pub fn load_frame_material(
    material: &FrameMaterial,
    modules: &px_shader::ModuleTable,
) -> Result<LoadedFrameMaterial, String> {
    let at = format!("帧自有材质 '{}'", material.name);
    let reflected = reflect_source(&at, &material.shader, modules)?;
    fragment_entry(&reflected.module, &material.entry, &at)?;
    let params = reflected
        .layout
        .pack(&material.params)
        .map_err(|err| format!("{at} 的参数：{err}"))?;
    Ok(LoadedFrameMaterial {
        name: material.name.clone(),
        entry: material.entry.clone(),
        textures: reflected
            .layout
            .textures
            .iter()
            .map(|slot| (slot.binding, slot.dimension))
            .collect(),
        version: content_key(&reflected.assembled)?,
        assembled: reflected.assembled,
        params,
    })
}

pub(crate) fn load_shader(
    member: &Member,
    pcg_root: &Path,
    modules: &px_shader::ModuleTable,
) -> Result<LoadedShader, String> {
    const REBAKE: &str = "重烘配方：cargo run -p px_graphs --bin shaders；再逐个 cargo run -p px_graphs --bin scene <名>";
    let path = member.resolve(pcg_root)?;
    let bundle = art::read_manifest(&path)?;
    let recorded = bundle
        .assets
        .first()
        .and_then(|asset| px_shader::closure_from_params(&asset.params));
    let (source, schema) = art::read_shader_parts(&path)?;
    let closure = px_shader::closure(&source, modules);
    let fingerprint = closure.fingerprint();
    match recorded {
        Some(value) if value == fingerprint => {}
        Some(value) => {
            return Err(format!(
                "shader 成员 {member} 的 include 闭包对不上：产物记的 {value:016x}｜盘上现在的 {fingerprint:016x}\n  \
                 ⇒ 这份产物是拿另一版 include 烘的，画出来既不是老那一版也不是新那一版。\n  {REBAKE}"
            ));
        }
        None => {
            return Err(format!(
                "shader 成员 {member} 的产物没有 include 闭包指纹。\n  {REBAKE}"
            ));
        }
    }

    let name = format!("{}/{}", member.graph, member.node);
    let reflected = reflect_source(&name, &source, modules)?;
    let layout = reflected.layout;
    let now = layout.to_json()?;
    match &schema {
        Some(text) if *text == now => {}
        Some(text) => {
            return Err(format!(
                "shader 成员 {member} 的 schema descriptor 对不上：\n  产物记的 {text}\n  现在的   {now}\n  {REBAKE}"
            ));
        }
        None => {
            return Err(format!(
                "shader 成员 {member} 的产物没有 schema descriptor。\n  {REBAKE}"
            ));
        }
    }

    Ok(LoadedShader {
        member: member.clone(),
        source,
        assembled: reflected.assembled,
        layout,
        closure: closure.summary(),
    })
}

fn load_texture(member: &Member, pcg_root: &Path, what: &str) -> Result<LoadedTexture, String> {
    let path = member.resolve(pcg_root)?;
    let bytes = std::fs::read(&path).map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let frames = px_protocol::stream::read_stream(&mut bytes.as_slice())
        .map_err(|err| format!("{what} {member}（{}）不是产物流：{err}", path.display()))?;
    let manifest = frames
        .iter()
        .find_map(|frame| match frame {
            px_protocol::stream::Frame::Art(bundle) => bundle.assets.first(),
            _ => None,
        })
        .ok_or_else(|| format!("{what} {member}（{}）里没有清单帧", path.display()))?;
    let shape = TextureShape::from_params(&manifest.params)
        .map_err(|err| format!("{what} {member}：{err}"))?;
    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            px_protocol::stream::Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .ok_or_else(|| format!("{what} {member}（{}）里没有载荷块", path.display()))?;
    let expected = match shape.format {
        TextureFormat::Rgba8Srgb => DType::U8,
        TextureFormat::Rgba16Float => DType::U16,
    };
    if blob.header.dtype != expected {
        return Err(format!(
            "{what} {member}：格式是 {} 但载荷位深是 {:?}（应当是 {expected:?}）",
            shape.format.name(),
            blob.header.dtype
        ));
    }
    if blob.bytes.len() != shape.chain_bytes() {
        return Err(format!(
            "{what} {member}：{}×{}×{} 层 {} 级 mip 应当是 {} 字节，实际 {} 字节",
            shape.width,
            shape.height,
            shape.layers,
            shape.levels,
            shape.chain_bytes(),
            blob.bytes.len()
        ));
    }
    let image = decode_base_level(&shape, &blob.bytes)?;
    Ok(LoadedTexture {
        member: member.clone(),
        shape,
        bytes: blob.bytes.clone(),
        image,
    })
}

fn load_geometry(geometry: &Geometry, id: &str, pcg_root: &Path) -> Result<LoadedGeometry, String> {
    match geometry {
        Geometry::Mesh { member, .. } => {
            let path = member.resolve(pcg_root)?;
            let (mesh, audit) = crate::mesh::load_mesh(&path.display().to_string())
                .map_err(|err| format!("物体 '{id}'：{err}"))?;
            println!("{}", audit.trim_end());
            Ok(LoadedGeometry::Mesh {
                member: member.clone(),
                mesh,
            })
        }
        Geometry::Primitive { name, params, .. } => {
            let mesh = primitive_mesh(name, params).map_err(|err| format!("物体 '{id}'：{err}"))?;
            Ok(LoadedGeometry::Primitive {
                name: name.clone(),
                params: params.clone(),
                mesh,
            })
        }
    }
}

pub fn primitive_mesh(name: &str, params: &BTreeMap<String, Value>) -> Result<Mesh, String> {
    let number = |key: &str| -> Result<f32, String> {
        match params.get(key) {
            Some(Value::Num(value)) => Ok(*value as f32),
            Some(other) => Err(format!(
                "图元 '{name}' 的参数 '{key}' 要一个数，实际是 {other:?}"
            )),
            None => Err(format!(
                "图元 '{name}' 缺参数 '{key}'；它有的参数：{}",
                if params.is_empty() {
                    "（空）".to_string()
                } else {
                    params.keys().cloned().collect::<Vec<_>>().join(" / ")
                }
            )),
        }
    };
    match name {
        "icosphere" => {
            let radius = number("radius")?;
            let subdivisions = number("subdivisions")?;
            if !(subdivisions.fract() == 0.0 && (1.0..=64.0).contains(&subdivisions)) {
                return Err(format!(
                    "图元 'icosphere' 的 'subdivisions' 是 {subdivisions}：要 1..=64 的整数"
                ));
            }
            Ok(crate::icosphere::icosphere(radius, subdivisions as u32))
        }
        other => Err(format!("不认识的图元 '{other}'；这份渲染器认：icosphere")),
    }
}

fn decode_base_level(shape: &TextureShape, bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let texels = shape.width as usize * shape.height as usize * shape.layers as usize;
    let level_bytes = texels * shape.format.texel_bytes();
    let level = bytes.get(..level_bytes).ok_or_else(|| {
        format!(
            "载荷只有 {} 字节，最细那一级就要 {level_bytes}",
            bytes.len()
        )
    })?;
    let mut rgba = Vec::with_capacity(texels * 4);
    match shape.format {
        TextureFormat::Rgba8Srgb => rgba.extend_from_slice(level),
        TextureFormat::Rgba16Float => {
            for texel in level.chunks_exact(8) {
                for channel in 0..4 {
                    let bits = u16::from_le_bytes([texel[channel * 2], texel[channel * 2 + 1]]);
                    let value = half_to_f32(bits).clamp(0.0, 1.0);
                    rgba.push((value * 255.0 + 0.5) as u8);
                }
            }
        }
    }
    let height = shape.height * shape.layers;
    image::RgbaImage::from_raw(shape.width, height, rgba)
        .ok_or_else(|| format!("解出来的字节数与 {}×{} 对不上", shape.width, height))
}

pub fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1F);
    let mantissa = u32::from(bits & 0x03FF);
    let value = match exponent {
        0 => {
            if mantissa == 0 {
                sign
            } else {
                let mut mantissa = mantissa;
                let mut exponent = 127 - 15 + 1;
                while mantissa & 0x0400 == 0 {
                    mantissa <<= 1;
                    exponent -= 1;
                }
                sign | (exponent << 23) | ((mantissa & 0x03FF) << 13)
            }
        }
        0x1F => sign | 0x7F80_0000 | (mantissa << 13),
        _ => sign | ((exponent + 127 - 15) << 23) | (mantissa << 13),
    };
    f32::from_bits(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest;

    const SCENE_LIST: &str = "target/oracle/orbit-bare-nolight.txt";

    fn scene_path() -> Option<PathBuf> {
        let list = shader::workspace().join(SCENE_LIST);
        if !list.exists() {
            println!("⚠ 跳过：{} 不在（target/ 不入 git）", list.display());
            return None;
        }
        let text = std::fs::read_to_string(&list)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", list.display()));
        let path = PathBuf::from(text.trim());
        if !path.exists() {
            println!(
                "⚠ 跳过：场景产物 {} 不在（target/ 不入 git）",
                path.display()
            );
            return None;
        }
        Some(path)
    }

    fn loaded() -> Option<LoadedScene> {
        let path = scene_path()?;
        let root = default_pcg_root();
        if !root.exists() {
            println!("⚠ 跳过：CAS 根 {} 不在（target/ 不入 git）", root.display());
            return None;
        }
        Some(
            load_scene(&path, &root)
                .unwrap_or_else(|err| panic!("载入 {} 失败：{err}", path.display())),
        )
    }

    fn number(params: &BTreeMap<String, Value>, key: &str) -> f64 {
        match params.get(key) {
            Some(Value::Num(value)) => *value,
            other => panic!("参数 '{key}' 不是一个数：{other:?}"),
        }
    }

    fn same_mesh(one: &Mesh, other: &Mesh) -> bool {
        one.positions == other.positions
            && one.normals == other.normals
            && one.uvs == other.uvs
            && one.indices == other.indices
    }

    fn digest_of(mesh: &Mesh) -> String {
        let mut bytes = Vec::new();
        for position in &mesh.positions {
            for value in position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for normal in &mesh.normals {
            for value in normal {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for uv in &mesh.uvs {
            for value in uv {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in &mesh.indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        digest::sha256_hex(&bytes)[..16].to_uppercase()
    }

    #[test]
    fn the_orbit_bare_nolight_scene_resolves_member_by_member() {
        let Some(scene) = loaded() else {
            println!("⚠ 这一档判据没跑（见上面那行）：不是通过，是没测");
            return;
        };
        println!("{}", scene.audit());

        let ids: Vec<&str> = scene
            .objects
            .iter()
            .map(|object| object.id.as_str())
            .collect();
        assert_eq!(ids, vec!["planet", "atmosphere"], "物体表");
        assert_eq!(scene.ambient, 80.0, "环境光");

        let planet = scene.object("planet").expect("planet 在");
        assert_eq!(
            planet.geometry.member().expect("网格成员").to_string(),
            "planet/surface@f5d69bcdde79"
        );
        assert_eq!(
            planet.shader.member.to_string(),
            "shaders/surface@f679cdf81015"
        );
        assert_eq!(
            planet
                .texture(1)
                .expect("第 1 格贴图")
                .texture
                .member
                .to_string(),
            "generated/surface_color@94acc1920c4a"
        );
        assert_eq!(planet.alpha, AlphaMode::Opaque);
        assert_eq!(planet.cull, CullMode::Back);
        assert!(planet.cast_shadow);
        assert_eq!(planet.textures.len(), 1);
        assert_eq!(planet.geometry.mesh().vertex_count(), 155_526);
        assert_eq!(planet.geometry.mesh().triangle_count(), 307_200);
        assert_eq!(
            planet
                .texture(1)
                .expect("第 1 格")
                .texture
                .image
                .dimensions(),
            (780, 520)
        );
        assert_eq!(planet.texture(1).expect("第 1 格").texture.shape.levels, 10);
        assert_eq!(
            planet.texture(1).expect("第 1 格").texture.bytes.len(),
            2_162_808
        );
        assert_eq!(
            planet.params.len(),
            planet.shader.layout.params_bytes as usize
        );
        assert_eq!(planet.f32_at("gain"), Some(2.0));
        assert_eq!(planet.f32_at("coverage"), Some(0.3499999940395355));
        assert_eq!(planet.f32_at("height"), Some(0.5));
        println!(
            "planet｜网格 {}｜shader {}｜参数 {} 字节｜第 1 格 {}",
            planet.geometry.describe(),
            planet.shader.member,
            planet.params.len(),
            planet.texture(1).expect("第 1 格").texture.label()
        );

        let atmosphere = scene.object("atmosphere").expect("atmosphere 在");
        let (name, params) = atmosphere.geometry.primitive().expect("图元");
        assert_eq!(name, "icosphere");
        assert_eq!(number(params, "radius"), 1.1399999856948853);
        assert_eq!(number(params, "subdivisions"), 64.0);
        assert_eq!(
            atmosphere.shader.member.to_string(),
            "shaders/atmosphere@d4501946bb0c"
        );
        assert_eq!(atmosphere.alpha, AlphaMode::Add);
        assert_eq!(atmosphere.cull, CullMode::Back);
        assert!(!atmosphere.cast_shadow);
        assert!(atmosphere.textures.is_empty());
        assert_eq!(atmosphere.geometry.mesh().vertex_count(), 42_252);
        assert_eq!(atmosphere.geometry.mesh().triangle_count(), 84_500);
        println!(
            "atmosphere｜{}｜shader {}｜参数 {} 字节｜{:?}/{:?}",
            atmosphere.geometry.describe(),
            atmosphere.shader.member,
            atmosphere.params.len(),
            atmosphere.alpha,
            atmosphere.cull
        );

        let skybox = scene.skybox.as_ref().expect("天空盒");
        assert_eq!(
            skybox.texture.member.to_string(),
            "generated/stars@bc2ac082b43d"
        );
        assert_eq!(skybox.texture.shape.layers, 6);
        assert_eq!(skybox.texture.shape.levels, 1);
        assert_eq!(skybox.texture.image.dimensions(), (512, 3072));
        println!(
            "天空盒｜{}｜{}×{}×{} 层｜{} 级 mip｜{} 字节",
            skybox.texture.label(),
            skybox.texture.shape.width,
            skybox.texture.shape.height,
            skybox.texture.shape.layers,
            skybox.texture.shape.levels,
            skybox.texture.bytes.len()
        );
    }

    #[test]
    fn the_primitive_parameters_come_from_the_document() {
        let Some(scene) = loaded() else {
            println!("⚠ 这一档判据没跑（见上面那行）：不是通过，是没测");
            return;
        };
        let atmosphere = scene.object("atmosphere").expect("atmosphere 在");
        let (name, params) = atmosphere.geometry.primitive().expect("图元");
        let radius = number(params, "radius") as f32;
        assert_eq!(radius.to_bits(), 1.14_f32.to_bits());
        assert_eq!(number(params, "subdivisions"), 64.0);

        let from_document = primitive_mesh(name, params).expect("按文档再造一份");
        assert!(
            same_mesh(atmosphere.geometry.mesh(), &from_document),
            "物体里那份网格必须就是**文档里那几个参数**造出来的那一份"
        );
        assert_eq!(
            digest_of(&crate::icosphere::icosphere(1.0, 64)),
            "B4B37AB464C3A743",
            "摘要口径必须是「小端拼 positions → normals → uvs → indices」"
        );
        assert_eq!(
            digest_of(atmosphere.geometry.mesh()),
            digest_of(&crate::icosphere::icosphere(1.14, 64)),
            "文档里那个半径真的进了位置（1.14 那份与 1.0 那份不是同一网格）"
        );
        assert_ne!(digest_of(atmosphere.geometry.mesh()), "B4B37AB464C3A743");

        let mut bigger = params.clone();
        bigger.insert("radius".to_string(), Value::Num(7.5));
        let other = primitive_mesh(name, &bigger).expect("换个半径再造一份");
        assert!(
            !same_mesh(&other, atmosphere.geometry.mesh()),
            "半径换了网格必须跟着换 —— 说明它读的是文档里的数，不是写死的常数"
        );
        assert_eq!(
            other.vertex_count(),
            atmosphere.geometry.mesh().vertex_count()
        );
        let worst = other
            .positions
            .iter()
            .map(|position| {
                (position[0] * position[0] + position[1] * position[1] + position[2] * position[2])
                    .sqrt()
            })
            .fold(0.0_f32, f32::max);
        assert!(
            (worst - 7.5).abs() < 1e-4,
            "换过半径的那份外接半径是 {worst}"
        );

        let mut denser = params.clone();
        let dense = 8.0;
        denser.insert("subdivisions".to_string(), Value::Num(dense));
        let coarse = primitive_mesh(name, &denser).expect("换个细分再造一份");
        assert_eq!(
            coarse.vertex_count(),
            10 * (dense as usize + 1).pow(2) + 2,
            "细分改了顶点数就该按闭式 10(s+1)²+2 变"
        );
        assert_ne!(
            coarse.indices.len(),
            atmosphere.geometry.mesh().indices.len()
        );
    }

    #[test]
    fn the_base_level_decoder_handles_both_formats() {
        let shape = TextureShape {
            width: 2,
            height: 2,
            layers: 1,
            levels: 1,
            format: TextureFormat::Rgba8Srgb,
        };
        let bytes: Vec<u8> = (0..16).collect();
        let image = decode_base_level(&shape, &bytes).expect("8 位解码");
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.get_pixel(1, 1).0, [12, 13, 14, 15]);

        let float = TextureShape {
            width: 2,
            height: 1,
            layers: 1,
            levels: 1,
            format: TextureFormat::Rgba16Float,
        };
        let mut payload = Vec::new();
        for bits in [
            0x3C00u16, 0x3800, 0x0000, 0x4000, 0xBC00, 0x7C00, 0x0001, 0x3555,
        ] {
            payload.extend_from_slice(&bits.to_le_bytes());
        }
        assert_eq!(half_to_f32(0x3C00), 1.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0x4000), 2.0);
        assert_eq!(half_to_f32(0xBC00), -1.0);
        assert_eq!(half_to_f32(0x0000), 0.0);
        let image = decode_base_level(&float, &payload).expect("半精度解码");
        assert_eq!(image.dimensions(), (2, 1));
        assert_eq!(image.get_pixel(0, 0).0, [255, 128, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 255, 0, 85]);

        let cube = TextureShape {
            width: 2,
            height: 2,
            layers: 6,
            levels: 1,
            format: TextureFormat::Rgba8Srgb,
        };
        let image = decode_base_level(&cube, &vec![7_u8; 2 * 2 * 6 * 4]).expect("立方图解码");
        assert_eq!(image.dimensions(), (2, 12));
    }

    fn frame_material_fixture(entry: &str) -> px_protocol::scene::FrameMaterial {
        let path = shader::workspace().join("art/frame/skybox.wgsl");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        let mut params = BTreeMap::new();
        params.insert("brightness".to_string(), Value::Num(900.0));
        px_protocol::scene::FrameMaterial {
            name: "skybox".to_string(),
            shader: text,
            entry: entry.to_string(),
            params,
        }
    }

    #[test]
    fn the_frame_material_is_reflected_and_packed_like_a_content_material() {
        let declared = frame_material_fixture("fragment");
        let loaded = load_frame_material(&declared, &shader::modules()).expect("装载帧材质");

        assert_eq!(loaded.name, "skybox");
        assert_eq!(
            loaded.entry, "fragment",
            "入口名是文档给的那个（已核对它存在）"
        );
        assert_eq!(loaded.textures, vec![(5, TextureDimension::Cube)]);
        assert_eq!(loaded.params.len(), 16);
        assert_eq!(
            f32::from_le_bytes(loaded.params[..4].try_into().expect("四个字节")),
            900.0
        );
        assert!(
            loaded
                .assembled
                .contains("params.brightness * view.exposure"),
            "亮度那一格要乘上相机的曝光（oracle：`skybox.brightness * exposure`）"
        );
        assert_eq!(
            loaded.version,
            version_of(&crate::digest::sha256_hex(loaded.assembled.as_bytes())).expect("内容键")
        );
        println!(
            "帧自有材质 '{}'：entry {}｜组装后 {} 字节｜参数 {} 字节｜贴图格 {:?}",
            loaded.name,
            loaded.entry,
            loaded.assembled.len(),
            loaded.params.len(),
            loaded.textures
        );
    }

    #[test]
    fn a_frame_material_entry_that_does_not_exist_is_refused_by_name() {
        let declared = frame_material_fixture("fs_main");
        let err = load_frame_material(&declared, &shader::modules()).expect_err("指不到 ⇒ 拒");
        assert!(err.contains("fs_main"), "要点名那个指不到的名字：{err}");
        assert!(
            err.contains("fragment"),
            "要把那份 WGSL 实际的入口列出来：{err}"
        );
        assert_eq!(
            err.matches("fragment").count(),
            2,
            "片元那一行与「全部入口」那一行都要有：{err}"
        );
    }
}
