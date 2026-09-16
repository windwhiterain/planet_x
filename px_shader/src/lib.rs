//! shader include 闭包：模块发现、`#import` 解析、可达闭包与它的指纹（§17.1、§52.3）。
//!
//! 为什么要有这个 crate：`art/shaders/*.wgsl`（入口）里的 `#import planet_x::*` 是
//! **naga_oil 在运行期**按模块名组装的，模块的真本住在 `px_render/assets/shaders/*.wgsl`
//! （各带 `#define_import_path`）。于是「这份 shader 到底是什么」不由入口文本一个文件决定。
//! 而 PCG 那一侧的产物键（§17.1「键 = 内容」）只哈希了入口文本 ⇒ 改一个 include，
//! 键不动、清单不动、场景键不动、槽版本不动，画面却会变 —— 这是「同一个键、不同内容」。
//!
//! 这里把**一份**解析规则交出来，三个用户共用：烘图侧（`px_graphs` 的 `shaders` / `scene` 图
//! 程序，把闭包指纹算进产物键）、运行期（`px_render::shaders` 的模块表与 `reflect` 的库指纹）、
//! 以及门与探针。各写一份的实现迟早对不上，而"对不上"正是要消灭的那个故障。
//!
//! ⚠ 外部符号（`bevy_pbr::…`）**只记名字**：它们的实现由 Bevy / naga_oil 的版本决定，
//! 那是 `px_ops::SHADER_VERSION` 手动那一档（§19.1）。这条边界要写进报告，不能装作它不存在。
//!
//! 依赖方向：叶子 crate，不依赖本仓任何 crate（`px_ops` / `px_graphs` 与 `px_render` 都能用）。

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

/// 两个根：**库**（`<asset_root>/shaders`，`px_render/assets/shaders`）与**入口**
/// （`<workspace>/art/shaders`）。顺序不定语义：`module_sources` 按名字收，重名要报错。
///
/// `asset_root` 是渲染器那边可覆盖的资产根（`px_render::asset_root()`，找 `shaders/common.wgsl`
/// 的那个目录）；烘图侧直接给 `<workspace>/px_render/assets`。
pub fn roots(workspace: &Path, asset_root: &Path) -> Vec<PathBuf> {
    vec![
        asset_root.join("shaders"),
        workspace.join("art").join("shaders"),
    ]
}

/// **本仓的约定**：库在 `<workspace>/px_render/assets/shaders`，入口在 `<workspace>/art/shaders`。
/// 烘图侧直接用它；渲染器那边把可覆盖的 `asset_root()` 传进 `roots` ⇒ 同一份约定，两种取法
/// （烘图侧不知道运行期的资产根在哪，运行期也不该猜工作区在哪）。
pub fn workspace_roots(workspace: &Path) -> Vec<PathBuf> {
    roots(workspace, &workspace.join("px_render").join("assets"))
}

/// `workspace_roots` 那一份约定下的模块表。烘图侧一行拿到：`workspace_modules(&px_ops::workspace_root())`。
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
        (CLOSURE_HI.to_string(), f64::from((fingerprint >> 32) as u32)),
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
// FNV-1a（只做变更检测，不做安全：与 `px_ops::noise::fnv1a` 同一套常数）
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
        assert_eq!(module_of("planet_x::noise", &table), Some("planet_x::noise"));
        assert_eq!(module_of("bevy_pbr::forward_io::VertexOutput", &table), None);
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
            ("planet_x::noise", "#define_import_path planet_x::noise\nfn fbm_3() {}\n"),
            ("planet_x::other", "#define_import_path planet_x::other\nfn other() {}\n"),
        ]);
        let changed_unreachable = modules(&[
            ("planet_x::noise", "#define_import_path planet_x::noise\nfn fbm_3() {}\n"),
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
            ("planet_x::other", "#define_import_path planet_x::other\nfn other() {}\n"),
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
        let two = closure("#import bevy_pbr::mesh_view_bindings::{view, lights}\n", &table);
        let three = closure("#import bevy_pbr::mesh_view_bindings::{view, lights}\n", &table);
        assert_ne!(
            one.fingerprint(),
            two.fingerprint(),
            "点了哪些外部符号是这份 shader 的一部分"
        );
        assert_eq!(two.fingerprint(), three.fingerprint(), "同一份必须同一个指纹");
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
        for fingerprint in [0_u64, 1, 0x0000_0001_0000_0000, u64::MAX, 0xdead_beef_cafe_f00d] {
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
            "库根应当是 px_render/assets/shaders：{}",
            roots[0].display()
        );
        assert!(
            roots[1].join("clouds.wgsl").exists(),
            "入口根应当是 art/shaders：{}",
            roots[1].display()
        );
        let modules = workspace_modules(workspace).expect("模块表");
        for name in ["planet_x::common", "planet_x::light", "planet_x::noise"] {
            assert!(modules.contains_key(name), "库里应当有 {name}");
        }
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
        std::fs::write(entry.join("clouds.wgsl"), "#import planet_x::noise::fbm_3\n")
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
        assert_eq!(files, wgsl_files(&roots).expect("文件表"), "同一棵树两次要给同一张表");
        assert_eq!(
            closure("#import planet_x::noise::fbm_3\n", &table)
                .modules
                .len(),
            1
        );
        assert!(module_sources(&[root.join("not-there")]).is_err(), "目录不在 ⇒ Err");

        std::fs::write(
            entry.join("noise-again.wgsl"),
            "#define_import_path planet_x::noise\nfn fbm_3() { /* 另一份 */ }\n",
        )
        .expect("写不了夹具");
        let duplicate = module_sources(&roots).expect_err("同名两份必须报错");
        assert!(duplicate.contains("两个真本"), "报错要说清是两个真本：{duplicate}");
        std::fs::remove_file(entry.join("noise-again.wgsl")).expect("清不掉夹具");
    }
}
