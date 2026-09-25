use std::path::{Path, PathBuf};

use px_pass::{
    Attachment, Cull, External, Frame, PassKind, Plan, ResolvedGeometry, ResolvedGroup,
    ResolvedMaterial, Role,
};
use px_protocol::material::MATERIAL_BIND_GROUP;
use px_protocol::scene::CullMode;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::art;
use crate::gpu::Gpu;
use crate::group0;
use crate::mat4::{Mat4, Quat, Vec3};
use crate::material::{self, FRAGMENT_ENTRY, Materials};
use crate::mesh::Mesh;
use crate::plan;
use crate::shader;
use crate::shot;

const ZERO_LAYOUT_ID: u64 = 0;
const STAGE_LAYOUT_ID: u64 = 1;
const MATERIAL_LAYOUT_ID: u64 = 2;

const PROCEDURAL_VERTICES: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaterialTable {
    Objects(usize),
    Frame(usize),
    Instance(usize),
}

impl MaterialTable {
    fn describe(self) -> String {
        match self {
            MaterialTable::Objects(index) => format!("物体 id 那张表（第 {index} 个物体）"),
            MaterialTable::Frame(index) => {
                format!("**帧自有材质**那张表（第 {index} 份，`frame_materials`）")
            }
            MaterialTable::Instance(index) => {
                format!("**生成的材质实例**那张表（第 {index} 份，`material_instances`）")
            }
        }
    }
}

fn material_table(
    spec: &px_protocol::scene::SceneSpec,
    name: &str,
) -> Result<MaterialTable, String> {
    let list = |names: Vec<&str>| -> String {
        if names.is_empty() {
            "（一个都没有）".to_string()
        } else {
            names.join(" / ")
        }
    };
    let objects = spec
        .objects
        .iter()
        .map(|object| object.id.as_str())
        .collect::<Vec<_>>();
    let frames = spec
        .frame_materials
        .iter()
        .map(|material| material.name.as_str())
        .collect::<Vec<_>>();
    let object = spec.objects.iter().position(|object| object.id == name);
    let frame = spec
        .frame_materials
        .iter()
        .position(|material| material.name == name);
    let instance = spec
        .material_instances
        .iter()
        .position(|instance| instance.name == name);
    match (object, frame, instance) {
        (Some(_), Some(_), _) => Err(format!(
            "材质名 '{name}' 在**两张表**里都有：物体 id [{}] 与帧自有材质 [{}]。\n  \
             同一个名字两处真本 ⇒ 宿主只能猜一个，而猜错是**一声不吭的错像素**。\n  \
             ⇒ 这是调用方要改的：物体 id 与帧材质名必须互不相同",
            list(objects),
            list(frames)
        )),
        (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => Err(format!(
            "材质名 '{name}' 既是真本（物体 id 或帧自有材质），又是生成的实例：\
             一个名字只能有一份真本"
        )),
        (Some(index), None, None) => Ok(MaterialTable::Objects(index)),
        (None, Some(index), None) => Ok(MaterialTable::Frame(index)),
        (None, None, Some(index)) => Ok(MaterialTable::Instance(index)),
        (None, None, None) => Err(format!(
            "材质名 '{name}' **三张表里都没有**。\n  \
             物体 id（它们的 id 就是材质名）：{}\n  帧自有材质（`frame_materials`）：{}\n  \
             生成的实例（`material_instances`）：{}",
            list(objects),
            list(frames),
            list(
                spec.material_instances
                    .iter()
                    .map(|instance| instance.name.as_str())
                    .collect()
            )
        )),
    }
}

fn skybox_slot(
    material: &art::LoadedFrameMaterial,
    skybox: Option<&art::Skybox>,
) -> Result<Option<u32>, String> {
    let at = format!("帧自有材质 '{}'", material.name);
    let slots = material
        .textures
        .iter()
        .map(|(binding, dimension)| format!("第 {binding} 格（{}）", dimension.name()))
        .collect::<Vec<_>>()
        .join(" / ");
    match (skybox, material.textures.as_slice()) {
        (None, []) => Ok(None),
        (None, _) => Err(format!(
            "{at} 声明了贴图格 [{}]，而这一帧的环境里**没有天空盒**（`environment.skybox` 是空的）\
             ：帧自有材质的贴图只有环境那一张来源，没有天空盒时这一格无处可绑",
            slots
        )),
        (Some(skybox), [(binding, dimension)]) => {
            let layers = skybox.texture.shape.layers;
            if dimension.layers() != layers {
                return Err(format!(
                    "{at} 在第 {binding} 格声明的是 {}（{} 层），而环境里那份天空盒 \
                     {} 是 {} 层 —— 同一张图在两边不是同一种东西",
                    dimension.name(),
                    dimension.layers(),
                    skybox.texture.label(),
                    layers
                ));
            }
            Ok(Some(*binding))
        }
        (Some(_), []) => Err(format!(
            "{at} 一格贴图都没声明，而这一帧的环境里有天空盒（`environment.skybox`）——\
             那份天空盒没有去处。要么这份 WGSL 少了一个 texture 声明，要么这一帧不该带天空盒"
        )),
        (Some(_), _) => Err(format!(
            "{at} 声明了 {} 格贴图（{}），而文档的 `frame_materials` 里**没有**\
             \"哪一格是哪张图\"这一栏：天空盒该落在哪一格没有任何依据。\
             ⚠ 不许挑一个 —— 要么这份 WGSL 只声明一格，要么先把那一栏加进契约",
            material.textures.len(),
            slots
        )),
    }
}

const DEPTH_RESOURCES: [&str; 2] = ["scene_depth", "scene_depth_sample"];

const SHADOW_TEXTURE_RESOURCES: [&str; 4] = [
    "point_shadow_atlas",
    "point_shadow_atlas_l1",
    "point_shadow_atlas_l2",
    "point_shadow_atlas_l3",
];

#[allow(dead_code)]
const SHADOW_ATLAS_WRITTEN: &str = "point_shadow_atlas";

pub struct Rendered {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub audit: Vec<String>,
    pub executed: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub declared_clouds: bool,
    pub timestamp_calls: u32,
}

struct Geometry {
    name: String,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    attributes: Vec<wgpu::VertexAttribute>,
    vertex_count: u32,
    index_count: u32,
}

impl Geometry {
    fn procedural(&self) -> bool {
        self.vertices.is_none()
    }

    fn layout(&self) -> wgpu::VertexBufferLayout<'_> {
        wgpu::VertexBufferLayout {
            array_stride: u64::from(Mesh::stride()),
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &self.attributes,
        }
    }
}

struct Stage {
    view_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Stage {
    fn set_view(&self, queue: &wgpu::Queue, camera: &crate::camera::Camera) {
        queue.write_buffer(&self.view_buffer, 0, &view_bytes(camera));
    }
}

struct Face {
    light: u32,
    face: u32,
    camera: crate::camera::Camera,
}

fn instance_bytes(transform: &px_protocol::scene::Transform) -> [u8; 112] {
    let rotation = Quat::from_xyzw(
        transform.rotation[0],
        transform.rotation[1],
        transform.rotation[2],
        transform.rotation[3],
    );
    let world_from_local = Mat4::from_scale_rotation_translation(
        Vec3::new(transform.scale[0], transform.scale[1], transform.scale[2]),
        rotation,
        Vec3::new(
            transform.translation[0],
            transform.translation[1],
            transform.translation[2],
        ),
    );
    let mut bytes = [0_u8; 112];
    let normal = world_from_local.normal_matrix_3x3();
    for (index, column) in [
        world_from_local.x_axis,
        world_from_local.y_axis,
        world_from_local.z_axis,
        world_from_local.w_axis,
    ]
    .iter()
    .enumerate()
    {
        for (slot, value) in [column.x, column.y, column.z, column.w].iter().enumerate() {
            let at = index * 16 + slot * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    for (index, column) in normal.iter().enumerate() {
        for (slot, value) in [column.x, column.y, column.z, 0.0].iter().enumerate() {
            let at = 64 + index * 16 + slot * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn view_bytes(camera: &crate::camera::Camera) -> [u8; 64] {
    let mut bytes = [0_u8; 64];
    for (index, column) in [
        camera.clip_from_world.x_axis,
        camera.clip_from_world.y_axis,
        camera.clip_from_world.z_axis,
        camera.clip_from_world.w_axis,
    ]
    .iter()
    .enumerate()
    {
        for (slot, value) in [column.x, column.y, column.z, column.w].iter().enumerate() {
            let at = index * 16 + slot * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn cull_of(cull: CullMode) -> Cull {
    match cull {
        CullMode::Back => Cull::Back,
        CullMode::Front => Cull::Front,
        CullMode::None => Cull::None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Views {
    Single(Option<[f32; 3]>),
    Sheet { columns: u32 },
}

struct Placement {
    camera: crate::camera::Camera,
    rect: Option<[u32; 4]>,
    index: usize,
    note: String,
}

impl Placement {
    fn viewport(&self) -> Option<[f32; 4]> {
        self.rect.map(|rect| {
            [
                rect[0] as f32,
                rect[1] as f32,
                rect[2] as f32,
                rect[3] as f32,
            ]
        })
    }

    fn uniform_viewport(&self, cell: (u32, u32)) -> [f32; 4] {
        let rect = self.rect.unwrap_or([0, 0, cell.0, cell.1]);
        [
            rect[0] as f32,
            rect[1] as f32,
            rect[2] as f32,
            rect[3] as f32,
        ]
    }
}

fn placements(
    views: Views,
    spec: &px_protocol::scene::SceneSpec,
    width: u32,
    height: u32,
) -> Result<(Vec<Placement>, (u32, u32)), String> {
    let aspect = width as f32 / height as f32;
    match views {
        Views::Single(cam) => Ok((
            vec![Placement {
                camera: crate::camera::probe_camera(cam, aspect),
                rect: None,
                index: 0,
                note: match cam {
                    Some([yaw, pitch, distance]) => {
                        format!("--cam {yaw},{pitch},{distance}")
                    }
                    None => "探针机位（不给 --cam）".to_string(),
                },
            }],
            (width, height),
        )),
        Views::Sheet { columns } => {
            if spec.cameras.is_empty() {
                return Err("--sheet 用的是产物自带的相机表：这一步没带（烘场景时用 \
                     `px_scene::cameras::review()` 写进场景文档的 `cameras`）"
                    .to_string());
            }
            let columns = columns.max(1);
            let rows = (spec.cameras.len() as u32).div_ceil(columns);
            let mut out = Vec::with_capacity(spec.cameras.len());
            for (index, camera) in spec.cameras.iter().enumerate() {
                let column = index as u32 % columns;
                let row = index as u32 / columns;
                out.push(Placement {
                    camera: crate::camera::review_camera(camera, aspect),
                    rect: Some([column * width, row * height, width, height]),
                    index,
                    note: format!(
                        "第 {} 格（{columns} 列排开的第 {row} 行第 {column} 列）｜产物相机 '{}'",
                        index, camera.tag
                    ),
                });
            }
            Ok((out, (width * columns, height * rows)))
        }
    }
}

pub fn run(
    gpu: &Gpu,
    scene_path: &Path,
    pcg_root: &Path,
    views: Views,
    width: u32,
    height: u32,
) -> Result<Rendered, String> {
    let mut session = Session::open(gpu, scene_path, pcg_root, views, width, height)?;
    session.draw(gpu, views, width, height)
}

pub struct Session {
    spec: px_protocol::scene::SceneSpec,
    scene: art::LoadedScene,
    executed_plan: Plan,
    executor: px_pass::Executor,
    geometries: Vec<Geometry>,
    bindings: Vec<material::MaterialBinding>,
    frame_materials: Vec<(art::LoadedFrameMaterial, material::MaterialBinding)>,
    faces: Vec<Face>,
    face_stages: Vec<Stage>,
    stage_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    materials: Materials,
    #[allow(dead_code)]
    instance_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    shadow_view: wgpu::TextureView,
    #[allow(dead_code)]
    shadow_sampler: wgpu::Sampler,
    #[allow(dead_code)]
    shadow_probe: Vec<Option<wgpu::Texture>>,
    scene_path: std::path::PathBuf,
    pcg_root: std::path::PathBuf,
    shader_slots: Vec<ShaderSlot>,
    shader_notes: Vec<String>,
    audit_head: Vec<String>,
    audit_rest: Vec<String>,
    loads_first: Vec<String>,
    layer: Option<Layer>,
}

struct Layer {
    target: (u32, u32),
    rects: Vec<Option<[u32; 4]>>,
    host_target: shot::Target,
    cells: Vec<Cell>,
    used: bool,
}

impl Layer {
    fn matches(&self, target: (u32, u32), placements: &[Placement]) -> bool {
        self.target == target
            && self.rects.len() == placements.len()
            && self
                .rects
                .iter()
                .zip(placements.iter())
                .all(|(rect, placement)| *rect == placement.rect)
    }
}

struct Cell {
    zero: group0::GroupZero,
    zero_dummy: group0::GroupZero,
    stage: Stage,
}

impl Cell {
    fn set_view(
        &self,
        queue: &wgpu::Queue,
        camera: &crate::camera::Camera,
        viewport: [f32; 4],
    ) -> String {
        let line = self.zero.set_view(queue, camera, viewport);
        self.stage.set_view(queue, camera);
        line
    }
}

#[derive(Debug, Clone)]
pub struct ShaderSlot {
    pub what: String,
    pub path: PathBuf,
    place: SlotPlace,
    pub version: u64,
    pub member: Option<String>,
    pub proven: bool,
    pub disk: String,
    pub source_hash: String,
    pub difference: String,
    pub text_hash: String,
    pub closure: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotPlace {
    Object(usize),
    Pass(usize),
    FrameMaterial(usize),
}

impl ShaderSlot {
    #[allow(clippy::too_many_arguments)]
    fn new(
        what: String,
        path: PathBuf,
        place: SlotPlace,
        assembled: &str,
        member: Option<String>,
        source: &str,
        disk: &str,
        modules: &px_shader::ModuleTable,
    ) -> ShaderSlot {
        let proven = source.replace("\r\n", "\n") == disk.replace("\r\n", "\n");
        ShaderSlot {
            what,
            path,
            place,
            version: text_version(assembled),
            member,
            proven,
            disk: sha16(disk),
            source_hash: sha16(source),
            difference: if proven {
                String::new()
            } else {
                first_difference(source, disk)
            },
            text_hash: sha16(source),
            closure: px_shader::closure(source, modules).fingerprint(),
        }
    }
}

fn text_version(assembled: &str) -> u64 {
    material::version_of(&crate::digest::sha256_hex(assembled.as_bytes()))
        .expect("sha256 的输出一定是 64 位十六进制")
}

fn sha16(text: &str) -> String {
    crate::digest::sha256_hex(text.as_bytes())[..16].to_string()
}

fn first_difference(left: &str, right: &str) -> String {
    let left = left.replace("\r\n", "\n");
    let right = right.replace("\r\n", "\n");
    let left_lines: Vec<&str> = left.lines().collect();
    let right_lines: Vec<&str> = right.lines().collect();
    for index in 0..left_lines.len().max(right_lines.len()) {
        let a = left_lines.get(index).copied().unwrap_or("（没有这一行）");
        let b = right_lines.get(index).copied().unwrap_or("（没有这一行）");
        if a != b {
            let clip = |text: &str| -> String {
                let mut out: String = text.chars().take(120).collect();
                if text.chars().count() > 120 {
                    out.push('…');
                }
                out
            };
            return format!(
                "第 {} 行起不同：\n    文档那份：{}\n    盘上那份：{}",
                index + 1,
                clip(a),
                clip(b)
            );
        }
    }
    "逐行相同（只差行尾：那一栏已经被归一过了）".to_string()
}

fn object_params(
    spec: &px_protocol::scene::SceneSpec,
    id: &str,
) -> std::collections::BTreeMap<String, px_protocol::Value> {
    spec.objects
        .iter()
        .find(|object| object.id == id)
        .map(|object| object.material.params.clone())
        .unwrap_or_default()
}

#[derive(Debug, Default)]
pub struct ReloadReport {
    pub changed_files: Vec<PathBuf>,
    pub reloaded: Vec<SlotChange>,
    pub refused: Vec<String>,
    pub notes: Vec<String>,
    pub skipped: usize,
    pub untouched: Untouched,
    pub millis: f64,
}

#[derive(Debug, Clone)]
pub struct SlotChange {
    pub what: String,
    pub path: PathBuf,
    pub text_hash: String,
    pub old: u64,
    pub new: u64,
    pub key: Option<(u64, u64)>,
    pub member: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Untouched {
    pub objects: usize,
    pub geometries: usize,
    pub textures: usize,
    pub params_bytes: usize,
    pub instances: usize,
    pub cells: usize,
}

impl ReloadReport {
    pub fn readout(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let files: Vec<String> = self
            .changed_files
            .iter()
            .map(|path| {
                path.strip_prefix(shader::workspace())
                    .unwrap_or(path)
                    .display()
                    .to_string()
            })
            .collect();
        if self.reloaded.is_empty() && self.refused.is_empty() {
            out.push(format!(
                "—— 热重载：盘上变了 {} 个文件（{}），但**没有哪一槽的文本跟着变** ⇒ 什么都没作废\
                 （跳过 {} 槽：原文与闭包都没变）——",
                files.len(),
                files.join(" / "),
                self.skipped
            ));
        } else {
            out.push(format!(
                "—— 热重载：盘上变了 {} 个文件（{}）｜作废 {} 槽｜跳过 {} 槽（原文与闭包都没变）\
                 ｜拒 {} 槽 ——",
                files.len(),
                files.join(" / "),
                self.reloaded.len(),
                self.skipped,
                self.refused.len()
            ));
        }
        for change in &self.reloaded {
            let key = match change.key {
                Some((before, after)) => {
                    format!("｜材质的管线键 shader {before:#018x} → {after:#018x}")
                }
                None => String::new(),
            };
            out.push(format!(
                "  {}｜{}｜原文 {}｜组装后 {:016x} → {:016x}{key}｜重载本身 {:.1} ms",
                change.what,
                change
                    .path
                    .strip_prefix(shader::workspace())
                    .unwrap_or(&change.path)
                    .display(),
                change.text_hash,
                change.old,
                change.new,
                self.millis
            ));
            if let Some(member) = &change.member {
                out.push(format!(
                    "    ⚠ 从这一刻起它跑的是**盘上**的文本（键 {:016x}），文档指的那个成员还是 {member}\
                     —— 热重载的语义就是这件事；要回到「文档说了算」就重烘再装载",
                    change.new
                ));
            }
        }
        for why in &self.refused {
            out.push(format!("  ✗ 拒：{why}"));
        }
        for note in &self.notes {
            out.push(format!("  · {note}"));
        }
        if !self.reloaded.is_empty() {
            out.push(format!(
                "  没动的层（计数）：物体 {} / 几何 {} / 贴图 {} 张 / 参数 {} 字节 / 实例数组 {} 份 / {} 格组\
                 ｜执行器（管线缓存与池子、seed 进去的那几张图）**同一个**",
                self.untouched.objects,
                self.untouched.geometries,
                self.untouched.textures,
                self.untouched.params_bytes,
                self.untouched.instances,
                self.untouched.cells
            ));
        }
        out
    }
}

impl Session {
    pub fn open(
        gpu: &Gpu,
        scene_path: &Path,
        pcg_root: &Path,
        views: Views,
        width: u32,
        height: u32,
    ) -> Result<Session, String> {
        let root = pcg_root.to_path_buf();
        let spec = art::read_spec(scene_path)?;
        let scene = art::load_scene(scene_path, &root)?;
        let mut audit: Vec<String> = Vec::new();
        audit.push(scene.audit());

        let plan = plan::build(&spec, &root)?;
        audit.push(format!(
            "计划：{} 个资源 / {} 条 pass（全部解析自文档，`px_pass::Plan::check()` 已过）",
            plan.resources.len(),
            plan.passes.len()
        ));
        for (index, pass) in plan.passes.iter().enumerate() {
            audit.push(format!(
                "  [{index}] '{}'｜{}｜写 [{}]｜深度目标 {}｜读 [{}]｜draws {}｜状态 {}",
                pass.label,
                pass.kind.name(),
                pass.writes.join(" / "),
                pass.depth_target.as_deref().unwrap_or("（不挂）"),
                pass.reads.join(" / "),
                pass.draws
                    .iter()
                    .map(|draw| if draw.material.is_empty() {
                        draw.geometry.clone()
                    } else {
                        format!("{}+{}", draw.geometry, draw.material)
                    })
                    .collect::<Vec<_>>()
                    .join(" / "),
                pass.render.name()
            ));
        }
        let mut executed_plan = all_passes(&plan, &mut audit)?;

        let loads_first: Vec<String> = loads_before_write(&executed_plan);
        if !loads_first.is_empty() {
            audit.push(format!(
                "⚠ 这一份计划里 [{}] 的**第一笔写就是 `load`**（没有任何一条更早的 pass 写过它）：\
             池子跨帧复用给的是**上一帧**的内容，而一次性那条路给的是清零的纹理 ⇒ \
             这一份**不开保留**（每一帧重新准备）。这不是「宿主偷懒」：要保留它，就得给执行器\
             一条「清空池子」的路（那是另一个单元的事）",
                loads_first.join(" / ")
            ));
        }

        let audit_head = std::mem::take(&mut audit);
        let (placements, target) = placements(views, &spec, width, height)?;

        let cluster = group0::lights_of(&spec.lights)?;
        audit.push(format!(
        "灯：文档 {} 盏 → 聚类缓冲前 {} 格（次序 = oracle 的排序键：开影子的在前，同档按文档次序）｜{}",
        spec.lights.len(),
        cluster.len(),
        if spec.lights.is_empty() {
            "（一盏都没有 ⇒ 整块全零 ⇒ 内容 shader 判 lit = false）".to_string()
        } else {
            spec.lights
                .iter()
                .map(|light| format!("{}（{:?}，影子 {}）", light.id, light.kind, light.shadows))
                .collect::<Vec<_>>()
                .join(" / ")
        }
    ));

        let mut executor = px_pass::Executor::new();

        let shadow_faces = {
            let mut bytes = Vec::with_capacity(18 * 16);
            let faces: Vec<[f32; 3]> = match &scene.shadow {
                Some(plan) if plan.faces.len() == 18 => plan.faces.clone(),
                _ => px_protocol::scene::SHADOW_FACE_BASIS
                    .iter()
                    .flat_map(|face| face.iter().copied())
                    .collect(),
            };
            for vector in &faces {
                bytes.extend_from_slice(&vector[0].to_le_bytes());
                bytes.extend_from_slice(&vector[1].to_le_bytes());
                bytes.extend_from_slice(&vector[2].to_le_bytes());
                bytes.extend_from_slice(&0.0_f32.to_le_bytes());
            }
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("组 0：立方体六面的基（文档给，见 SHADOW_FACE_BASIS）"),
                    usage: wgpu::BufferUsages::STORAGE,
                    contents: &bytes,
                })
        };
        let shadow_sampler = group0::point_shadow_sampler(&gpu.device);
        let (shadow_textures, shadow_views, shadow_note) = {
            let mut textures: Vec<Option<wgpu::Texture>> = Vec::with_capacity(4);
            let mut views: Vec<wgpu::TextureView> = Vec::with_capacity(4);
            let mut notes: Vec<String> = Vec::with_capacity(4);
            for name in SHADOW_TEXTURE_RESOURCES {
                let (texture, view, note) = match plan.resource(name) {
                    Some(resource) => {
                        let (cube_width, cube_height) = resource.size.resolve(target.0, target.1);
                        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some(resource.name.as_str()),
                            size: wgpu::Extent3d {
                                width: cube_width,
                                height: cube_height,
                                depth_or_array_layers: resource.layers,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Depth32Float,
                            usage: px_pass::texture_usage(resource) | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        });
                        executor.seed(resource, cube_width, cube_height, texture.clone())?;
                        let view = texture.create_view(&wgpu::TextureViewDescriptor {
                            label: Some("点光虚拟影图 atlas（D2Array/DepthOnly）"),
                            format: None,
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            usage: None,
                            aspect: wgpu::TextureAspect::DepthOnly,
                            base_mip_level: 0,
                            mip_level_count: None,
                            base_array_layer: 0,
                            array_layer_count: Some(resource.layers),
                        });
                        let note = format!(
                            "文档烘的那份（{cube_width}×{cube_height} × {} 层，每面一层，层号 = 灯×6 + 面）",
                            resource.layers,
                        );
                        audit.push(format!(
                            "  ⚠ 池子里的 '{name}' 现在就是**宿主建的**这一张\
                 （{cube_width}×{cube_height} × {} 层）：每一页各挂它的**单层**视图 + 一格 \
                 viewport，group 0 第 2 格挂整份的 **D2Array** 视图",
                            resource.layers
                        ));
                        (Some(texture), view, note)
                    }
                    None => {
                        let (texture, view) = group0::fallback_cube(&gpu.device);
                        audit.push(format!(
                            "⚠ 文档里没有 '{name}'：这一帧**没有投影的点光**\
                 （帧图把那份资源与那六条 pass 一起跳过了）⇒ group 0 第 2 格绑**兜底**的\
                 1×1×6 全 0 cube。它永远不会被采到（内容 shader 只在 `shadow_maps` 那一位\
                 立起来时才进 `fetch_point_shadow`，而那个位来自 `flags`）"
                        ));
                        (
                            Some(texture),
                            view,
                            "**兜底**的 1×1×6 全 0 cube（这一帧没有投影的点光）".to_string(),
                        )
                    }
                };
                textures.push(texture);
                views.push(view);
                notes.push(note);
            }
            (textures, views, notes.join("｜"))
        };
        let shadow_view_refs: [&wgpu::TextureView; 4] = [
            &shadow_views[0],
            &shadow_views[1],
            &shadow_views[2],
            &shadow_views[3],
        ];
        let shadow_texture = shadow_textures.first().cloned().flatten();
        let _shadow_texture = shadow_texture.clone();

        let contract = scene
            .objects
            .first()
            .ok_or_else(|| "场景里一个物体都没有：拿不到一份可反射的 shader".to_string())?;
        let contract_module = shader::validate(
            &format!("{}（group 0 契约）", contract.id),
            &contract.shader.assembled,
        )?;
        let shadow_page_table = {
            let contents: &[u8] = match &scene.shadow {
                Some(plan) if !plan.table.is_empty() => plan.table.as_slice(),
                _ => &[0_u8; 4],
            };
            if std::env::var_os("PX_AUDIT_SHADOW").is_some() && contents.len() >= 4 {
                let head = u32::from_le_bytes([contents[0], contents[1], contents[2], contents[3]]);
                let levels = if contents.len() >= 8 {
                    u32::from_le_bytes([contents[4], contents[5], contents[6], contents[7]])
                        & 0xFFFF
                } else {
                    0
                };
                let prefix: Vec<u32> = (0..levels as usize)
                    .filter_map(|j| {
                        let at = 8 + j * 4;
                        (contents.len() >= at + 4).then(|| {
                            u32::from_le_bytes([
                                contents[at],
                                contents[at + 1],
                                contents[at + 2],
                                contents[at + 3],
                            ])
                        })
                    })
                    .collect();
                println!(
                    "影子页表头：pages_per_side={}｜atlas 页格={} 个/边｜levels={}｜前缀表={:?}｜\
                     （atlas {}² ⇒ 页格 {} 个/边）",
                    head & 0xFFFF,
                    head >> 16,
                    levels,
                    prefix,
                    scene
                        .shadow
                        .as_ref()
                        .map(|plan| plan.atlas_side)
                        .unwrap_or(0),
                    scene
                        .shadow
                        .as_ref()
                        .map(|plan| plan.atlas_side / 128)
                        .unwrap_or(0),
                );
            }
            audit.push(match &scene.shadow {
                Some(plan) if !plan.table.is_empty() => format!(
                    "虚拟影图页表：{} 字节（{} 盏灯，段起点 {:?}）｜采样侧按\
                     「段起点 + 行基址 + 掩码 popcount 排名」查页｜atlas {} 层 × {}²",
                    plan.table.len(),
                    plan.light_offsets.len(),
                    plan.light_offsets,
                    plan.layers,
                    plan.atlas_side,
                ),
                _ => {
                    "虚拟影图页表：**空**（这一份产物没有投影的灯）⇒ 采样侧一格都查不到".to_string()
                }
            });
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("组 0：虚拟影图页表（烘图侧算，见 px-scene::vshadow）"),
                    usage: wgpu::BufferUsages::STORAGE,
                    contents,
                })
        };
        let shadow_page_offsets = {
            let mut bytes = Vec::new();
            for offset in scene
                .shadow
                .as_ref()
                .map(|plan| plan.light_offsets.as_slice())
                .unwrap_or_default()
            {
                bytes.extend_from_slice(&offset.to_le_bytes());
            }
            if bytes.is_empty() {
                bytes.extend_from_slice(&0_u32.to_le_bytes());
            }
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("组 0：页表段起点（每盏灯一个 u32）"),
                    usage: wgpu::BufferUsages::STORAGE,
                    contents: &bytes,
                })
        };

        let build_zero = |camera: &crate::camera::Camera,
                          view: &wgpu::TextureView,
                          atlas_views: &[&wgpu::TextureView],
                          viewport: [f32; 4],
                          mesh_instances: &wgpu::Buffer,
                          shadow_page_offsets: &wgpu::Buffer,
                          shadow_faces: &wgpu::Buffer|
         -> Result<group0::GroupZero, String> {
            group0::frame(
                &gpu.device,
                &contract_module,
                camera,
                scene.ambient,
                &cluster,
                viewport,
                view,
                atlas_views,
                &shadow_sampler,
                &shadow_page_table,
                mesh_instances,
                &shadow_page_offsets,
                &shadow_faces,
                &shadow_note,
            )
        };

        let mut seeded: Vec<(String, wgpu::Texture)> = Vec::new();
        for name in DEPTH_RESOURCES {
            let Some(resource) = plan.resource(name) else {
                return Err(format!(
                    "文档里没有资源 '{name}'（声明了的：{}）：这一档要把它 seed 成宿主建的那张深度图",
                    plan.resources
                        .iter()
                        .map(|resource| resource.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ));
            };
            let (depth_width, depth_height) = resource.size.resolve(target.0, target.1);
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(name),
                size: wgpu::Extent3d {
                    width: depth_width,
                    height: depth_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: px_pass::texture_usage(resource),
                view_formats: &[],
            });
            executor.seed(resource, depth_width, depth_height, texture.clone())?;
            audit.push(format!(
                "  ⚠ 池子里的 '{name}' 现在就是**宿主建的**这一张（{}×{} {:?}，用途照文档声明）：\
             谁被顶掉了要看得见 —— 不 seed 的话 copy 与绑定会各拿一张同名纹理，\
             而那种错法是**一声不吭的错像素**",
                depth_width,
                depth_height,
                texture.format()
            ));
            seeded.push((name.to_string(), texture));
        }
        let depth_texture = |name: &str| -> Option<&wgpu::Texture> {
            seeded
                .iter()
                .find(|(seeded_name, _)| seeded_name == name)
                .map(|(_, texture)| texture)
        };
        let depth_target_name = executed_plan
            .passes
            .iter()
            .find_map(|pass| match pass.render.depth {
                Attachment::None => None,
                _ => pass.depth_target.as_deref(),
            })
            .ok_or_else(|| {
                "这一档的计划里没有一条 pass 挂深度附件：group 0 的第 20 格就没有来源了".to_string()
            })?;
        let sampled_depth = executed_plan
            .passes
            .iter()
            .find(|pass| {
                pass.kind == PassKind::Copy
                    && pass.reads.first().map(String::as_str) == Some(depth_target_name)
            })
            .and_then(|pass| pass.writes.first().cloned())
            .unwrap_or_else(|| depth_target_name.to_string());
        let sampled_view = depth_texture(&sampled_depth)
            .ok_or_else(|| {
                format!(
                    "group 0 第 20 格该绑 '{sampled_depth}'，而宿主这一档只 seed 了 [{}]",
                    seeded
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                )
            })?
            .create_view(&wgpu::TextureViewDescriptor::default());
        audit.push(format!(
            "group 0 第 20 格（depth_prepass_texture）⇒ '{sampled_depth}'（从帧表推出来的：\
         深度附件用 '{depth_target_name}'，而那条 copy 把它搬成了 '{sampled_depth}'）"
        ));

        for pass in &executed_plan.passes {
            if pass.render.depth == Attachment::None {
                continue;
            }
            if pass.depth_target.as_deref() != Some(sampled_depth.as_str()) {
                continue;
            }
            return Err(format!(
                "pass '{}' 把 '{sampled_depth}' 当深度附件挂着，而 group 0 的第 20 格\
             （depth_prepass_texture）绑的**也是它**：wgpu 不许同一条 pass 里把一张图既当附件、\
             又当资源绑进绑定组（实测：`TextureUses(DEPTH_STENCIL_WRITE) is an exclusive usage \
             and cannot be used with any other usages within the usage scope`）。\
             三条出路：① 帧表里**先拷贝一次**（深度 → 快照，采样那一方读快照；这就是这一档的形状）；\
             ② 那条 pass 不挂深度附件（丢掉遮挡测试）；③ 让执行器按**每一条 pass** 解析材质\
             （现在 `Frame::materials` 是按名字查的一张平表）。⚠ 拷贝那一步属于帧表，宿主不自己插",
                pass.label
            ));
        }
        audit.push(
        "group 0（七格全绑，哪怕 shader 只声明了一部分）：**每格一份**（相机与 `view.viewport` \
         都在里面），逐格建在下面那一段"
            .to_string(),
    );

        let face_keys: Vec<(u32, u32)> = {
            let mut keys: Vec<(u32, u32)> = executed_plan
                .passes
                .iter()
                .filter_map(|pass| {
                    spec.passes
                        .iter()
                        .enumerate()
                        .find(|(index, document)| document.label_or(*index) == pass.label)
                        .and_then(|(_, document)| document.cube_face)
                        .map(|cube| (cube.light, cube.face))
                })
                .collect();
            keys.sort_unstable();
            keys.dedup();
            keys
        };
        let mut faces: Vec<Face> = Vec::with_capacity(face_keys.len());
        for (light, face) in face_keys {
            let clustered = cluster.get(light as usize).ok_or_else(|| {
                format!(
                    "文档里有一条 cube_face 指着第 {light} 个 cube，而聚类缓冲只有 {} 格",
                    cluster.len()
                )
            })?;
            let light_position = Vec3::new(
                clustered.position_radius[0],
                clustered.position_radius[1],
                clustered.position_radius[2],
            );
            let face_camera = crate::camera::face_view(
                light_position,
                face,
                group0::POINT_LIGHT_SHADOW_MAP_NEAR_Z,
            );
            let _ = &face_camera;
            let layer = light * group0::SHADOW_CUBE_FACES + face;
            audit.push(format!(
            "  面 (灯 {light}, 面 {face}, 层 {layer})：world_from_view 的平移 ({:.6}, {:.6}, {:.6})｜
             clip_from_world 的 w 列 ({:.9e}, {:.9e}, {:.9e}, {:.9e})",
            face_camera.world_from_view.w_axis.x,
            face_camera.world_from_view.w_axis.y,
            face_camera.world_from_view.w_axis.z,
            face_camera.clip_from_world.w_axis.x,
            face_camera.clip_from_world.w_axis.y,
            face_camera.clip_from_world.w_axis.z,
            face_camera.clip_from_world.w_axis.w
        ));
            faces.push(Face {
                light,
                face,
                camera: face_camera,
            });
        }

        let mut geometries: Vec<Geometry> = Vec::with_capacity(scene.objects.len());
        for object in &scene.objects {
            let mesh = object.geometry.mesh();
            let bytes = mesh.interleaved().map_err(|err| {
                format!(
                    "物体 '{}'（{}）的顶点流：{err}",
                    object.id,
                    object.geometry.describe()
                )
            })?;
            let index_bytes: Vec<u8> = mesh
                .indices
                .iter()
                .flat_map(|index| index.to_le_bytes())
                .collect();
            geometries.push(Geometry {
                name: object.id.clone(),
                vertices: Some(gpu.device.create_buffer_init(&BufferInitDescriptor {
                    label: Some("顶点（交错，布局来自产物属性表）"),
                    usage: wgpu::BufferUsages::VERTEX,
                    contents: &bytes,
                })),
                indices: Some(gpu.device.create_buffer_init(&BufferInitDescriptor {
                    label: Some("索引"),
                    usage: wgpu::BufferUsages::INDEX,
                    contents: &index_bytes,
                })),
                attributes: Mesh::vertex_attributes(),
                vertex_count: mesh.vertex_count() as u32,
                index_count: mesh.indices.len() as u32,
            });
            audit.push(format!(
            "几何 '{}'：{} 顶点 / {} 三角形｜顶点流 {} 字节（stride {}，属性 {:?}）｜索引 {} 字节",
            object.id,
            mesh.vertex_count(),
            mesh.triangle_count(),
            bytes.len(),
            Mesh::stride(),
            Mesh::attributes()
                .iter()
                .map(|(location, format, from)| format!("@{location}={format:?}（{from}）"))
                .collect::<Vec<_>>(),
            index_bytes.len()
        ));
        }

        for pass in &executed_plan.passes {
            if pass.kind != PassKind::Geometry {
                continue;
            }
            let module = shader::validate(
                &format!("pass '{}' 的顶点阶段", pass.label),
                &pass.vertex_shader,
            )?;
            let inputs = vertex_inputs(&module, &pass.vertex_entry, &pass.label)?;
            let procedural = inputs.is_empty();
            for draw in &pass.draws {
                let known = geometries
                    .iter()
                    .position(|geometry| geometry.name == draw.geometry);
                let geometry = match (known, procedural) {
                    (Some(index), false) => &geometries[index],
                    (Some(_), true) => {
                        return Err(format!(
                            "pass '{}' 的顶点阶段（{}）一个 `@location` 都不读（顶点全由 \
                         `vertex_index` 现算），而这一笔点的几何 '{}' 是**物体的网格**：\
                         那份顶点缓冲不会被用到 —— 要么这段顶点阶段写错了，要么这一笔画错了",
                            pass.label, pass.vertex_entry, draw.geometry
                        ));
                    }
                    (None, true) => {
                        geometries.push(procedural_geometry(draw.geometry.clone()));
                        audit.push(format!(
                            "几何 '{}'：**程序化**（没有顶点缓冲；{} 个顶点由顶点阶段按 \
                         `vertex_index` 现算 —— pass '{}' 的顶点阶段一个 `@location` 都不读）",
                            draw.geometry, PROCEDURAL_VERTICES, pass.label
                        ));
                        geometries.last().expect("刚 push 的")
                    }
                    (None, false) => {
                        return Err(format!(
                            "pass '{}' 要画几何 '{}'，而宿主这一档没给这个名字（给了：{}）",
                            pass.label,
                            draw.geometry,
                            geometries
                                .iter()
                                .map(|geometry| geometry.name.as_str())
                                .collect::<Vec<_>>()
                                .join(" / ")
                        ));
                    }
                };
                if !procedural {
                    check_vertex_inputs(&inputs, &pass.label, geometry)?;
                }
            }
            audit.push(format!(
                "顶点阶段（pass '{}'）的输入与产物属性表对得上：{}（声明的输入 {:?}）",
                pass.label, pass.vertex_entry, inputs
            ));
        }

        let mut materials = Materials::new(&gpu.device, &gpu.queue);
        let material_layout = materials.bind_group_layout().clone();
        let stage_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("组 1：这一条 pass 的参数"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let mut instance_stream: Vec<u8> = Vec::with_capacity(scene.objects.len() * 112);
        for object in &scene.objects {
            instance_stream.extend_from_slice(&instance_bytes(&object.transform));
        }
        let instance_buffer = gpu.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("组 1：MeshInstance 数组（长度 = 物体数）"),
            usage: wgpu::BufferUsages::STORAGE,
            contents: &instance_stream,
        });

        let make_stage = |label: &str, view: &crate::camera::Camera| -> Stage {
            let mut contents = view_bytes(view).to_vec();
            contents.resize(crate::plan::GEOMETRY_PARAMS_SIZE, 0);
            contents[crate::plan::VIEW_PAGE_OFFSET..crate::plan::VIEW_PAGE_OFFSET + 4]
                .copy_from_slice(&0.0_f32.to_le_bytes());
            contents[crate::plan::VIEW_PAGE_OFFSET + 4..crate::plan::VIEW_PAGE_OFFSET + 8]
                .copy_from_slice(&0.0_f32.to_le_bytes());
            contents[crate::plan::VIEW_PAGE_OFFSET + 8..crate::plan::VIEW_PAGE_OFFSET + 12]
                .copy_from_slice(&1.0_f32.to_le_bytes());
            contents[crate::plan::VIEW_PAGE_OFFSET + 12..crate::plan::VIEW_PAGE_OFFSET + 16]
                .copy_from_slice(&1.0_f32.to_le_bytes());
            let buffer = gpu.device.create_buffer_init(&BufferInitDescriptor {
                label: Some(label),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                contents: &contents,
            });
            let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &stage_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(crate::plan::GEOMETRY_PARAMS_SIZE as u64),
                    }),
                }],
            });
            Stage {
                view_buffer: buffer,
                bind_group,
            }
        };
        let face_stages: Vec<Stage> = faces
            .iter()
            .map(|face| {
                make_stage(
                    &format!("组 1：PassView（灯 {} 面 {}）", face.light, face.face),
                    &face.camera,
                )
            })
            .collect();
        {
            let mut filled = 0_usize;
            for pass in executed_plan.passes.iter_mut() {
                if pass.params.len() < 64 || pass.kind != px_pass::PassKind::Geometry {
                    continue;
                }
                let Some(layer) = pass.layer else { continue };
                let Some(face) = faces
                    .iter()
                    .find(|face| face.light * group0::SHADOW_CUBE_FACES + face.face == layer)
                else {
                    continue;
                };
                pass.params[0..64].copy_from_slice(&view_bytes(&face.camera));
                for draw in pass.draws.iter_mut() {
                    if !draw.material.ends_with("@shadowpage") {
                        draw.material = format!("{}@shadowpage", draw.material);
                    }
                }
                if std::env::var_os("PX_SHADOW_FULLPAGE").is_some() {
                    let rect = [0.0_f32, 0.0, 1.0, 1.0];
                    for (index, number) in rect.iter().enumerate() {
                        let at = crate::plan::VIEW_PAGE_OFFSET + index * 4;
                        pass.params[at..at + 4].copy_from_slice(&number.to_le_bytes());
                    }
                }
                filled += 1;
            }
            audit.push(format!(
                "页 pass 的视图：{filled} 条参数块的 `view_proj` 由宿主按 `cube_face` 那一面填上\
                 （烘图侧只写了 `view_page`，矩阵那 64 字节是占位 0）"
            ));
            if std::env::var_os("PX_AUDIT_SHADOW").is_some() {
                if let Some(pass) = executed_plan
                    .passes
                    .iter()
                    .find(|pass| pass.params.len() >= 64 && pass.layer.is_some())
                {
                    let mut first = [0.0_f32; 4];
                    for (index, slot) in first.iter_mut().enumerate() {
                        let at = 48 + index * 4;
                        *slot = f32::from_le_bytes([
                            pass.params[at],
                            pass.params[at + 1],
                            pass.params[at + 2],
                            pass.params[at + 3],
                        ]);
                    }
                    let layer = pass.layer.unwrap_or(0);
                    let light = faces
                        .iter()
                        .find(|face| face.light * group0::SHADOW_CUBE_FACES + face.face == layer)
                        .map(|face| face.light);
                    audit.push(format!(
                        "  · 页 pass '{}'（层 {layer}）的 `view_proj` 第一列 = {first:?}｜\
                         那一面的灯位 = {:?}",
                        pass.label,
                        light
                            .and_then(|index| cluster.get(index as usize))
                            .map(|light| light.position_radius[0..3].to_vec()),
                    ));
                }
            }
            for pass in executed_plan
                .passes
                .iter()
                .filter(|pass| pass.label.starts_with("point_shadow") && !pass.draws.is_empty())
                .take(2)
            {
                let draw = &pass.draws[0];
                audit.push(format!(
                    "  · '{}'｜viewport {:?}｜参数 {} B｜几何 '{}'｜材质 '{}'｜
                     {:?}",
                    pass.label,
                    pass.viewport,
                    pass.params.len(),
                    draw.geometry,
                    draw.material,
                    pass.render,
                ));
            }
        }
        audit.push(format!(
        "组 1（两类参数）：**super** = 每视图一份 `PassView`（{} 份 × 64 B = {} B：\
         相机 **每格一份**（{} 格）+ 影子面 {}）｜**instance** = 全帧**一份**实例数组\
         （{} 个物体 × 112 B = {} B，按 `@builtin(instance_index)` 选格）。拆之前是每 \
         (物体, 视图) 一份 176 B 的 `MeshStage`（{} 份 = {} B）⇒ 数据从 (物体 × 视图) 那一维上下来了",
        faces.len() + placements.len(),
        (faces.len() + placements.len()) * 64,
        placements.len(),
        faces.len(),
        scene.objects.len(),
        instance_stream.len(),
        (faces.len() + placements.len()) * scene.objects.len(),
        (faces.len() + placements.len()) * scene.objects.len() * 176,
    ));

        let mut bindings = Vec::with_capacity(scene.objects.len());
        for object in &scene.objects {
            let binding = materials.bind(&gpu.device, &gpu.queue, object)?;
            audit.push(format!(
            "材质 '{}'：shader {}｜参数 {} 字节｜绑上的格 {:?}｜走兜底白图的格 {:?}（布局是 12 格超集）",
            object.id,
            object.shader.member,
            object.params.len(),
            binding.bound_slots,
            binding.fallback_slots
        ));
            bindings.push(binding);
        }

        for pass in &plan.passes {
            for draw in &pass.draws {
                if draw.material.is_empty() {
                    continue;
                }
                let table = material_table(&spec, &draw.material)?;
                audit.push(format!(
                    "pass '{}' 的 draw（几何 '{}'）要材质 '{}' ⇒ {}",
                    pass.label,
                    draw.geometry,
                    draw.material,
                    table.describe()
                ));
            }
        }

        let modules = shader::modules();
        let mut frame_materials: Vec<(art::LoadedFrameMaterial, material::MaterialBinding)> =
            Vec::with_capacity(spec.frame_materials.len());
        for declared in &spec.frame_materials {
            let loaded = art::load_frame_material(declared, &modules)?;
            let slot = skybox_slot(&loaded, scene.skybox.as_ref())?;
            let textures: Vec<art::BoundTexture> = match (&scene.skybox, slot) {
                (Some(skybox), Some(binding)) => vec![skybox.bound(binding)],
                _ => Vec::new(),
            };
            let binding = materials.bind_request(
                &gpu.device,
                &gpu.queue,
                &material::BindingRequest {
                    key: material::frame_key(loaded.version),
                    label: loaded.name.as_str(),
                    params: loaded.params.as_slice(),
                    textures: textures.as_slice(),
                },
            )?;
            audit.push(format!(
                "帧自有材质 '{}'：entry {}｜组装后 {} 字节（内容键 {:016x}）｜参数 {} 字节｜
             声明的贴图格 {:?}｜绑上的格 {:?}｜走兜底白图的格 {:?}（**与内容材质同一条 \
             12 格超集的路**：帧材质的「自有」是兑现者自有，不是契约自有）",
                loaded.name,
                loaded.entry,
                loaded.assembled.len(),
                loaded.version,
                loaded.params.len(),
                loaded.textures,
                binding.bound_slots,
                binding.fallback_slots
            ));
            if let (Some(skybox), Some(binding)) = (&scene.skybox, slot) {
                audit.push(format!(
                    "  ⚠ 第 {binding} 格绑的是环境里那份天空盒 {}（采样器 {:?} —— \
                 oracle 是**显式**传 `Sampler::clamped()`，不是落回缺省；\
                 它与 `Sampler::default()` 差在 `address_u`）",
                    skybox.texture.label(),
                    skybox.sampler
                ));
            }
            frame_materials.push((loaded, binding));
        }

        let host_target = shot::Target::new(&gpu.device, target.0, target.1);
        let mut cells: Vec<Cell> = Vec::with_capacity(placements.len());
        for placement in &placements {
            let viewport = placement.uniform_viewport((width, height));
            let zero = build_zero(
                &placement.camera,
                &sampled_view,
                &shadow_view_refs,
                viewport,
                &instance_buffer,
                &shadow_page_offsets,
                &shadow_faces,
            )?;
            let dummy_shadow = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("影子哑图：给影子页 pass 的组 0 用"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let dummy_view = dummy_shadow.create_view(&wgpu::TextureViewDescriptor {
                label: Some("影子哑图视图"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
            let dummy_view_refs = [&dummy_view, &dummy_view, &dummy_view, &dummy_view];
            let zero_dummy = build_zero(
                &placement.camera,
                &sampled_view,
                &dummy_view_refs,
                viewport,
                &instance_buffer,
                &shadow_page_offsets,
                &shadow_faces,
            )?;
            let camera_stage = make_stage(
                &format!("组 1：PassView（相机，第 {} 格）", placement.index),
                &placement.camera,
            );
            cells.push(Cell {
                zero,
                zero_dummy,
                stage: camera_stage,
            });
        }

        let (shader_slots, shader_notes) = Session::shader_slots_of(
            &spec,
            &scene,
            &frame_materials,
            pcg_root,
            &executed_plan,
            &modules,
        );

        Ok(Session {
            spec,
            scene,
            executed_plan,
            executor,
            geometries,
            bindings,
            frame_materials,
            faces,
            face_stages,
            stage_layout,
            material_layout,
            materials,
            instance_buffer,
            shadow_view: shadow_views[0].clone(),
            shadow_sampler,
            shadow_probe: shadow_textures,
            scene_path: scene_path.to_path_buf(),
            pcg_root: pcg_root.to_path_buf(),
            audit_head,
            audit_rest: audit,
            loads_first,
            shader_slots,
            shader_notes,
            layer: Some(Layer {
                target,
                rects: placements.iter().map(|placement| placement.rect).collect(),
                host_target,
                cells,
                used: false,
            }),
        })
    }

    pub fn draw(
        &mut self,
        gpu: &Gpu,
        views: Views,
        width: u32,
        height: u32,
    ) -> Result<Rendered, String> {
        self.draw_with(gpu, views, width, height, None)
    }

    pub fn draw_stamps(
        &mut self,
        gpu: &Gpu,
        views: Views,
        width: u32,
        height: u32,
        stamps: &crate::spans::FrameStamps<'_>,
    ) -> Result<Rendered, String> {
        self.draw_with(gpu, views, width, height, Some(stamps))
    }

    fn draw_with(
        &mut self,
        gpu: &Gpu,
        views: Views,
        width: u32,
        height: u32,
        stamps: Option<&crate::spans::FrameStamps<'_>>,
    ) -> Result<Rendered, String> {
        let (placements, target) = placements(views, &self.spec, width, height)?;
        let mut rebuild = !self
            .layer
            .as_ref()
            .is_some_and(|layer| layer.matches(target, &placements));
        if self.layer.as_ref().is_some_and(|layer| layer.used) && !self.loads_first.is_empty() {
            rebuild = true;
        }
        if rebuild {
            let (path, root) = (self.scene_path.clone(), self.pcg_root.clone());
            *self = Session::open(gpu, &path, &root, views, width, height)?;
        }
        if let Some(stamps) = stamps {
            if stamps.passes() as usize != self.executed_plan.passes.len() {
                return Err(format!(
                    "时间戳槽是按 {} 条 pass 排的，而这一帧的计划里有 {} 条：\
                     槽与实物对不上（读到的会是别的 pass 的格）",
                    stamps.passes(),
                    self.executed_plan.passes.len()
                ));
            }
        }
        let mut layer = self.layer.take().expect("上面刚保证过它在");
        let outcome = self.draw_into(
            gpu,
            &mut layer,
            views,
            &placements,
            target,
            width,
            height,
            stamps,
        );
        self.layer = Some(layer);
        outcome
    }

    pub fn pass_kinds(&self) -> Vec<(String, px_pass::PassKind)> {
        self.executed_plan
            .passes
            .iter()
            .map(|pass| (pass.label.clone(), pass.kind))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_into(
        &mut self,
        gpu: &Gpu,
        layer: &mut Layer,
        views: Views,
        placements: &[Placement],
        target: (u32, u32),
        width: u32,
        height: u32,
        stamps: Option<&crate::spans::FrameStamps<'_>>,
    ) -> Result<Rendered, String> {
        if let Some(stamps) = stamps {
            if placements.len() != 1 {
                return Err(format!(
                    "这一帧有 {} 格，而时间戳槽只按**单张**排（每帧 {} 格）：\
                     12 格会往同一批格上写 12 遍，读到的只是其中一遍",
                    placements.len(),
                    crate::spans::FrameStamps::stride_of(stamps.passes())
                ));
            }
        }
        let Session {
            spec,
            scene,
            executed_plan,
            bindings,
            frame_materials,
            faces,
            face_stages,
            stage_layout,
            material_layout,
            geometries,
            audit_head,
            audit_rest,
            executor,
            ..
        } = self;
        let mut audit: Vec<String> = Vec::with_capacity(audit_head.len() + audit_rest.len() + 8);
        let mut timestamp_calls = 0_u32;
        audit.extend(audit_head.iter().cloned());
        audit.extend(describe_views(views, placements, width, height, target));
        audit.extend(audit_rest.iter().cloned());
        for (cell, placement) in layer.cells.iter_mut().zip(placements.iter()) {
            let viewport = placement.uniform_viewport((width, height));
            audit.push(format!(
                "—— {} 的 group 0（`view.viewport` = ({}, {}, {}, {})）——",
                placement.note, viewport[0], viewport[1], viewport[2], viewport[3]
            ));
            audit.push(cell.set_view(&gpu.queue, &placement.camera, viewport));
            audit.extend(cell.zero.audit.iter().map(|line| format!("  {line}")));
        }
        let mut sets: Vec<Vec<External<'_>>> = Vec::with_capacity(executed_plan.passes.len());
        for (index, pass) in executed_plan.passes.iter().enumerate() {
            let mut set: Vec<External<'_>> = Vec::new();
            let _ = (index, &pass.label);
            if pass.writes.first().map(String::as_str) == Some(px_protocol::scene::VIEW_BUILTIN) {
                set.push(External {
                    name: px_protocol::scene::VIEW_BUILTIN,
                    role: Role::Write,
                    view: &layer.host_target.view,
                    format: shot::FORMAT,
                });
            }
            sets.push(set);
        }
        let resolved_geometry: Vec<ResolvedGeometry<'_>> = geometries
            .iter()
            .enumerate()
            .map(|(index, geometry)| ResolvedGeometry {
                name: geometry.name.as_str(),
                vertices: geometry
                    .vertices
                    .as_ref()
                    .map(|buffer| (buffer, geometry.layout())),
                indices: geometry
                    .indices
                    .as_ref()
                    .map(|buffer| (buffer, wgpu::IndexFormat::Uint32, geometry.index_count)),
                vertex_count: geometry.vertex_count,
                instances: if index < scene.objects.len() {
                    index as u32..index as u32 + 1
                } else {
                    0..1
                },
            })
            .collect();

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("px_render 的一帧（一格一次）"),
            });
        if let Some(stamps) = stamps {
            stamps.begin_frame(&mut encoder);
        }
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("px_render 宿主目标清零（让复用的目标等于新目标）"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &layer.host_target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        for (cell, placement) in layer.cells.iter().zip(placements.iter()) {
            let mut quiet: Vec<String> = Vec::new();
            let audit_sink: &mut Vec<String> = if placement.index == 0 {
                &mut audit
            } else {
                &mut quiet
            };
            let shadowpage_names: Vec<String> = scene
                .objects
                .iter()
                .map(|object| format!("{}@shadowpage", object.id))
                .collect();
            let table = cell_materials(
                &cell.zero,
                &cell.zero_dummy,
                &shadowpage_names,
                &cell.stage,
                scene,
                bindings,
                spec,
                executed_plan,
                faces,
                face_stages,
                stage_layout,
                material_layout,
                frame_materials,
                audit_sink,
            )?;
            let frame = Frame {
                width: target.0,
                height: target.1,
                viewport: placement.viewport(),
                sets: &sets,
                geometries: &resolved_geometry,
                materials: &table,
                zero_dummy: Some((&cell.zero_dummy.layout, &cell.zero_dummy.bind_group)),
            };
            let audit_text = match stamps {
                Some(stamps) => {
                    let (text, calls) = executor.execute_timed(
                        &gpu.device,
                        &mut encoder,
                        executed_plan,
                        &frame,
                        stamps.executor_view(),
                    )?;
                    timestamp_calls += calls;
                    text
                }
                None => executor.execute(&gpu.device, &mut encoder, executed_plan, &frame)?,
            };
            audit.push(format!(
                "第 {} 格的执行器审计：\n{audit_text}",
                placement.index
            ));
        }
        if let Some(stamps) = stamps {
            stamps.end_frame(&mut encoder);
            stamps.resolve_now(&mut encoder);
        }
        gpu.queue.submit(Some(encoder.finish()));
        layer.used = true;

        if std::env::var_os("PX_AUDIT_SHADOW").is_some() {
            for (level, texture) in self.shadow_probe.iter().enumerate() {
                let Some(texture) = texture else {
                    continue;
                };
                println!(
                    "影子 atlas 探针：级 {level}｜{}×{} × {} 层（**这一帧画完之后**的读数）",
                    texture.width(),
                    texture.height(),
                    texture.depth_or_array_layers()
                );
                for layer in 0..texture.depth_or_array_layers().min(6) {
                    match read_depth_stats(&gpu.device, &gpu.queue, texture, layer) {
                        Some((max, nonzero, total, corner)) => println!(
                            "影子 atlas 探针：级 {level} 层 {layer}｜最大深度 {max:.8}｜非零 {nonzero} / {total}\
                             （左上 256² 里 {corner}）"
                        ),
                        None => println!("影子 atlas 探针：级 {level} 层 {layer}｜读不回来"),
                    }
                }
            }
        }

        let pixels = shot::read_back(&gpu.device, &gpu.queue, &layer.host_target)?;
        Ok(Rendered {
            pixels,
            width: target.0,
            height: target.1,
            audit,
            executed: executed_plan
                .passes
                .iter()
                .map(|pass| pass.label.clone())
                .collect(),
            skipped: Vec::new(),
            declared_clouds: spec.expects.iter().any(|tag| tag == "clouds"),
            timestamp_calls,
        })
    }

    fn shader_slots_of(
        spec: &px_protocol::scene::SceneSpec,
        scene: &art::LoadedScene,
        frame_materials: &[(art::LoadedFrameMaterial, material::MaterialBinding)],
        pcg_root: &Path,
        plan: &Plan,
        modules: &px_shader::ModuleTable,
    ) -> (Vec<ShaderSlot>, Vec<String>) {
        let mut slots: Vec<ShaderSlot> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for (index, object) in scene.objects.iter().enumerate() {
            let name = format!("{}.wgsl", object.shader.member.node);
            let what = format!("材质 '{}'", object.id);
            match shader::try_source_of(&name) {
                Ok((text, path)) => slots.push(ShaderSlot::new(
                    what,
                    path,
                    SlotPlace::Object(index),
                    &object.shader.assembled,
                    Some(object.shader.member.to_string()),
                    &object.shader.source,
                    &text,
                    modules,
                )),
                Err(err) => notes.push(format!("{what}：{err} ⇒ 这一槽不热重载")),
            }
        }
        for (index, pass) in spec.passes.iter().enumerate() {
            let Some(member) = pass.shader.as_ref() else {
                continue;
            };
            let what = format!("pass '{}' 的片元", pass.label_or(index));
            let name = format!("{}.wgsl", member.node);
            match shader::try_source_of(&name) {
                Ok((text, path)) => match art::load_shader(member, &pcg_root, &shader::modules()) {
                    Ok(loaded) => slots.push(ShaderSlot::new(
                        what,
                        path,
                        SlotPlace::Pass(index),
                        &plan.passes[index].shader,
                        Some(member.to_string()),
                        &loaded.source,
                        &text,
                        modules,
                    )),
                    Err(err) => notes.push(format!("{what}：{err} ⇒ 这一槽不热重载")),
                },
                Err(err) => notes.push(format!("{what}：{err} ⇒ 这一槽不热重载")),
            }
        }
        for (index, frame) in spec.frame_materials.iter().enumerate() {
            let what = format!("帧自有材质 '{}'", frame.name);
            let path = shader::workspace()
                .join("art")
                .join("frame")
                .join(format!("{}.wgsl", frame.name));
            match std::fs::read_to_string(&path) {
                Ok(text) => match frame_materials.get(index) {
                    Some((loaded, _)) => slots.push(ShaderSlot::new(
                        what,
                        path,
                        SlotPlace::FrameMaterial(index),
                        &loaded.assembled,
                        None,
                        &frame.shader,
                        &text,
                        modules,
                    )),
                    None => notes.push(format!("{what}：产物里没有装载过的那一份 ⇒ 不热重载")),
                },
                Err(err) => notes.push(format!(
                    "{what}：读不了 {}：{err}（它在文档里是内联全文、盘上找不到同名文件）\
                     ⇒ 这一槽不热重载",
                    path.display()
                )),
            }
        }
        let vertex: Vec<String> = spec
            .passes
            .iter()
            .filter(|pass| !pass.vertex_shader.trim().is_empty())
            .map(|pass| pass.label.clone())
            .collect();
        if !vertex.is_empty() {
            notes.push(format!(
                "顶点阶段（pass {}）在文档里是**内联全文**、不带来源名 ⇒ 不热重载\
                 （`art/frame/vertex_*.wgsl` 改了要重烘）",
                vertex.join(" / ")
            ));
        }
        (slots, notes)
    }

    pub fn reload_shaders(&mut self, changed: &[PathBuf]) -> ReloadReport {
        let started = std::time::Instant::now();
        let mut report = ReloadReport {
            changed_files: changed.to_vec(),
            ..ReloadReport::default()
        };
        let modules = match shader::try_modules() {
            Ok(modules) => modules,
            Err(err) => {
                report
                    .refused
                    .push(format!("读不了 shader 库：{err}（下一次改动再试）"));
                report.millis = started.elapsed().as_secs_f64() * 1e3;
                return report;
            }
        };
        let slots = self.shader_slots.clone();
        report.notes = self.shader_notes.clone();
        let mut staged: Vec<(usize, String, String, u64, String, u64)> = Vec::new();
        for (index, slot) in slots.iter().enumerate() {
            if !slot.proven {
                report.refused.push(format!(
                    "{}：盘上那份与文档记的那份**对不上**（{} ≠ {}）⇒ 我不敢说它就是这一槽的来源，\
                     不重载；要改它请重烘\n    {}",
                    slot.what, slot.disk, slot.source_hash, slot.difference
                ));
                continue;
            }
            let text = match std::fs::read_to_string(&slot.path) {
                Ok(text) => text,
                Err(err) => {
                    report.refused.push(format!(
                        "{}：读不了 {}：{err}",
                        slot.what,
                        slot.path.display()
                    ));
                    continue;
                }
            };
            let text_hash = sha16(&text);
            let closure = px_shader::closure(&text, &modules).fingerprint();
            if text_hash == slot.text_hash && closure == slot.closure {
                report.skipped += 1;
                continue;
            }
            let reflected = match art::reflect_source(&slot.what, &text, &modules) {
                Ok(reflected) => reflected,
                Err(err) => {
                    report.refused.push(format!("{}：{err}", slot.what));
                    continue;
                }
            };
            let version = text_version(&reflected.assembled);
            if version == slot.version {
                report.skipped += 1;
                continue;
            }
            if let Some(entry) = self.entry_of(slot) {
                if let Err(err) = art::fragment_entry(&reflected.module, &entry, &slot.what) {
                    report.refused.push(format!("{}：{err}", slot.what));
                    continue;
                }
            }
            if let Err(why) = self.contract_unchanged(slot, &reflected) {
                report.refused.push(format!("{}：{why}", slot.what));
                continue;
            }
            report.reloaded.push(SlotChange {
                what: slot.what.clone(),
                path: slot.path.clone(),
                text_hash: text_hash.clone(),
                old: slot.version,
                new: version,
                key: None,
                member: slot.member.clone(),
            });
            staged.push((
                index,
                text,
                text_hash,
                closure,
                reflected.assembled,
                version,
            ));
        }
        if staged.is_empty() {
            report.millis = started.elapsed().as_secs_f64() * 1e3;
            return report;
        }
        let mut candidate = self.executed_plan.clone();
        for (index, _, _, _, text, _) in &staged {
            if let SlotPlace::Pass(pass) = slots[*index].place {
                candidate.passes[pass].shader = text.clone();
            }
        }
        if let Err(err) = candidate.check() {
            report.refused.push(format!(
                "整份计划过不了 `Plan::check()`：{err} ⇒ 这一批改动**一条都没落**（画面还是上一版）"
            ));
            report.reloaded.clear();
            report.millis = started.elapsed().as_secs_f64() * 1e3;
            return report;
        }
        for (index, text_source, text_hash, closure, text, version) in staged {
            let place = slots[index].place;
            self.shader_slots[index].version = version;
            self.shader_slots[index].text_hash = text_hash;
            self.shader_slots[index].closure = closure;
            let _ = text_source;
            if let SlotPlace::Object(object) = place {
                let before = material::key_of(&self.scene.objects[object]).map(|key| key.shader);
                self.scene.objects[object].shader.assembled = text;
                let after = material::key_of(&self.scene.objects[object]).map(|key| key.shader);
                if let (Ok(before), Ok(after)) = (before, after) {
                    if let Some(change) = report
                        .reloaded
                        .iter_mut()
                        .find(|change| change.what == slots[index].what)
                    {
                        change.key = Some((before, after));
                    }
                }
            } else if let SlotPlace::Pass(pass) = place {
                self.executed_plan.passes[pass].shader = text;
            } else if let SlotPlace::FrameMaterial(frame) = place {
                if let Some((loaded, _)) = self.frame_materials.get_mut(frame) {
                    loaded.assembled = text;
                    loaded.version = version;
                }
            }
        }
        report.untouched = self.untouched();
        report.millis = started.elapsed().as_secs_f64() * 1e3;
        for line in report.readout() {
            self.audit_rest.push(line);
        }
        report
    }

    fn entry_of(&self, slot: &ShaderSlot) -> Option<String> {
        match slot.place {
            SlotPlace::Object(_) => Some(FRAGMENT_ENTRY.to_string()),
            SlotPlace::Pass(index) => Some(self.spec.passes.get(index)?.entry.clone()),
            SlotPlace::FrameMaterial(index) => {
                Some(self.frame_materials.get(index)?.0.entry.clone())
            }
        }
    }

    fn contract_unchanged(
        &self,
        slot: &ShaderSlot,
        reflected: &art::Reflected,
    ) -> Result<(), String> {
        match slot.place {
            SlotPlace::Object(index) => {
                let object = self
                    .scene
                    .objects
                    .get(index)
                    .ok_or_else(|| "物体不见了".to_string())?;
                let before = object
                    .shader
                    .layout
                    .to_json()
                    .map_err(|err| format!("旧布局解不开：{err}"))?;
                let after = reflected
                    .layout
                    .to_json()
                    .map_err(|err| format!("新布局解不开：{err}"))?;
                if before != after {
                    return Err(format!(
                        "反射出来的契约变了 ⇒ 不重载（改布局要重烘）：\n  文档那一版 {before}\n  盘上这一版 {after}"
                    ));
                }
                let packed = reflected
                    .layout
                    .pack(&object_params(&self.spec, &object.id))
                    .map_err(|err| format!("按新布局打包参数：{err}"))?;
                if packed != object.params {
                    return Err(format!(
                        "参数块按新布局打出来与手里那份不同（{} 字节 vs {} 字节）⇒ 不重载",
                        packed.len(),
                        object.params.len()
                    ));
                }
                Ok(())
            }
            SlotPlace::Pass(index) => {
                let pass = self
                    .spec
                    .passes
                    .get(index)
                    .ok_or_else(|| "pass 不见了".to_string())?;
                let packed = reflected
                    .layout
                    .pack(&pass.params)
                    .map_err(|err| format!("按新布局打包参数：{err}"))?;
                let now = &self.executed_plan.passes[index];
                if packed != now.params {
                    return Err(format!(
                        "参数块按新布局打出来与计划里那份不同（{} 字节 vs {} 字节）⇒ 不重载",
                        packed.len(),
                        now.params.len()
                    ));
                }
                let declared: Vec<u32> = reflected
                    .layout
                    .textures
                    .iter()
                    .map(|slot| slot.binding)
                    .collect();
                if declared != now.slots {
                    return Err(format!(
                        "声明的贴图格变了（{:?} → {declared:?}）⇒ 不重载",
                        now.slots
                    ));
                }
                Ok(())
            }
            SlotPlace::FrameMaterial(index) => {
                let frame = self
                    .spec
                    .frame_materials
                    .get(index)
                    .ok_or_else(|| "帧材质不见了".to_string())?;
                let loaded = self
                    .frame_materials
                    .get(index)
                    .map(|(loaded, _)| loaded)
                    .ok_or_else(|| "帧材质没装载过".to_string())?;
                let packed = reflected
                    .layout
                    .pack(&frame.params)
                    .map_err(|err| format!("按新布局打包参数：{err}"))?;
                if packed != loaded.params {
                    return Err(format!(
                        "参数块按新布局打出来与手里那份不同（{} 字节 vs {} 字节）⇒ 不重载",
                        packed.len(),
                        loaded.params.len()
                    ));
                }
                let declared: Vec<(u32, _)> = reflected
                    .layout
                    .textures
                    .iter()
                    .map(|slot| (slot.binding, slot.dimension))
                    .collect();
                if declared != loaded.textures {
                    return Err(format!(
                        "声明的贴图格变了（{:?} → {declared:?}）⇒ 不重载",
                        loaded.textures
                    ));
                }
                Ok(())
            }
        }
    }

    fn untouched(&self) -> Untouched {
        let textures = self
            .scene
            .objects
            .iter()
            .map(|object| object.textures.len())
            .sum();
        Untouched {
            objects: self.scene.objects.len(),
            geometries: self.geometries.len(),
            textures,
            params_bytes: self
                .scene
                .objects
                .iter()
                .map(|object| object.params.len())
                .sum(),
            instances: 1,
            cells: self
                .layer
                .as_ref()
                .map(|layer| layer.cells.len())
                .unwrap_or(0),
        }
    }
}

fn loads_before_write(plan: &Plan) -> Vec<String> {
    let mut written: Vec<String> = Vec::new();
    let mut loaded: Vec<String> = Vec::new();
    let note = |loaded: &mut Vec<String>, written: &mut Vec<String>, name: &str| {
        if !written.iter().any(|seen| seen == name) && !loaded.iter().any(|seen| seen == name) {
            loaded.push(name.to_string());
        }
        written.push(name.to_string());
    };
    for pass in &plan.passes {
        match pass.kind {
            PassKind::Copy => {
                for name in &pass.writes {
                    written.push(name.clone());
                }
            }
            _ => {
                if let Some(target) = pass.target() {
                    if pass.render.color == Attachment::Load {
                        note(&mut loaded, &mut written, target);
                    } else {
                        written.push(target.to_string());
                    }
                }
                if let Some(target) = pass.depth_target.as_deref() {
                    if pass.render.depth == Attachment::Load {
                        note(&mut loaded, &mut written, target);
                    } else {
                        written.push(target.to_string());
                    }
                }
            }
        }
    }
    loaded.retain(|name| name != px_protocol::scene::VIEW_BUILTIN);
    loaded
}

fn describe_views(
    views: Views,
    placements: &[Placement],
    width: u32,
    height: u32,
    target: (u32, u32),
) -> Vec<String> {
    let mut audit = vec![format!(
        "怎么看：{:?}｜{} 台相机｜一格 {}×{}（aspect {}，位模式 {:08X}）⇒ 目标 {}×{}\
         ｜无限 reverse-Z / Depth32Float / 清 0.0 / GreaterEqual",
        views,
        placements.len(),
        width,
        height,
        width as f32 / height as f32,
        (width as f32 / height as f32).to_bits(),
        target.0,
        target.1
    )];
    for placement in placements {
        audit.push(format!(
            "  {}｜from_xyz({}, {}, {}).looking_at(ZERO, Y)｜viewport {}",
            placement.note,
            placement.camera.position.x,
            placement.camera.position.y,
            placement.camera.position.z,
            match placement.uniform_viewport((width, height)) {
                [x, y, w, h] => format!("({x}, {y}, {w}, {h})"),
            }
        ));
    }
    audit
}

fn read_depth_stats(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
) -> Option<(f32, usize, usize, usize)> {
    let side = texture.width();
    let bytes_per_row = side * 4;
    let size = u64::from(bytes_per_row) * u64::from(side);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("影子 atlas 统计"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("影子 atlas 统计"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(side),
            },
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(60)),
        })
        .ok()?;
    receiver.recv().ok()?.ok()?;
    let data = slice.get_mapped_range();
    let mut max = 0.0_f32;
    let mut nonzero = 0_usize;
    let mut corner = 0_usize;
    let mut bbox = (u32::MAX, u32::MAX, 0_u32, 0_u32);
    let total = (side * side) as usize;
    for y in 0..side {
        for x in 0..side {
            let at = (y * bytes_per_row + x * 4) as usize;
            let value = f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
            if value > 0.0 {
                nonzero += 1;
                if value > max {
                    max = value;
                }
                if x < 256 && y < 256 {
                    corner += 1;
                }
                bbox = (bbox.0.min(x), bbox.1.min(y), bbox.2.max(x), bbox.3.max(y));
            }
        }
    }
    drop(data);
    buffer.unmap();
    if nonzero > 0 {
        println!(
            "    （非零的包围盒 = x {}..{}、y {}..{}）",
            bbox.0, bbox.2, bbox.1, bbox.3
        );
    }
    Some((max, nonzero, total, corner))
}

#[allow(clippy::too_many_arguments)]
fn cell_materials<'a>(
    zero: &'a group0::GroupZero,
    zero_dummy: &'a group0::GroupZero,
    shadowpage_names: &'a [String],
    camera_stage: &'a Stage,
    scene: &'a art::LoadedScene,
    bindings: &'a [material::MaterialBinding],
    spec: &'a px_protocol::scene::SceneSpec,
    plan: &'a Plan,
    faces: &'a [Face],
    face_stages: &'a [Stage],
    stage_layout: &'a wgpu::BindGroupLayout,
    material_layout: &'a wgpu::BindGroupLayout,
    frame_materials: &'a [(art::LoadedFrameMaterial, material::MaterialBinding)],
    audit: &mut Vec<String>,
) -> Result<Vec<ResolvedMaterial<'a>>, String> {
    let mut resolved_materials: Vec<ResolvedMaterial<'_>> = scene
        .objects
        .iter()
        .zip(bindings.iter())
        .map(|(object, binding)| {
            Ok(ResolvedMaterial {
                name: object.id.as_str(),
                groups: vec![
                    ResolvedGroup {
                        group: 0,
                        bind_group: &zero.bind_group,
                        layout: zero.layout.clone(),
                        layout_id: ZERO_LAYOUT_ID,
                        dynamic: false,
                        dynamic_offset: 0,
                    },
                    ResolvedGroup {
                        group: 1,
                        bind_group: &camera_stage.bind_group,
                        layout: stage_layout.clone(),
                        layout_id: STAGE_LAYOUT_ID,
                        dynamic: true,
                        dynamic_offset: 0,
                    },
                    ResolvedGroup {
                        group: MATERIAL_BIND_GROUP,
                        bind_group: &binding.bind_group,
                        layout: material_layout.clone(),
                        layout_id: MATERIAL_LAYOUT_ID,
                        dynamic: false,
                        dynamic_offset: 0,
                    },
                ],
                blend: material::key_of(object)?.blend(),
                cull: cull_of(object.cull),
                fragment_shader: object.shader.assembled.as_str(),
                fragment_entry: FRAGMENT_ENTRY,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let object_materials: Vec<ResolvedMaterial<'a>> =
        resolved_materials[..shadowpage_names.len().min(resolved_materials.len())].to_vec();
    for (base, name) in object_materials.iter().zip(shadowpage_names.iter()) {
        let mut variant = base.clone();
        variant.name = name.as_str();
        if let Some(group) = variant.groups.iter_mut().find(|group| group.group == 0) {
            group.bind_group = &zero_dummy.bind_group;
            group.layout = zero_dummy.layout.clone();
            group.layout_id = ZERO_LAYOUT_ID;
        }
        resolved_materials.push(variant);
    }
    resolved_materials.extend(
        frame_materials
            .iter()
            .map(|(loaded, binding)| ResolvedMaterial {
                name: loaded.name.as_str(),
                groups: vec![
                    ResolvedGroup {
                        group: 0,
                        bind_group: &zero.bind_group,
                        layout: zero.layout.clone(),
                        layout_id: ZERO_LAYOUT_ID,
                        dynamic: false,
                        dynamic_offset: 0,
                    },
                    ResolvedGroup {
                        group: MATERIAL_BIND_GROUP,
                        bind_group: &binding.bind_group,
                        layout: material_layout.clone(),
                        layout_id: MATERIAL_LAYOUT_ID,
                        dynamic: false,
                        dynamic_offset: 0,
                    },
                ],
                blend: binding.key.blend(),
                cull: Cull::None,
                fragment_shader: loaded.assembled.as_str(),
                fragment_entry: loaded.entry.as_str(),
            }),
    );

    if !spec.material_instances.is_empty() {
        let face_of = |light: u32, face: u32| -> Result<usize, String> {
            faces
                .iter()
                .position(|candidate| candidate.light == light && candidate.face == face)
                .ok_or_else(|| format!("这一面（灯 {light}，面 {face}）没有建出来（内部不一致）"))
        };
        for instance in &spec.material_instances {
            let users: Vec<(&str, Option<px_protocol::scene::PassCubeFace>)> = plan
                .passes
                .iter()
                .filter_map(|pass| {
                    if !pass.draws.iter().any(|draw| draw.material == instance.name) {
                        return None;
                    }
                    let document = spec
                        .passes
                        .iter()
                        .enumerate()
                        .find(|(index, document)| document.label_or(*index) == pass.label)
                        .map(|(_, document)| document);
                    Some((pass.label.as_str(), document.and_then(|d| d.cube_face)))
                })
                .collect();
            let cube = match users.as_slice() {
                [] => {
                    return Err(format!(
                        "生成的材质实例 '{}' 没有任何一条 pass 用它（文档那边应当已经拒过）",
                        instance.name
                    ));
                }
                [(label, Some(cube))] => {
                    audit.push(format!(
                        "实例 '{}' ⇒ 照 '{}'，用在 pass '{label}' 上 ⇒ 那一面的 PassView\
                         （灯 {}，面 {}，层 {}）",
                        instance.name, instance.base, cube.light, cube.face, cube.layer
                    ));
                    *cube
                }
                [(label, None)] => {
                    return Err(format!(
                        "实例 '{}' 被 pass '{label}' 用了，而那条 pass 没有 `cube_face`：\
                         实例是要照某一面的 PassView 建的，没有那一面就没有依据",
                        instance.name
                    ));
                }
                many => {
                    let faces: Vec<String> = many
                        .iter()
                        .map(|(label, cube)| match cube {
                            Some(cube) => format!("{label}（灯 {}，面 {}）", cube.light, cube.face),
                            None => format!("{label}（没有 cube_face）"),
                        })
                        .collect();
                    return Err(format!(
                        "实例 '{}' 被 {} 条 pass 用了：{}。一个名字只能解析出**一套组** —— \
                         两处用、面不同就是「哪一面赢」说不清（那正是被否掉的优先级规则）",
                        instance.name,
                        many.len(),
                        faces.join(" / ")
                    ));
                }
            };
            let face_index = face_of(cube.light, cube.face)?;
            let base_index = resolved_materials
                .iter()
                .position(|material| material.name == instance.base)
                .ok_or_else(|| {
                    format!(
                        "实例 '{}' 照的那份 '{}' 不在材质表里（表里有：{}）",
                        instance.name,
                        instance.base,
                        resolved_materials
                            .iter()
                            .map(|material| material.name)
                            .collect::<Vec<_>>()
                            .join(" / ")
                    )
                })?;
            let base = &resolved_materials[base_index];
            let object_index = scene
                .objects
                .iter()
                .position(|object| object.id == instance.base);
            for pass in plan.passes.iter() {
                for draw in &pass.draws {
                    if draw.material != instance.name {
                        continue;
                    }
                    if draw.geometry != instance.base {
                        return Err(format!(
                            "pass '{}' 拿实例 '{}'（照 '{}'）去画几何 '{}'：名字是照**物体**\
                             生成的，而这一笔的剔除/混合/片元阶段来自 '{}'、几何与实例下标来自\
                             '{}' —— 两者指不同的物体时，这一笔就是「用 A 的网格跑 B 的材质档」",
                            pass.label,
                            instance.name,
                            instance.base,
                            draw.geometry,
                            instance.base,
                            draw.geometry
                        ));
                    }
                }
            }
            if object_index.is_none() {
                return Err(format!(
                    "实例 '{}' 照的是帧自有材质 '{}'：生成的实例只能照**物体**的材质\
                     （`material_instances` 是烘图侧按「哪个物体投影」生成的），\
                     照一份帧自有材质（天空盒那种）会静静地拿到它的剔除档去投影",
                    instance.name, instance.base
                ));
            }
            let groups = vec![ResolvedGroup {
                group: 1,
                bind_group: &face_stages[face_index].bind_group,
                layout: stage_layout.clone(),
                layout_id: STAGE_LAYOUT_ID,
                dynamic: false,
                dynamic_offset: 0,
            }];
            resolved_materials.push(ResolvedMaterial {
                name: instance.name.as_str(),
                groups,
                blend: base.blend,
                cull: base.cull,
                fragment_shader: base.fragment_shader,
                fragment_entry: base.fragment_entry,
            });
        }
    }
    Ok(resolved_materials)
}

fn all_passes(plan: &Plan, audit: &mut Vec<String>) -> Result<Plan, String> {
    let executed: Vec<&str> = plan.passes.iter().map(|pass| pass.label.as_str()).collect();
    audit.push(format!(
        "本切片执行的 pass：{}（{} / {}，**按文档全执行**）",
        executed.join(" → "),
        executed.len(),
        plan.passes.len()
    ));
    let plan = plan.clone();
    plan.check()
        .map_err(|err| format!("交给执行器的那一份计划说不通：{err}"))?;
    Ok(plan)
}

fn create_depth(device: &wgpu::Device, width: u32, height: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VertexInput {
    location: u32,
    format: wgpu::VertexFormat,
}

fn vertex_inputs(
    module: &naga::Module,
    entry_name: &str,
    label: &str,
) -> Result<Vec<VertexInput>, String> {
    let entry = module
        .entry_points
        .iter()
        .find(|entry| entry.name == entry_name && entry.stage == naga::ShaderStage::Vertex)
        .ok_or_else(|| {
            format!(
                "pass '{label}' 的顶点阶段里没有入口 '{entry_name}'（它有的入口：{}）",
                module
                    .entry_points
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            )
        })?;
    let mut inputs = Vec::new();
    for argument in &entry.function.arguments {
        let Some(naga::Binding::Location { location, .. }) = argument.binding else {
            continue;
        };
        let format = vertex_format(module, argument.ty).ok_or_else(|| {
            format!(
                "pass '{label}' 的顶点阶段在 location({location}) 上要的类型这一版认不出来：{:?}",
                module.types[argument.ty].inner
            )
        })?;
        inputs.push(VertexInput { location, format });
    }
    Ok(inputs)
}

fn check_vertex_inputs(
    inputs: &[VertexInput],
    label: &str,
    geometry: &Geometry,
) -> Result<(), String> {
    for input in inputs {
        let found = geometry
            .attributes
            .iter()
            .find(|attribute| attribute.shader_location == input.location);
        match found {
            Some(attribute) if attribute.format == input.format => {}
            Some(attribute) => {
                return Err(format!(
                    "pass '{label}' 的顶点阶段在 location({}) 上要 {:?}，\
                     而几何 '{}' 在那里给的是 {:?}：属性表与顶点阶段说的不是一件事",
                    input.location, input.format, geometry.name, attribute.format
                ));
            }
            None => {
                return Err(format!(
                    "pass '{label}' 的顶点阶段要 location({})（{:?}），\
                     而几何 '{}' 的属性表只有 [{}]：产物没带这个属性",
                    input.location,
                    input.format,
                    geometry.name,
                    geometry
                        .attributes
                        .iter()
                        .map(|attribute| format!(
                            "{}={:?}",
                            attribute.shader_location, attribute.format
                        ))
                        .collect::<Vec<_>>()
                        .join(" / ")
                ));
            }
        }
    }
    Ok(())
}

fn procedural_geometry(name: String) -> Geometry {
    Geometry {
        name,
        vertices: None,
        indices: None,
        attributes: Vec::new(),
        vertex_count: PROCEDURAL_VERTICES,
        index_count: 0,
    }
}

fn vertex_format(
    module: &naga::Module,
    ty: naga::Handle<naga::Type>,
) -> Option<wgpu::VertexFormat> {
    use naga::{ScalarKind, TypeInner, VectorSize};
    match &module.types[ty].inner {
        TypeInner::Scalar(scalar) => match (scalar.kind, scalar.width) {
            (ScalarKind::Float, 4) => Some(wgpu::VertexFormat::Float32),
            (ScalarKind::Uint, 4) => Some(wgpu::VertexFormat::Uint32),
            (ScalarKind::Sint, 4) => Some(wgpu::VertexFormat::Sint32),
            _ => None,
        },
        TypeInner::Vector { size, scalar } => match (scalar.kind, scalar.width, size) {
            (ScalarKind::Float, 4, VectorSize::Bi) => Some(wgpu::VertexFormat::Float32x2),
            (ScalarKind::Float, 4, VectorSize::Tri) => Some(wgpu::VertexFormat::Float32x3),
            (ScalarKind::Float, 4, VectorSize::Quad) => Some(wgpu::VertexFormat::Float32x4),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::material::TextureDimension;
    use px_protocol::scene::{Member, SceneSpec};

    fn spec_of(objects: &[&str], frames: &[&str]) -> SceneSpec {
        let object = |id: &str| {
            format!(
                r#"{{"id": "{id}",
                     "geometry": {{"source": "primitive", "name": "icosphere", "params": {{"radius": 1.0}}}},
                     "material": {{"shader": {{"graph": "shaders", "node": "surface", "key": "00"}}}}}}"#
            )
        };
        let frame = |name: &str| {
            format!(
                r#"{{"name": "{name}",
                     "shader": "@fragment fn fragment() -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}",
                     "entry": "fragment"}}"#
            )
        };
        let text = format!(
            r#"{{"schema": {}, "name": "夹具", "objects": [{}], "frame_materials": [{}]}}"#,
            px_protocol::SCENE_SCHEMA,
            objects
                .iter()
                .map(|id| object(id))
                .collect::<Vec<_>>()
                .join(","),
            frames
                .iter()
                .map(|name| frame(name))
                .collect::<Vec<_>>()
                .join(",")
        );
        serde_json::from_str(&text).expect("夹具文档")
    }

    #[test]
    fn the_sheet_is_four_columns_of_three_rows_for_twelve_cameras() {
        let mut spec = spec_of(&["planet"], &[]);
        spec.cameras = (0..12)
            .map(|index| px_protocol::art::Camera::raw([0.0, 0.0, 1.0], 3.15, format!("c{index}")))
            .collect();
        let (placements, target) =
            placements(Views::Sheet { columns: 4 }, &spec, 960, 640).expect("12 台相机排得下");
        assert_eq!(target, (3840, 1920));
        assert_eq!(placements.len(), 12);
        for (index, placement) in placements.iter().enumerate() {
            let column = (index % 4) as u32;
            let row = (index / 4) as u32;
            assert_eq!(
                placement.rect,
                Some([column * 960, row * 640, 960, 640]),
                "第 {index} 台相机落在第 {row} 行第 {column} 列"
            );
            assert_eq!(
                placement.uniform_viewport((960, 640)),
                [(column * 960) as f32, (row * 640) as f32, 960.0, 640.0]
            );
        }
    }

    #[test]
    fn a_single_view_has_no_cell() {
        let spec = spec_of(&["planet"], &[]);
        let (placements, target) = placements(Views::Single(None), &spec, 960, 640).unwrap();
        assert_eq!(target, (960, 640));
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].rect, None);
        assert_eq!(placements[0].viewport(), None, "单张那条路不设 viewport");
        assert_eq!(
            placements[0].uniform_viewport((960, 640)),
            [0.0, 0.0, 960.0, 640.0],
            "`view.viewport` 仍然是整幅（与加格子之前逐字节相同）"
        );
    }

    #[test]
    fn a_sheet_without_a_camera_table_names_the_real_reason() {
        let spec = spec_of(&["planet"], &[]);
        let Err(why) = placements(Views::Sheet { columns: 4 }, &spec, 960, 640) else {
            panic!("没有相机表时 --sheet 必须当场拒");
        };
        assert!(why.contains("产物自带的相机表"), "{why}");
        assert!(why.contains("px_scene::cameras::review()"), "要指路：{why}");
    }

    #[test]
    fn zero_columns_fall_back_to_one_column() {
        let mut spec = spec_of(&["planet"], &[]);
        spec.cameras = (0..12)
            .map(|index| px_protocol::art::Camera::raw([0.0, 0.0, 1.0], 3.15, format!("c{index}")))
            .collect();
        let (placements, target) =
            placements(Views::Sheet { columns: 0 }, &spec, 960, 640).unwrap();
        assert_eq!(target, (960, 7680));
        assert_eq!(placements[7].rect, Some([0, 7 * 640, 960, 640]));
    }

    #[test]
    fn a_material_name_resolves_to_exactly_one_table() {
        let spec = spec_of(&["planet", "atmosphere"], &["skybox"]);
        assert_eq!(
            material_table(&spec, "planet").expect("物体那张表"),
            MaterialTable::Objects(0)
        );
        assert_eq!(
            material_table(&spec, "atmosphere").expect("物体那张表"),
            MaterialTable::Objects(1)
        );
        assert_eq!(
            material_table(&spec, "skybox").expect("帧材质那张表"),
            MaterialTable::Frame(0)
        );
        assert!(
            material_table(&spec, "planet")
                .unwrap()
                .describe()
                .contains("物体")
        );
        assert!(
            material_table(&spec, "skybox")
                .unwrap()
                .describe()
                .contains("帧自有材质")
        );
    }

    #[test]
    fn a_name_in_neither_table_is_refused_with_all_lists() {
        let spec = spec_of(&["planet"], &["skybox"]);
        let err = material_table(&spec, "skybx").expect_err("三张表都没有 ⇒ 拒");
        assert!(err.contains("planet"), "要列出物体 id：{err}");
        assert!(err.contains("skybox"), "要列出帧自有材质：{err}");
        assert!(err.contains("生成的实例"), "要列出生成的实例表：{err}");
        assert!(err.contains("三张表里都没有"), "{err}");
    }

    #[test]
    fn a_name_in_both_tables_is_refused_as_ambiguity() {
        let spec = spec_of(&["planet"], &["planet"]);
        let err = material_table(&spec, "planet").expect_err("两张表都有 ⇒ 拒");
        assert!(err.contains("两张表"), "{err}");
        assert!(err.contains("物体 id"), "{err}");
        assert!(err.contains("帧自有材质"), "{err}");
    }

    fn frame_material(textures: &[(u32, TextureDimension)]) -> art::LoadedFrameMaterial {
        art::LoadedFrameMaterial {
            name: "skybox".to_string(),
            entry: "fragment".to_string(),
            assembled: String::new(),
            params: Vec::new(),
            textures: textures.to_vec(),
            version: 0,
        }
    }

    fn skybox_fixture(layers: u32) -> art::Skybox {
        let shape = px_protocol::art::TextureShape {
            width: 512,
            height: 512,
            layers,
            levels: 1,
            format: px_protocol::art::TextureFormat::Rgba8Srgb,
        };
        art::Skybox {
            sampler: px_protocol::scene::Sampler::clamped(),
            texture: art::LoadedTexture {
                member: Member::new("generated", "stars", "00"),
                shape,
                bytes: Vec::new(),
                image: image::RgbaImage::new(512, 512),
            },
        }
    }

    #[test]
    fn the_skybox_lands_on_the_only_slot_the_frame_material_declares() {
        let cube = [(5, TextureDimension::Cube)];
        let skybox = skybox_fixture(6);
        assert_eq!(
            skybox_slot(&frame_material(&cube), Some(&skybox)).expect("一格 ⇒ 就是它"),
            Some(5)
        );

        let err = skybox_slot(&frame_material(&[]), Some(&skybox)).expect_err("没有去处 ⇒ 拒");
        assert!(err.contains("没有去处"), "{err}");

        let two = [(5, TextureDimension::Cube), (7, TextureDimension::Cube)];
        let err = skybox_slot(&frame_material(&two), Some(&skybox)).expect_err("歧义 ⇒ 拒");
        assert!(
            err.contains("第 5 格") && err.contains("第 7 格"),
            "要列出那几格：{err}"
        );
        assert!(err.contains("没有任何依据"), "{err}");

        let flat = [(1, TextureDimension::D2)];
        let err = skybox_slot(&frame_material(&flat), Some(&skybox)).expect_err("维度不符 ⇒ 拒");
        assert!(err.contains("6 层"), "{err}");

        assert_eq!(
            skybox_slot(&frame_material(&[]), None).expect("两边都空"),
            None
        );
        let err = skybox_slot(&frame_material(&cube), None).expect_err("无处可绑 ⇒ 拒");
        assert!(err.contains("没有天空盒"), "{err}");
    }

    #[test]
    fn the_frame_material_state_follows_the_oracle_pipeline() {
        let key = material::frame_key(0x1234);
        assert_eq!(key.shader, 0x1234);
        assert_eq!(key.cull, material::cull_code(CullMode::None));
        assert_eq!(
            key.alpha,
            material::alpha_code(px_protocol::scene::AlphaMode::Opaque)
        );
        assert_eq!(
            key.blend(),
            None,
            "不混合：oracle 那条管线的 blend 就是 None"
        );
        assert_eq!(key.cull_face(), None, "两面都画");
    }

    #[test]
    fn procedural_geometry_has_no_buffers_and_three_vertices() {
        let geometry = procedural_geometry("skybox".to_string());
        assert!(geometry.vertices.is_none());
        assert!(geometry.indices.is_none());
        assert!(geometry.attributes.is_empty());
        assert_eq!(geometry.vertex_count, PROCEDURAL_VERTICES);
        assert_eq!(
            PROCEDURAL_VERTICES, 3,
            "oracle 的 `render_pass.draw(0..3, 0..1)`"
        );
        assert!(geometry.procedural(), "没有顶点缓冲**就是**程序化");
    }

    fn plan_pass(label: &str, kind: PassKind, writes: &[&str], render: &str) -> px_pass::PassPlan {
        px_pass::PassPlan {
            kind,
            label: label.to_string(),
            shader: String::new(),
            entry: String::new(),
            reads: Vec::new(),
            writes: writes.iter().map(|name| name.to_string()).collect(),
            texture_slots: None,
            params: Vec::new(),
            slots: Vec::new(),
            render: px_pass::RenderState::parse(render).expect("状态那几栏"),
            viewport: None,
            params_offset: 0,
            draws: Vec::new(),
            depth_target: None,
            layer: None,
            vertex_shader: String::new(),
            vertex_entry: String::new(),
        }
    }

    #[test]
    fn a_load_after_a_write_does_not_block_reuse() {
        let plan = Plan {
            passes: vec![
                plan_pass(
                    "opaque",
                    PassKind::Geometry,
                    &["scene_color_a"],
                    "color=clear(0,0,0,1)|depth=none|depth_write=true|compare=greater_equal|winding=ccw",
                ),
                plan_pass(
                    "sky",
                    PassKind::Geometry,
                    &["scene_color_a"],
                    "color=load|depth=none|depth_write=false|compare=greater_equal|winding=ccw",
                ),
            ],
            ..Default::default()
        };
        assert!(
            loads_before_write(&plan).is_empty(),
            "有人先写过了 ⇒ 池子可以复用"
        );
    }

    #[test]
    fn a_load_before_any_write_blocks_reuse() {
        let plan = Plan {
            passes: vec![plan_pass(
                "invert",
                PassKind::Geometry,
                &["scene_color_b"],
                "color=load|depth=none|depth_write=true|compare=greater_equal|winding=ccw",
            )],
            ..Default::default()
        };
        assert_eq!(loads_before_write(&plan), vec!["scene_color_b".to_string()]);
    }

    #[test]
    fn the_host_target_is_never_the_reason_to_refuse_reuse() {
        let plan = Plan {
            passes: vec![plan_pass(
                "blit",
                PassKind::Fullscreen,
                &[px_protocol::scene::VIEW_BUILTIN],
                "color=load|depth=none|depth_write=true|compare=greater_equal|winding=ccw",
            )],
            ..Default::default()
        };
        assert!(loads_before_write(&plan).is_empty(), "宿主目标由宿主自己清");
    }

    #[test]
    fn the_first_difference_points_at_the_line_and_shows_both_sides() {
        let left = "#import planet_x::light\nfn f() -> f32 { return 1.0; }\n";
        let right = "#import planet_x::light\nfn f() -> f32 { return 2.0; }\n";
        let why = first_difference(left, right);
        assert!(why.contains("第 2 行"), "{why}");
        assert!(why.contains("return 1.0"), "{why}");
        assert!(why.contains("return 2.0"), "{why}");
    }

    #[test]
    fn a_line_ending_difference_is_not_a_difference() {
        assert_eq!(
            first_difference("fn f() {}\n", "fn f() {}\r\n"),
            "逐行相同（只差行尾：那一栏已经被归一过了）"
        );
    }

    #[test]
    fn a_missing_tail_line_is_reported_at_its_line() {
        let why = first_difference("a\nb\nc\n", "a\nb\n");
        assert!(why.contains("第 3 行"), "{why}");
        assert!(why.contains("（没有这一行）"), "{why}");
    }

    #[test]
    fn a_copy_counts_as_a_write_for_the_depth_resource() {
        let mut copy = plan_pass(
            "copy_depth",
            PassKind::Copy,
            &["scene_depth_sample"],
            "color=none|depth=none|depth_write=true|compare=greater_equal|winding=ccw",
        );
        copy.reads = vec!["scene_depth".to_string()];
        let mut after = plan_pass(
            "clouds",
            PassKind::Geometry,
            &["scene_color_a"],
            "color=load|depth=none|depth_write=false|compare=greater_equal|winding=ccw",
        );
        after.reads = vec!["scene_depth_sample".to_string()];
        let good = Plan {
            passes: vec![copy.clone()],
            ..Default::default()
        };
        assert!(loads_before_write(&good).is_empty(), "拷贝是整张覆盖");

        let mut only_reader = plan_pass(
            "clouds",
            PassKind::Geometry,
            &["scene_color_a"],
            "color=clear(0,0,0,1)|depth=none|depth_write=false|compare=greater_equal|winding=ccw",
        );
        only_reader.reads = vec!["scene_depth_sample".to_string()];
        let only_reader = Plan {
            passes: vec![only_reader],
            ..Default::default()
        };
        assert!(
            loads_before_write(&only_reader).is_empty(),
            "读它不改变池子的内容"
        );

        let mut loader = copy.clone();
        loader.label = "shadow".to_string();
        loader.render = px_pass::RenderState::parse(
            "color=load|depth=none|depth_write=true|compare=greater_equal|winding=ccw",
        )
        .expect("状态");
        let plan = Plan {
            passes: vec![copy, loader],
            ..Default::default()
        };
        assert!(
            loads_before_write(&plan).is_empty(),
            "拷贝在前 ⇒ 这一格有内容"
        );
    }
}
