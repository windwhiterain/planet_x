use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use bevy::asset::io::AssetSourceBuilder;
use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::{AssetPath, AssetServer, Handle};
use bevy::prelude::*;
use bevy::shader::Shader;

/// 槽资产源的名字。材质里的 `ShaderRef::Path` 指向 `slots://<槽>.<版本>.wgsl`。
pub const SOURCE: &str = "slots";

pub const CLOUDS: &str = "clouds.wgsl";
pub const ATMOSPHERE: &str = "atmosphere.wgsl";
pub const SURFACE: &str = "surface.wgsl";
/// **通用材质的槽**（`crate::material::DocMaterial`）：一份 WGSL = 一份材质，
/// 槽名不代表"云 / 地表 / 大气"那种语义。S4 把旧三个槽删掉之后就只剩它一个。
pub const MATERIAL: &str = "material.wgsl";

/// 有 WGSL 真本的槽名。三个槽全在这张表里：行星表面从 §39.6 阶段 3 起也自写材质了
/// （`StandardMaterial` 没有"只压直接光"的位置，云影进不去），所以没有"走 Bevy 内建材质"
/// 的槽了 —— 每个 part 都必须把真本一起带上。
pub const WGSL_SLOTS: [&str; 4] = ["clouds", "atmosphere", "surface", "material"];

/// 认识的槽名全集。认错槽 = 装了个没人看的 shader，或者一块该有内容的壳画的是占位 ——
/// 所以两个方向的错配（该带的没带、不该带的带了）都报错，不猜。
pub const ALL_SLOTS: [&str; 4] = ["clouds", "atmosphere", "surface", "material"];

/// 每个槽最多同时养几份**版本**（占位不算）。超了就把最久没用过的那份放掉。
///
/// 为什么要有上界：每条活着的版本都是一份 WGSL 资产、外加它编出来的一整套管线
/// （正向 / prepass / 阴影 / 变体）。不放的话，一个长跑的会话来回切 N 版就攒 N 套。
/// 代价是**放掉的那一版下次用要重编** —— 所以 K 是「内存 ↔ 来回切不重编」的一个选择题，
/// 不是一个实现细节。
pub const MAX_LIVE_VERSIONS: usize = 4;

/// 这个 part 该往槽里装哪个文件？`has_shader` = 产物里带没带 `shader` 成员。
/// 认识的槽一律要有真本：没有 WGSL 的槽只能画占位（洋红），那不是能出图的状态。
pub fn wgsl_for(slot: &str, has_shader: bool) -> Result<Option<String>, String> {
    if WGSL_SLOTS.contains(&slot) {
        if !has_shader {
            return Err(format!(
                "槽 '{slot}' 没有 shader 成员：没有 WGSL 的槽只能画出占位（洋红），\
                 场景产物必须把真本一起带上"
            ));
        }
        return Ok(Some(format!("{slot}.wgsl")));
    }
    Err(format!(
        "不认识的 shader 槽 '{slot}'；这份渲染器认：{}",
        ALL_SLOTS.join(" / ")
    ))
}

// ---------------------------------------------------------------------------
// 版本号 / 文件名
// ---------------------------------------------------------------------------

/// 内容版本号 = 场景产物里那个 64 位十六进制内容键的前 16 位。
/// 键 = 内容（§17.1），所以同一个号永远是同一份字节。
pub fn version_of(key: &str) -> Result<u64, String> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "shader 成员的内容键应当是 64 位十六进制，实际是 '{key}'"
        ));
    }
    u64::from_str_radix(&key[..16], 16).map_err(|err| format!("内容键 '{key}' 解不开：{err}"))
}

/// 槽名自带 `.wgsl` 后缀（`clouds.wgsl`），版本插在后缀前面：`clouds.<16 位版本>.wgsl`。
/// 保留 `.wgsl` 后缀是必须的：加载器按扩展名认。
fn file_name(slot: &str, tag: &str) -> String {
    match slot.strip_suffix(".wgsl") {
        Some(stem) => format!("{stem}.{tag}.wgsl"),
        None => format!("{slot}.{tag}"),
    }
}

pub fn placeholder_file(slot: &str) -> String {
    file_name(slot, "placeholder")
}

pub fn version_file(slot: &str, version: u64) -> String {
    file_name(slot, &format!("{version:016x}"))
}

fn as_path(file: &str) -> AssetPath<'static> {
    AssetPath::from_path_buf(PathBuf::from(file)).with_source(SOURCE)
}

pub fn placeholder_path(slot: &str) -> AssetPath<'static> {
    as_path(&placeholder_file(slot))
}

pub fn version_path(slot: &str, version: u64) -> AssetPath<'static> {
    as_path(&version_file(slot, version))
}

// ---------------------------------------------------------------------------
// 活着的版本表
// ---------------------------------------------------------------------------

/// 为什么是全局的：`Material::specialize` 是个**静态**函数（Bevy 的材质钩子全静态：
/// `fragment_shader()` 没有 `self`），它既拿不到 `World` 也拿不到 `AssetServer`，
/// 而它偏偏是唯一能按**这份材质**改管线描述符的地方。所以句柄只能从进程内的一张表里取。
#[derive(Default)]
struct Live {
    /// (槽, 版本) → 句柄。**强引用**：握着它这一份资产就不会被卸载，管线也就还在。
    handles: HashMap<(String, u64), Handle<Shader>>,
    /// LRU：最近用过的在尾。淘汰从头部走。
    order: VecDeque<(String, u64)>,
}

fn live() -> &'static Mutex<Live> {
    static LIVE: OnceLock<Mutex<Live>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(Live::default()))
}

fn lock() -> std::sync::MutexGuard<'static, Live> {
    live().lock().unwrap_or_else(|err| err.into_inner())
}

/// 这一份版本的句柄（没有就是 **None** —— 那意味着材质该继续用占位，不许猜）。
/// 顺带记一次使用，给 LRU 用。
pub fn version_handle(slot: &str, version: u64) -> Option<Handle<Shader>> {
    let mut live = lock();
    let key = (slot.to_string(), version);
    let handle = live.handles.get(&key)?.clone();
    live.order.retain(|old| old != &key);
    live.order.push_back(key);
    Some(handle)
}

pub fn live_versions(slot: &str) -> Vec<u64> {
    let live = lock();
    let mut out: Vec<u64> = live
        .handles
        .keys()
        .filter(|(name, _)| name == slot)
        .map(|(_, version)| *version)
        .collect();
    out.sort_unstable();
    out
}

/// 把一份版本挂上：**只在第一次见到这个版本时**建一个 shader 资产。
///
/// 关键：**不 reload**。新路径 = 新资产 = 新 id ⇒ 旧版本的资产一个字节都没动，
/// 它的管线原封不动留在缓存里；来回切 A→B→A 时 A 是缓存命中，不会重编。
///
/// ⚠ 也**不走路径装载**（`server.load(path)`）。那条路是异步的：IO 任务读完 → 主世界的
/// 资产事件系统收下 → 下一帧才被渲染世界抽走。材质在**下一帧**就要特化管线，而
/// `PipelineCache` 一旦发现 shader 不在 `ShaderCache` 里就判 `ShaderNotLoaded` —— 那是**终态**，
/// 资产后来到了也不重试 ⇒ 整条管线永久失败、所有请求被拒。P32 实测这条路是**随机**翻车的
/// （同样的代码，上一次五档扫描过了、下一次在同一档崩）。`AssetServer::add` 是同步的，
/// 资产当场就有了，把这段时序整个抹掉。`wgsl` 里的 `#import` 全是绝对模块名
/// （`planet_x::common` / `bevy_pbr::…`），所以路径串只当身份用，不需要能解析出文件。
///
/// 返回 `true` = 这一版是**第一次**见（管线要现编，调用方该等它）；`false` = 已经在养，零动作。
pub fn activate(server: &AssetServer, slot: &str, version: u64, wgsl: &str) -> bool {
    let mut live = lock();
    let key = (slot.to_string(), version);
    if live.handles.contains_key(&key) {
        live.order.retain(|old| old != &key);
        live.order.push_back(key);
        return false;
    }
    let handle = server.add(Shader::from_wgsl(
        wgsl.to_string(),
        version_path(slot, version).to_string(),
    ));
    live.handles.insert(key.clone(), handle);
    live.order.push_back(key);
    while live.order.len() > MAX_LIVE_VERSIONS {
        let Some(old) = live.order.pop_front() else {
            break;
        };
        if live.handles.remove(&old).is_some() {
            // 强引用一放，资产被卸载，Bevy 的 `PipelineCache` 收到 `Removed` 会
            // 把靠它的管线一并删掉（`pipeline_cache.rs` 的 `remove_shader`）——
            // 不会留下指向死资产的管线。下次再用这一版要重编，代价记在这里。
            println!(
                "shader 槽 {}.{}：超过 {MAX_LIVE_VERSIONS} 份，放掉最久没用过的那一份",
                old.0,
                &format!("{:016x}", old.1)[..8]
            );
        }
    }
    true
}

// ---------------------------------------------------------------------------
// 占位
// ---------------------------------------------------------------------------

const PLACEHOLDER_CLOUDS: &str = r#"#import bevy_pbr::forward_io::VertexOutput

struct CloudParams {
    orientation: vec4<f32>,
    tint: vec4<f32>,
    inner: f32,
    outer: f32,
    density: f32,
    coverage: f32,
    base: f32,
    top: f32,
    detail_scale: f32,
    detail_strength: f32,
    erode: f32,
    phase: f32,
    shadow: f32,
    steps: u32,
    bump: f32,
    seed: u32,
    ablate: u32,
    slope_scale: f32,
    taper: f32,
    coverage_gain: f32,
    surface_level: f32,
    bound: u32,
    gradient: u32,
    wind: f32,
    wind_skin: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: CloudParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var coverage_map: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var coverage_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
"#;

const PLACEHOLDER_ATMOSPHERE: &str = r#"#import bevy_pbr::forward_io::VertexOutput

struct AtmosphereParams {
    inner: f32,
    outer: f32,
    density: f32,
    softness: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: AtmosphereParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
"#;

const PLACEHOLDER_SURFACE: &str = r#"#import bevy_pbr::forward_io::VertexOutput

struct SurfaceParams {
    orientation: vec4<f32>,
    emissive: vec4<f32>,
    inner: f32,
    outer: f32,
    coverage: f32,
    shadow: f32,
    height: f32,
    gain: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SurfaceParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var albedo_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var glow_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var glow_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var coverage_map: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var coverage_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
"#;

/// 通用材质的占位：**同一套绑定**（0 = 参数块、1/3 = 2D、5/7 = cube，采样器在 +1）、洋红。
/// 它的声明形状就是 `crate::reflect` 里那张表 —— 占位与真本布局不一致的话，
/// 启动时那几条预热管线会直接编不出来。
const PLACEHOLDER_MATERIAL: &str = r#"#import bevy_pbr::forward_io::VertexOutput

struct DocParams {
    unused: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: DocParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var texture0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var sampler0: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var texture1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var sampler1: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var texture2: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var sampler2: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var texture3: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var sampler3: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
"#;

pub fn placeholders() -> [(&'static str, &'static str); 4] {
    [
        (CLOUDS, PLACEHOLDER_CLOUDS),
        (ATMOSPHERE, PLACEHOLDER_ATMOSPHERE),
        (SURFACE, PLACEHOLDER_SURFACE),
        (MATERIAL, PLACEHOLDER_MATERIAL),
    ]
}

/// 槽的内容住在内存目录里：没有占位文件，真本由场景产物在运行时装进来。
/// `Dir` 内部是 `Arc<RwLock<..>>`，所以改写对已经建好的 reader 立刻可见。
#[derive(Resource, Clone)]
pub struct ShaderSlots {
    dir: Dir,
}

impl ShaderSlots {
    pub fn seeded() -> Self {
        let dir = Dir::new(PathBuf::from("slots"));
        for (slot, source) in placeholders() {
            dir.insert_asset_text(Path::new(&placeholder_file(slot)), source);
        }
        Self { dir }
    }

    pub fn dir(&self) -> &Dir {
        &self.dir
    }

    /// 现在槽里装的是哪一份（返回版本号；`None` = 还没装过真本，画的是占位）。
    /// 只用来打日志 —— 判断该不该动的逻辑在 `activate` 里（那张表才是真相）。
    pub fn peek_version(&self, slot: &str) -> Option<u64> {
        let live = lock();
        live.order
            .iter()
            .rev()
            .find(|(name, _)| name == slot)
            .map(|(_, version)| *version)
    }
}

pub struct SlotsPlugin;

impl Plugin for SlotsPlugin {
    fn build(&self, app: &mut App) {
        let slots = ShaderSlots::seeded();
        let dir = slots.dir.clone();
        app.register_asset_source(
            SOURCE,
            AssetSourceBuilder::new(move || Box::new(MemoryAssetReader { root: dir.clone() })),
        );
        app.insert_resource(slots);
        app.add_systems(PreStartup, seed_slot_shaders);
    }
}

/// 启动时先把两个**占位**槽 load 一遍：材质还没出现时管线就要能建起来，
/// 这一刻场景还没来 ⇒ 槽里是占位（绑定一致、颜色是洋红，一眼看得出不是成品）。
/// 这一份占位管线同时也是"某个场景还没装真本"时的兜底画面。
fn seed_slot_shaders(server: Res<AssetServer>) {
    for (slot, _) in placeholders() {
        let _handle: Handle<Shader> = server.load(placeholder_path(slot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wgsl_slot_without_its_member_is_an_error_not_a_placeholder() {
        assert_eq!(
            wgsl_for("clouds", true).unwrap().as_deref(),
            Some("clouds.wgsl")
        );
        assert_eq!(
            wgsl_for("atmosphere", true).unwrap().as_deref(),
            Some("atmosphere.wgsl")
        );
        assert_eq!(
            wgsl_for("surface", true).unwrap().as_deref(),
            Some("surface.wgsl")
        );
        for slot in WGSL_SLOTS {
            let missing = wgsl_for(slot, false).unwrap_err();
            assert!(missing.contains("占位"), "要说清画出来的是什么：{missing}");
        }
    }

    #[test]
    fn every_slot_takes_a_shader_member_and_no_other_name_is_known() {
        assert_eq!(WGSL_SLOTS, ALL_SLOTS, "现在没有走内建材质的槽了");
        let unknown = wgsl_for("lava", true).unwrap_err();
        for slot in ALL_SLOTS {
            assert!(unknown.contains(slot), "报错要列出认识的槽：{unknown}");
        }
    }

    /// 版本号必须只由内容键决定，而且**不同内容必须是不同的路径** ——
    /// 路径撞了就等于把两版塞进同一个 asset，那正是"换档必重编"的老毛病。
    #[test]
    fn different_shader_content_means_a_different_slot_path() {
        let a = "d9a3ea0c4d75e41aa86356dbd48e9156f86e9f8f13086dd8e13f4fdb4b4c4205";
        // 手写 50 个 0 太容易数错（这份夹具真数错过一次），按长度拼出来。
        let b = format!("829920a54cbb{}ff", "0".repeat(50));
        assert_eq!(a.len(), 64);
        assert_eq!(b.len(), 64, "测试自己的夹具要够长");
        let va = version_of(a).unwrap();
        let vb = version_of(&b).unwrap();
        assert_ne!(va, vb, "内容不同 ⇒ 版本不同");
        assert_ne!(version_file(CLOUDS, va), version_file(CLOUDS, vb));
        assert_ne!(
            placeholder_file(CLOUDS),
            version_file(CLOUDS, va),
            "占位不能被某一版真本盖掉"
        );
        // 扩展名必须还是 .wgsl：加载器按扩展名认。
        assert!(version_file(CLOUDS, va).ends_with(".wgsl"));
        assert_eq!(version_file(CLOUDS, va), format!("clouds.{va:016x}.wgsl"));
        // 键不是 64 位十六进制 ⇒ 报错，不静默取前 16 位。
        assert!(version_of("deadbeef").is_err());
        assert!(version_of(&"z".repeat(64)).is_err());
    }
}
