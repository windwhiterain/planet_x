//! shader include 闭包：模块发现、`#import` 解析、可达闭包与它的指纹（§17.1、§52.3）。
//!
//! 为什么要有这个 crate：`art/shaders/*.wgsl`（入口）里的 `#import planet_x::*` 是
//! **naga_oil 在运行期**按模块名组装的，模块的真本住在 `art/shaders/lib/*.wgsl`
//! （各带 `#define_import_path`）。于是「这份 shader 到底是什么」不由入口文本一个文件决定。
//! 而 PCG 那一侧的产物键（§17.1「键 = 内容」）只哈希了入口文本 ⇒ 改一个 include，
//! 键不动、清单不动、场景键不动、槽版本不动，画面却会变 —— 这是「同一个键、不同内容」。
//!
//! 这里把**一份**解析规则交出来，三个用户共用：烘图侧（`px_graphs` 的 `shaders` / `scene` 图
//! 程序，把闭包指纹算进产物键）、运行期（`px_render::shaders` 的模块表与 `reflect` 的库指纹）、
//! 以及门与探针。各写一份的实现迟早对不上，而"对不上"正是要消灭的那个故障。
//! ⚠ 上面那个 `px_render::shaders` 是**已删的 Bevy 宿主**的落点（§154；`git show f121ee3^:…`）。
//!
//! ⚠ **S8-c 标注：运行期那半边的落点换了名字**（§154 删了 `px_render`）——
//! 那时（S8-c）运行期是 `px_render_wgpu::shader` 的模块表 + `px_shader::reflect` 的库指纹。
//! "三个用户"这条**分法**没变（烘图侧 / 运行期 / 门与探针），改的只是"运行期"是谁。
//! ⚠ **§157 取代（2026-09-19）：那个 crate 又改名叫 `px_render`** ⇒ 运行期今天是
//! `px_render::shader` 的模块表 + `px_shader::reflect` 的库指纹。⚠ 同一个名字在 §154–§156
//! 里指**已删的 Bevy 宿主**，从 §157 那一笔起指**现在的唯一宿主**（裸 wgpu）—— 读旧句按旧义读。
//!
//! ⚠ 外部符号（`bevy_pbr::…`）**只记名字**：它们的实现由 Bevy / naga_oil 的版本决定，
//! 那是 `px_graph::SHADER_VERSION` 手动那一档（§19.1）。这条边界要写进报告，不能装作它不存在。
//!
//! 依赖方向：**叶子 crate** —— 除了 `px_protocol`（只有类型，没有实现）不依赖本仓任何 crate，
//! 也不依赖 bevy / wgpu。`px_graph` / `px_graphs`（烘图侧）与 `px_render`（运行期）都能用它：
//! 烘图侧要在这里算键、组装、反射，运行期要在同一份规则下装载与对账。
//! ⚠ 这一句里的"`px_render`（运行期）"在 §157 之前指 Bevy 宿主、之后指 wgpu 宿主 —— 名字换过手，
//! 而"运行期"这个**角色**没换过（§157）。
//!
//! ⚠ **S8-c 标注：运行期那个 crate 当时叫 `px_render_wgpu`**（§154 删了 Bevy 宿主 `px_render`）。
//! 依赖方向的**约束**（叶子、不拖 bevy / wgpu）反而更强了 —— 今天它也**不拖 wgpu**。
//! ⚠ **§157（2026-09-19）：它改名叫 `px_render` 了** ⇒ 这一句今天读作"运行期 = `px_render`"。

pub mod assemble;
pub mod host_stubs;
pub mod reflect;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 模块表：`#define_import_path` 给的名字 → 源码。用 `BTreeMap` 而不是 `HashMap`：
/// 指纹要按名字排序才有确定的字节流（顺序不能影响键）。
pub type ModuleTable = BTreeMap<String, String>;

/// `#import <这条>` 的前缀（入口与模块里都可能有）。
pub const IMPORT_PREFIX: &str = "#import ";
/// 模块声明自己名字的那一行：只有库模块有，入口没有。
pub const MODULE_PREFIX: &str = "#define_import_path ";

/// 指纹的命名空间：换规则（比如以后把外部符号也哈希进去）就换一个，老指纹不会假装还成立。
const MODULES_NAMESPACE: &str = "px_shader/modules/v1";
const CLOSURE_NAMESPACE: &str = "px_shader/closure/v1";

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

// ---------------------------------------------------------------------------
// 根目录 / 文件发现
// ---------------------------------------------------------------------------

/// **本仓的约定**：库在 `<workspace>/art/shaders/lib`，入口在 `<workspace>/art/shaders`。
/// 两个根是**同一个契约的两半**，所以只有这一个函数说了算 —— 烘图侧（`px_graphs`）
/// 与宿主（`px_render`）都从它拿根；谁也不再自己拼一次路径。
///
/// ⚠ 为什么库**搬到 `art/shaders/lib/`**（S8-a）：它原来住 `px_render/assets/shaders`
/// —— 那是 **bevy 宿主那个 crate 的资产目录**，而库的内容与"谁来渲染"无关：烘图侧要它、
/// 裸 wgpu 宿主也要它。宿主删掉之后，"库住在某个宿主的资产目录里"就变成一句**没有宿主
/// 可指的话**。搬到艺术内容自己的根下面 ⇒ 库与入口同属一份内容，与宿主是谁无关。
///
/// ⚠ 这次搬家**不许动任何产物键**：闭包指纹哈希的是 `(模块名, 源码)` 对与外部符号名
/// （见 [`Closure::fingerprint`]），**从不含路径**；模块名来自文件里的 `#define_import_path`。
/// 所以纯搬家改不了任何键 —— 由回归集（J1/J2/J3 + 逃生门六份文件字节）复测证明，不是推的。
///
/// ⚠ 两个根都**只扫一层**（`wgsl_files` 不递归）：`lib/` 是子目录，所以入口根那一遍
/// 不会把库再收一次（收两次会在 `module_sources` 里撞成"两个真本"而当场报错）。
pub fn workspace_roots(workspace: &Path) -> Vec<PathBuf> {
    vec![
        workspace.join("art").join("shaders").join("lib"),
        workspace.join("art").join("shaders"),
    ]
}

/// `workspace_roots` 那一份约定下的模块表。烘图侧一行拿到：`workspace_modules(&px_graph::workspace_root())`。
pub fn workspace_modules(workspace: &Path) -> Result<ModuleTable, String> {
    module_sources(&workspace_roots(workspace))
}

/// 根目录下的全部 `.wgsl`（排序 ⇒ 结果与 `read_dir` 的顺序无关）。目录不在 ⇒ `Err`，不静默跳过。
pub fn wgsl_files(roots: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for root in roots {
        let entries = std::fs::read_dir(root)
            .map_err(|err| format!("读不了 shader 目录 {}：{err}", root.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("wgsl") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

// ---------------------------------------------------------------------------
// 解析
// ---------------------------------------------------------------------------

/// 这一份源码给模块起的名字（`#define_import_path planet_x::noise` → `planet_x::noise`）。
pub fn import_path_of(source: &str) -> Option<&str> {
    source
        .lines()
        .find_map(|line| line.trim().strip_prefix(MODULE_PREFIX).map(str::trim))
}

/// 入口里的每一条 `#import` 子句，**原样**（`::{a, b}` 不拆，`.` 与花括号都是它的内容）。
///
/// 不拆是有意的：外部符号那一条要记「点了哪些名字」，`{view, lights}` 与 `{view}` 必须不同。
pub fn imports_of(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix(IMPORT_PREFIX)
                .map(|rest| rest.trim().to_string())
        })
        .collect()
}

/// 一条 import 子句指向哪个模块：**最长模块名前缀**，与 naga_oil 的按名字解析同口径。
///
/// - `planet_x::noise::{fbm_3, rotate_vector}` → `planet_x::noise`
/// - `planet_x::light::sun_light` → `planet_x::light`
/// - `bevy_pbr::forward_io::VertexOutput` → `None`（外部符号，本仓没有这个模块）
///
/// ⚠ 整串本身是模块名时优先用它（`#import planet_x::common` 这种写法成立）。
pub fn module_of<'a>(import: &str, modules: &'a ModuleTable) -> Option<&'a str> {
    if let Some((name, _)) = modules.get_key_value(import) {
        return Some(name.as_str());
    }
    let mut rest = import;
    while let Some((head, _)) = rest.rsplit_once("::") {
        if let Some((name, _)) = modules.get_key_value(head) {
            return Some(name.as_str());
        }
        rest = head;
    }
    None
}

/// 两个根下的模块表：`#define_import_path` 给的名字 → 源码。
///
/// ⚠ 同名两处（两个文件声明同一个模块名）⇒ `Err`，不许先到先得：那种情况下一份实现会
/// 悄悄盖掉另一份，而"用的是哪一份"决定了键与画面能不能对上（与 `shader_source_of` 同口径）。
pub fn module_sources(roots: &[PathBuf]) -> Result<ModuleTable, String> {
    let mut modules: ModuleTable = BTreeMap::new();
    let mut owner: BTreeMap<String, PathBuf> = BTreeMap::new();
    for path in wgsl_files(roots)? {
        let source = std::fs::read_to_string(&path)
            .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
        let Some(name) = import_path_of(&source) else {
            continue;
        };
        if let Some(previous) = owner.get(name) {
            return Err(format!(
                "模块 '{name}' 有两个真本：{} 与 {} —— 名字必须唯一，否则键算的是这一份、装的是那一份",
                previous.display(),
                path.display()
            ));
        }
        owner.insert(name.to_string(), path);
        modules.insert(name.to_string(), source);
    }
    Ok(modules)
}

/// **入口** shader 的真本：在 [`workspace_roots`] 那两个根下按**文件名**找。
///
/// 为什么这条规则要住在共享 crate 里：找入口这一件事原先在 `px_render::shaders::shader_source_of`
/// 与 `px_render::shader::try_source_of` **各写了一遍**，而 `px_probe` 要是再写第三遍，
/// 「游标那样一份内容 shader 到底是盘上哪个文件」就会有三个答案。规则只有一条：
/// **按文件名找，找到 0 个或 ≥2 个都算失败** —— 重名不许先到先得，那会让两边各测一份。
///
/// 找不到 / 重名都返回 `Err`（不 panic）：装载期那条路要"当场拒"，热重载那条路要
/// "这一槽不重载，并说清为什么"，两者只差调用方拿到 `Err` 之后干什么（§147 那条口径）。
pub fn workspace_source_of(workspace: &Path, name: &str) -> Result<(String, PathBuf), String> {
    let roots = workspace_roots(workspace);
    let found: Vec<PathBuf> = wgsl_files(&roots)?
        .into_iter()
        .filter(|path| path.file_name().and_then(|value| value.to_str()) == Some(name))
        .collect();
    let places = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" / ")
    };
    match found.len() {
        1 => {
            let path = found.into_iter().next().expect("长度是 1");
            let text = std::fs::read_to_string(&path)
                .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
            Ok((text, path))
        }
        0 => Err(format!(
            "哪里都找不到 shader '{name}'（找过 {}）",
            places(&roots)
        )),
        _ => Err(format!(
            "shader '{name}' 在两处都有：{} —— 名字必须唯一，否则测的是这一份、用的是那一份",
            places(&found)
        )),
    }
}

// ---------------------------------------------------------------------------
// 闭包
// ---------------------------------------------------------------------------

/// 一份入口真正会**进管线**的东西：可达模块（名字 → 源码）+ 解不开的外部符号（只记子句）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Closure {
    pub modules: BTreeMap<String, String>,
    pub externals: BTreeSet<String>,
}

/// 从入口出发按 `#import` 走一遍可达闭包。
///
/// 只收**可达**的：`art/shaders/surface.wgsl` 改了不该让 `clouds` 换键（那是白烧重烘，
/// 而重烘要顺着场景键一路烧下去）。所以是闭包，不是"目录下所有文件"。
pub fn closure(entry: &str, modules: &ModuleTable) -> Closure {
    let mut closure = Closure::default();
    let mut queue: Vec<String> = imports_of(entry);
    while let Some(import) = queue.pop() {
        let Some(name) = module_of(&import, modules) else {
            closure.externals.insert(import);
            continue;
        };
        if closure.modules.contains_key(name) {
            continue;
        }
        let Some(source) = modules.get(name) else {
            continue;
        };
        queue.extend(imports_of(source));
        closure.modules.insert(name.to_string(), source.clone());
    }
    closure
}

impl Closure {
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty() && self.externals.is_empty()
    }

    /// 闭包指纹：可达模块的（名字, 源码）+ 外部符号的子句，长度前缀、按名字排序。
    /// 排序 ⇒ 与 `#import` 的书写顺序无关；长度前缀 ⇒ `("ab","c")` 与 `("a","bc")` 不撞。
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = Fnv::new(CLOSURE_NAMESPACE);
        for (name, source) in &self.modules {
            hasher.chunk(name.as_bytes());
            hasher.chunk(source.as_bytes());
        }
        for import in &self.externals {
            hasher.chunk(import.as_bytes());
        }
        hasher.finish()
    }

    /// 给日志/报告的一行。烘图侧每次都要打出来：闭包里有什么是"这次烘的是哪一份"的一部分。
    pub fn summary(&self) -> String {
        let modules = join_or_none(self.modules.keys());
        let externals = join_or_none(self.externals.iter());
        format!(
            "include 闭包 {:016x}｜可达模块 {}【{modules}】｜外部符号 {}【{externals}】",
            self.fingerprint(),
            self.modules.len(),
            self.externals.len(),
        )
    }
}

/// 整张模块表的指纹（不区分可达性）：给只想知道"库变没变"的缓存用（`px_render::reflect`）。
/// ⚠ 那个落点是**已删的 Bevy 宿主**的模块（§154；`git show f121ee3^:px_render/src/reflect.rs`）。
/// ⚠ S8-c 标注：S8-c 时那个落点叫 `px_render_wgpu`（§154 删了 Bevy 宿主 `px_render`）；
/// §157（2026-09-19）起那个 crate 又叫 `px_render` —— 但**它没有 `reflect` 模块**（反射在
/// `px_shader::reflect`），所以这一句里的 `px_render::reflect` **永远**指 Bevy 那支。
pub fn modules_fingerprint(modules: &ModuleTable) -> u64 {
    let mut hasher = Fnv::new(MODULES_NAMESPACE);
    for (name, source) in modules {
        hasher.chunk(name.as_bytes());
        hasher.chunk(source.as_bytes());
    }
    hasher.finish()
}

fn join_or_none<'a>(items: impl Iterator<Item = &'a String>) -> String {
    let joined = items.cloned().collect::<Vec<_>>().join(" / ");
    if joined.is_empty() {
        "（无）".to_string()
    } else {
        joined
    }
}

// ---------------------------------------------------------------------------
// 指纹进出清单参数
// ---------------------------------------------------------------------------

/// 闭包指纹在产物清单里的两个参数名。清单参数只有 f64（`BTreeMap<String, f64>`）⇒ 拆两半。
pub const CLOSURE_HI: &str = "closure_hi";
pub const CLOSURE_LO: &str = "closure_lo";

/// 64 位指纹 → 清单参数。f64 能精确表示 32 位整数，所以这条无损。
pub fn closure_params(fingerprint: u64) -> [(String, f64); 2] {
    [
        (
            CLOSURE_HI.to_string(),
            f64::from((fingerprint >> 32) as u32),
        ),
        (
            CLOSURE_LO.to_string(),
            f64::from((fingerprint & 0xffff_ffff) as u32),
        ),
    ]
}

/// 清单参数 → 指纹。缺项 / 不是 32 位整数 ⇒ `None`（旧产物没记过，调用方该当场拒，不许猜）。
pub fn closure_from_params(params: &BTreeMap<String, f64>) -> Option<u64> {
    let half = |value: &f64| -> Option<u32> {
        if *value >= 0.0 && value.fract() == 0.0 && *value <= f64::from(u32::MAX) {
            Some(*value as u32)
        } else {
            None
        }
    };
    let hi = half(params.get(CLOSURE_HI)?)?;
    let lo = half(params.get(CLOSURE_LO)?)?;
    Some((u64::from(hi) << 32) | u64::from(lo))
}

// ---------------------------------------------------------------------------
// FNV-1a（只做变更检测，不做安全：与 `px_graph::noise::fnv1a` 同一套常数）
// ---------------------------------------------------------------------------

struct Fnv(u64);

impl Fnv {
    fn new(namespace: &str) -> Self {
        let mut hasher = Self(FNV_OFFSET);
        hasher.chunk(namespace.as_bytes());
        hasher
    }

    fn chunk(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        for byte in bytes {
            self.byte(*byte);
        }
    }

    fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(FNV_PRIME);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, source)| (name.to_string(), source.to_string()))
            .collect()
    }

    fn fixture(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("px_shader 必须住在 workspace 下")
            .join("target")
            .join("px_shader-fixture")
            .join(name);
        std::fs::create_dir_all(&root).expect("建不了夹具目录");
        root
    }

    #[test]
    fn an_import_clause_names_its_module_by_the_longest_prefix() {
        let table = modules(&[
            ("planet_x::noise", "#define_import_path planet_x::noise\n"),
            ("planet_x::light", "#define_import_path planet_x::light\n"),
        ]);
        assert_eq!(
            module_of("planet_x::noise::{fbm_3, rotate_vector}", &table),
            Some("planet_x::noise")
        );
        assert_eq!(
            module_of("planet_x::light::sun_light", &table),
            Some("planet_x::light")
        );
        assert_eq!(
            module_of("planet_x::noise", &table),
            Some("planet_x::noise")
        );
        assert_eq!(
            module_of("bevy_pbr::forward_io::VertexOutput", &table),
            None
        );
        assert_eq!(
            module_of("bevy_pbr::mesh_view_bindings::{view, lights}", &table),
            None,
            "外部符号解不开 ⇒ 由调用方记进 externals，不许静默丢掉"
        );
    }

    #[test]
    fn the_closure_is_reachable_modules_plus_external_symbols() {
        let table = modules(&[
            (
                "planet_x::common",
                "#define_import_path planet_x::common\nfn shell_thickness() -> f32 { 1.0 }\n",
            ),
            (
                "planet_x::noise",
                "#import planet_x::common::shell_thickness\n#define_import_path planet_x::noise\nfn fbm_3() -> f32 { shell_thickness() }\n",
            ),
            (
                "planet_x::unused",
                "#define_import_path planet_x::unused\nfn nobody_calls_me() -> f32 { 2.0 }\n",
            ),
        ]);
        let entry = "#import planet_x::noise::fbm_3\n#import bevy_pbr::forward_io::VertexOutput\n";
        let closure = closure(entry, &table);
        assert_eq!(
            closure.modules.keys().collect::<Vec<_>>(),
            vec!["planet_x::common", "planet_x::noise"],
            "只收可达的：unused 不在闭包里"
        );
        assert_eq!(
            closure.externals.iter().cloned().collect::<Vec<_>>(),
            vec!["bevy_pbr::forward_io::VertexOutput"]
        );
        assert!(!closure.is_empty());
    }

    #[test]
    fn the_fingerprint_follows_the_reachable_closure_and_not_the_whole_directory() {
        let entry = "#import planet_x::noise::fbm_3\n";
        let base = modules(&[
            (
                "planet_x::noise",
                "#define_import_path planet_x::noise\nfn fbm_3() {}\n",
            ),
            (
                "planet_x::other",
                "#define_import_path planet_x::other\nfn other() {}\n",
            ),
        ]);
        let changed_unreachable = modules(&[
            (
                "planet_x::noise",
                "#define_import_path planet_x::noise\nfn fbm_3() {}\n",
            ),
            (
                "planet_x::other",
                "#define_import_path planet_x::other\nfn other() { /* 改了 */ }\n",
            ),
        ]);
        let changed_reachable = modules(&[
            (
                "planet_x::noise",
                "#define_import_path planet_x::noise\nfn fbm_3() { /* 改了 */ }\n",
            ),
            (
                "planet_x::other",
                "#define_import_path planet_x::other\nfn other() {}\n",
            ),
        ]);
        assert_eq!(
            closure(entry, &base).fingerprint(),
            closure(entry, &changed_unreachable).fingerprint(),
            "不可达的模块改了不该换键：那只会白烧重烘，而重烘顺场景键一路烧下去"
        );
        assert_ne!(
            closure(entry, &base).fingerprint(),
            closure(entry, &changed_reachable).fingerprint(),
            "可达的模块改了必须换键"
        );
    }

    #[test]
    fn external_symbols_are_part_of_the_fingerprint_by_name() {
        let table = modules(&[("planet_x::noise", "#define_import_path planet_x::noise\n")]);
        let one = closure("#import bevy_pbr::mesh_view_bindings::{view}\n", &table);
        let two = closure(
            "#import bevy_pbr::mesh_view_bindings::{view, lights}\n",
            &table,
        );
        let three = closure(
            "#import bevy_pbr::mesh_view_bindings::{view, lights}\n",
            &table,
        );
        assert_ne!(
            one.fingerprint(),
            two.fingerprint(),
            "点了哪些外部符号是这份 shader 的一部分"
        );
        assert_eq!(
            two.fingerprint(),
            three.fingerprint(),
            "同一份必须同一个指纹"
        );
    }

    #[test]
    fn the_fingerprint_does_not_depend_on_the_import_order() {
        let table = modules(&[
            ("planet_x::common", "#define_import_path planet_x::common\n"),
            ("planet_x::noise", "#define_import_path planet_x::noise\n"),
        ]);
        let first = closure(
            "#import planet_x::common::shell_thickness\n#import planet_x::noise::fbm_3\n",
            &table,
        );
        let swapped = closure(
            "#import planet_x::noise::fbm_3\n#import planet_x::common::shell_thickness\n",
            &table,
        );
        assert_eq!(first.fingerprint(), swapped.fingerprint());
    }

    #[test]
    fn length_prefixes_keep_the_concatenation_unambiguous() {
        let split_one = modules(&[
            ("ab", "#define_import_path ab\n"),
            ("c", "#define_import_path c\n"),
        ]);
        let split_two = modules(&[
            ("a", "#define_import_path a\n"),
            ("bc", "#define_import_path bc\n"),
        ]);
        assert_ne!(
            modules_fingerprint(&split_one),
            modules_fingerprint(&split_two),
            "（名字, 源码）拼在一起没有长度前缀就会撞上"
        );
    }

    #[test]
    fn the_fingerprint_survives_the_manifest_parameter_round_trip() {
        for fingerprint in [
            0_u64,
            1,
            0x0000_0001_0000_0000,
            u64::MAX,
            0xdead_beef_cafe_f00d,
        ] {
            let params: BTreeMap<String, f64> = closure_params(fingerprint).into_iter().collect();
            assert_eq!(closure_from_params(&params), Some(fingerprint));
        }
        assert_eq!(
            closure_from_params(&BTreeMap::new()),
            None,
            "旧产物没记过闭包 ⇒ None（调用方该当场拒，不许当 0 用）"
        );
    }

    #[test]
    fn the_workspace_convention_points_at_the_real_shader_roots() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("px_shader 必须住在 workspace 下");
        let roots = workspace_roots(workspace);
        assert!(
            roots[0].join("common.wgsl").exists(),
            "库根应当是 art/shaders/lib：{}",
            roots[0].display()
        );
        assert!(
            roots[1].join("clouds.wgsl").exists(),
            "入口根应当是 art/shaders：{}",
            roots[1].display()
        );
        // ⚠ 这一条判的是**约定**，不是"今天有几份文件"（2026-09-20 改）：
        //   原来这里写死 `10`（"库 3 份 + 入口 7 份"），于是**加一份 shader**
        //   （`art/shaders/gasgiant.wgsl` 就是这么加的）会让一条与数量无关的测试红 ——
        //   而它真正要守的两条是：① 一个根只扫一层 ⇒ 每个文件只被收一次
        //   （同一个模块以"两个真本"的形式出现会当场报错）；② **库带 `#define_import_path`、
        //   入口不带**（`--bin shaders` 判"是不是入口"用的就是这一条）。
        let files = wgsl_files(&roots).expect("文件表");
        let unique: std::collections::BTreeSet<&std::path::PathBuf> = files.iter().collect();
        assert_eq!(
            files.len(),
            unique.len(),
            "同一个 .wgsl 被两个根都收了一遍（差 {}）：{files:?}",
            files.len() - unique.len(),
        );
        let library_files = wgsl_files(&roots[..1]).expect("库根的文件表");
        assert!(
            !library_files.is_empty(),
            "库根一份 .wgsl 都没有：{}",
            roots[0].display()
        );
        for path in &library_files {
            let text = std::fs::read_to_string(path).expect("读库文件");
            assert!(
                import_path_of(&text).is_some(),
                "库根下的 {} 没有 `#define_import_path`（它会被当成一份入口去烘）",
                path.display()
            );
        }
        for path in files.iter().filter(|path| !library_files.contains(path)) {
            let text = std::fs::read_to_string(path).expect("读入口文件");
            assert!(
                import_path_of(&text).is_none(),
                "入口根下的 {} 带着 `#define_import_path`（它会被当成模块，而不是一份入口）",
                path.display()
            );
        }
        let modules = workspace_modules(workspace).expect("模块表");
        for name in ["planet_x::common", "planet_x::light", "planet_x::noise"] {
            assert!(modules.contains_key(name), "库里应当有 {name}");
        }
        // 入口查找也走同一份约定（`px_probe` 与宿主共用这一条）。
        let (text, path) = workspace_source_of(workspace, "clouds.wgsl").expect("入口真本");
        assert!(
            path.ends_with("art/shaders/clouds.wgsl"),
            "找到的应当是入口根下那一份：{}",
            path.display()
        );
        assert!(
            text.contains("#import planet_x::light::sun_light"),
            "读到的是入口那一份"
        );
        assert!(
            workspace_source_of(workspace, "nobody.wgsl").is_err(),
            "找不到就是 Err —— 那正是「这一槽不重载」要说清的事"
        );
    }

    #[test]
    fn module_discovery_reads_both_roots_and_refuses_a_duplicate_name() {
        let root = fixture("discovery");
        let library = root.join("library");
        let entry = root.join("entry");
        std::fs::create_dir_all(&library).expect("建不了库目录");
        std::fs::create_dir_all(&entry).expect("建不了入口目录");
        std::fs::write(
            library.join("noise.wgsl"),
            "#define_import_path planet_x::noise\nfn fbm_3() {}\n",
        )
        .expect("写不了夹具");
        std::fs::write(
            entry.join("clouds.wgsl"),
            "#import planet_x::noise::fbm_3\n",
        )
        .expect("写不了夹具");
        let roots = vec![library.clone(), entry.clone()];
        let table = module_sources(&roots).expect("模块表");
        assert_eq!(table.len(), 1, "入口没有 import_path，不算模块");
        assert!(table.contains_key("planet_x::noise"));
        let files = wgsl_files(&roots).expect("文件表");
        assert_eq!(files.len(), 2);
        assert!(
            files.windows(2).all(|pair| pair[0] <= pair[1]),
            "按路径排序 ⇒ 与 read_dir 的顺序无关"
        );
        assert_eq!(
            files,
            wgsl_files(&roots).expect("文件表"),
            "同一棵树两次要给同一张表"
        );
        assert_eq!(
            closure("#import planet_x::noise::fbm_3\n", &table)
                .modules
                .len(),
            1
        );
        assert!(
            module_sources(&[root.join("not-there")]).is_err(),
            "目录不在 ⇒ Err"
        );

        std::fs::write(
            entry.join("noise-again.wgsl"),
            "#define_import_path planet_x::noise\nfn fbm_3() { /* 另一份 */ }\n",
        )
        .expect("写不了夹具");
        let duplicate = module_sources(&roots).expect_err("同名两份必须报错");
        assert!(
            duplicate.contains("两个真本"),
            "报错要说清是两个真本：{duplicate}"
        );
        std::fs::remove_file(entry.join("noise-again.wgsl")).expect("清不掉夹具");
    }

    /// 四份**内容入口**在裸 wgpu 宿主那张桩表下组装出来的文本与它的 include 闭包：**钉住的读数**。
    ///
    /// ⚠ 为什么这一条是判据而不是"打印一下"：S8-b 要判的是"**改 `#import` 的名字**有没有动到
    /// 组装出来的字"。而组装器把 `#import` 那几行**整行丢掉**（它们不进产物，见
    /// [`assemble::render_source`]）⇒「文本不变」与「闭包不变」是两件事，必须**分开量**：
    ///
    /// - 字节数 + FNV：组装文本（`#import` 展开后的那一份，就是喂给 `create_shader_module` 的）；
    /// - 闭包指纹：**外部符号的名字在这里**（`Closure::fingerprint` 哈希的是 import 子句本身）
    ///   ⇒ 它同时是 `px_graph::shader_key` 的输入之一（`键 = WGSL 字节 ‖ 闭包指纹`）。
    ///
    /// ⚠ 这正是 S8-b 那条死结的读数：改一个外部符号的**名字**，组装文本可以一个字节都不动，
    /// 而闭包指纹**必然**变 ⇒ 产物键变 ⇒ `art/anchor/frozen/*.pxart` 里钉着的 shader 成员键变。
    /// 谁要动这些名字，这一条会告诉他"动的是哪一半"。
    ///
    /// ⚠ `ring.wgsl` 是**负对照**：它一个 `planet_x::` 模块都不引、外部符号只有 `VertexOutput`
    /// 一个（§154 记过：换桩表时它逐字节不变，就是因为它不引那几个差别符号）。
    #[test]
    fn the_four_entry_shaders_assemble_to_these_bytes_under_the_wgpu_host_table() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("px_shader 必须住在 workspace 下");
        let modules = workspace_modules(workspace).expect("模块表");
        // (入口, 组装文本字节数, 组装文本 FNV-1a, 闭包指纹)
        let pinned = [
            (
                "atmosphere.wgsl",
                9528_usize,
                0xb222_07bb_e62e_b0fa_u64,
                0x7c31_cc92_7eab_ed01_u64,
            ),
            (
                "clouds.wgsl",
                57537,
                0x6f86_5584_df03_ad4d,
                0x26d9_7a07_2b26_9596,
            ),
            (
                "ring.wgsl",
                18576,
                0x29db_0dbd_6674_8c6c,
                0x8c39_408d_fc5e_5e7b,
            ),
            (
                "surface.wgsl",
                31545,
                0xdd77_9069_e838_9bb7,
                0x8865_fd2a_ed6d_4b64,
            ),
        ];
        for (name, bytes, fnv, closure_fingerprint) in pinned {
            let (source, _path) = workspace_source_of(workspace, name).expect("入口真本");
            assert_eq!(
                closure(&source, &modules).fingerprint(),
                closure_fingerprint,
                "{name} 的 include 闭包变了 —— 外部符号的名字也在指纹里，\
                 而 `px_graph::shader_key` 拿它算产物键（改了它，冻在 art/anchor/frozen 的产物键就跟着变）"
            );
            let mut seen = Vec::new();
            let assembled = assemble::render_source(
                &source,
                &modules,
                crate::host_stubs::wgpu_host_stub,
                &mut seen,
            );
            assert_eq!(
                assembled.len(),
                bytes,
                "{name} 组装出来的字节数变了（喂给 create_shader_module 的就是这一份）"
            );
            let mut plain = Fnv(FNV_OFFSET);
            for byte in assembled.as_bytes() {
                plain.byte(*byte);
            }
            assert_eq!(plain.finish(), fnv, "{name} 组装出来的字节流变了");
        }
    }
}
