//! See docs/shaders.md

pub mod assemble;
pub mod host_stubs;
pub mod reflect;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub type ModuleTable = BTreeMap<String, String>;

pub const IMPORT_PREFIX: &str = "#import ";
pub const MODULE_PREFIX: &str = "#define_import_path ";

const MODULES_NAMESPACE: &str = "px_shader/modules/v1";
const CLOSURE_NAMESPACE: &str = "px_shader/closure/v1";

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn workspace_roots(workspace: &Path) -> Vec<PathBuf> {
    vec![
        workspace.join("art").join("shaders").join("lib"),
        workspace.join("art").join("shaders"),
    ]
}

pub fn workspace_modules(workspace: &Path) -> Result<ModuleTable, String> {
    module_sources(&workspace_roots(workspace))
}

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

pub fn import_path_of(source: &str) -> Option<&str> {
    source
        .lines()
        .find_map(|line| line.trim().strip_prefix(MODULE_PREFIX).map(str::trim))
}

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Closure {
    pub modules: BTreeMap<String, String>,
    pub externals: BTreeSet<String>,
}

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

pub const CLOSURE_HI: &str = "closure_hi";
pub const CLOSURE_LO: &str = "closure_lo";

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

    #[test]
    fn the_four_entry_shaders_assemble_to_these_bytes_under_the_wgpu_host_table() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("px_shader 必须住在 workspace 下");
        let modules = workspace_modules(workspace).expect("模块表");
        let pinned = [
            (
                "atmosphere.wgsl",
                9528_usize,
                0xb222_07bb_e62e_b0fa_u64,
                0x7c31_cc92_7eab_ed01_u64,
            ),
            (
                "clouds.wgsl",
                59581,
                0x8a92_b256_5b14_290b,
                0x26d9_7a07_2b26_9596,
            ),
            (
                "ring.wgsl",
                20620,
                0x7bf8_9bc3_adbb_623e,
                0x8c39_408d_fc5e_5e7b,
            ),
            (
                "surface.wgsl",
                33589,
                0xe964_701b_8495_fe2d,
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
