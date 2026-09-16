use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use bevy::asset::io::AssetSourceBuilder;
use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::{AssetPath, AssetServer, Handle};
use bevy::prelude::*;
use bevy::shader::Shader;

use px_protocol::material::{PARAMS_BINDING, TEXTURE_SLOTS};

/// 槽资产源的名字。材质里的 `ShaderRef::Path` 指向 `slots://<槽>.<版本>.wgsl`。
pub const SOURCE: &str = "slots";

/// **通用材质的槽**：一份 WGSL = 一份材质。槽名不再代表"云 / 地表 / 大气"那种语义
/// —— 那正是通用渲染要去掉的东西（§65）：渲染器只认"产物给的这一份 WGSL 与它的绑定契约"。
pub const MATERIAL: &str = "material.wgsl";

/// 槽里最多同时养几份**版本**（占位不算）。超了就把最久没用过的那份放掉。
///
/// 为什么要有上界：每条活着的版本都是一份 WGSL 资产、外加它编出来的一整套管线
/// （正向 / prepass / 阴影 / 变体）。不放的话，一个长跑的会话来回切 N 版就攒 N 套。
/// 代价是**放掉的那一版下次用要重编** —— 所以 K 是「内存 ↔ 来回切不重编」的一个选择题，
/// 不是一个实现细节。
pub const MAX_LIVE_VERSIONS: usize = 4;

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

/// 占位材质的 WGSL：**由契约表生成**（`px_protocol::material`）。
///
/// 这一段原来是**手抄**那张表（§67.4 的第 2 处）⇒ 加宽超集时两边一起改、漏一处就是
/// 「布局里有这一格、占位 shader 里没有」。生成之后不存在这个问题：占位永远与表同形。
///
/// 组号写的是 `#{MATERIAL_BIND_GROUP}`（运行期由 Bevy 的 shader def 替成 3）：
/// 这份文本是**交给 Bevy 装**的，替数的事归它 —— 离线那边替的是同一个数
/// （`px_shader::assemble`），`the_placeholder_declares_exactly_the_table` 那条单测盯着这件事。
fn placeholder_material() -> String {
    /// 每条绑定声明的组前缀。写成 `#{}` 占位是**有意的**：这份文本交给 Bevy 装载，
    /// 由它的 shader def 替成运行期那个数（离线那边由 `px_shader::assemble` 替同一个）。
    const GROUP: &str = "@group(#{MATERIAL_BIND_GROUP})";
    let mut out = String::from("#import bevy_pbr::forward_io::VertexOutput\n\n");
    out.push_str("struct DocParams {\n    unused: f32,\n};\n\n");
    out.push_str(&format!(
        "{GROUP} @binding({PARAMS_BINDING}) var<uniform> params: DocParams;\n"
    ));
    for (index, (binding, dimension)) in TEXTURE_SLOTS.iter().enumerate() {
        out.push_str(&format!(
            "{GROUP} @binding({binding}) var texture{index}: {}<f32>;\n",
            dimension.name()
        ));
        out.push_str(&format!(
            "{GROUP} @binding({}) var sampler{index}: sampler;\n",
            binding + 1
        ));
    }
    out.push_str(
        "\n@fragment\nfn fragment(in: VertexOutput) -> @location(0) vec4<f32> {\n    \
         return vec4<f32>(1.0, 0.0, 1.0, 1.0);\n}\n",
    );
    out
}

pub fn placeholders() -> Vec<(&'static str, String)> {
    vec![(MATERIAL, placeholder_material())]
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
            dir.insert_asset_text(Path::new(&placeholder_file(slot)), &source);
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

    /// 槽名的形状：`material.wgsl` 这种自带后缀的槽，版本插在后缀前面。
    /// 认错槽名 = 装了一份没人看的 WGSL，所以这里只钉住**我们唯一的那个槽**的取文件名规则。
    #[test]
    fn the_only_slot_is_the_generic_material() {
        assert_eq!(MATERIAL, "material.wgsl");
        assert_eq!(placeholder_file(MATERIAL), "material.placeholder.wgsl");
        assert_eq!(version_file(MATERIAL, 0x1234), "material.0000000000001234.wgsl");
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
        assert_ne!(version_file(MATERIAL, va), version_file(MATERIAL, vb));
        assert_ne!(
            placeholder_file(MATERIAL),
            version_file(MATERIAL, va),
            "占位不能被某一版真本盖掉"
        );
        // 扩展名必须还是 .wgsl：加载器按扩展名认。
        assert!(version_file(MATERIAL, va).ends_with(".wgsl"));
        assert_eq!(version_file(MATERIAL, va), format!("material.{va:016x}.wgsl"));
        // 键不是 64 位十六进制 ⇒ 报错，不静默取前 16 位。
        assert!(version_of("deadbeef").is_err());
        assert!(version_of(&"z".repeat(64)).is_err());
    }

    /// 占位是**生成的**：它声明的绑定必须与契约表逐格相同 —— 这正是原来那张手抄表
    /// 会漂开的地方（§67.4 第 2 处）。这里把生成文本离线组装一遍再反射回来对账。
    #[test]
    fn the_placeholder_declares_exactly_the_table() {
        let source = placeholder_material();
        let modules = crate::shaders::module_sources();
        let mut seen = Vec::new();
        let assembled = crate::shaders::render_source(&source, &modules, &mut seen);
        assert!(
            !assembled.contains("#{MATERIAL_BIND_GROUP}"),
            "离线组装必须把组号替掉（替的就是运行期那个数）"
        );
        let layout = px_shader::reflect::reflect_assembled(&assembled, "占位").expect("反射占位");
        assert_eq!(
            layout.textures.len(),
            TEXTURE_SLOTS.len(),
            "占位声明的贴图格数必须等于表里的格数"
        );
        for (slot, (binding, dimension)) in layout.textures.iter().zip(TEXTURE_SLOTS.iter()) {
            assert_eq!(slot.binding, *binding);
            assert_eq!(slot.dimension, *dimension);
        }
        assert_eq!(
            layout.textures.len(),
            12,
            "加宽之后是 8 张 2D + 4 张 cube（§74.4 裁决 (a)）"
        );
    }
}
