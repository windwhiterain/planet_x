use std::path::{Path, PathBuf};
use std::process::Command;

use px_graph_schema::PxOp;
use px_graph_schema::blake3;

pub const PACKAGE: &str = "px_inst";

pub const RECIPE: &str = "px_graphs/src/inst_recipe.rs";

const VERSION: &[u8] = b"px_inst/v1";

pub struct Inst<'a> {
    pub op_id: &'a str,
    pub interface: u64,
    pub decl_hash: &'a str,
    pub alg_roots: &'a [&'a str],
    pub source: &'a str,
    pub template: &'a str,
}

pub fn normalize_template(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[derive(Debug, Clone)]
pub struct InstInfo {
    pub op_id: String,
    pub interface: u64,
    pub decl_hash: String,
    pub alg_roots: Vec<String>,
    pub source: String,
    pub template: String,
    pub library: String,
}

pub fn info_of<O: PxOp>(op_id: &str, alg_roots: &[&str], source: &str, template: &str) -> InstInfo {
    InstInfo {
        op_id: op_id.to_string(),
        interface: O::interface(),
        decl_hash: O::decl_hash().to_string(),
        alg_roots: alg_roots.iter().map(|root| root.to_string()).collect(),
        source: source.to_string(),
        template: template.to_string(),
        library: library_path(&key_of::<O>(op_id, alg_roots, source, template).unwrap_or_default())
            .display()
            .to_string(),
    }
}

pub fn key_of<O: PxOp>(
    op_id: &str,
    alg_roots: &[&str],
    source: &str,
    template: &str,
) -> Result<String, String> {
    key_of_facts(
        op_id,
        O::interface(),
        O::decl_hash(),
        alg_roots,
        source,
        template,
    )
}

pub fn key_of_facts(
    op_id: &str,
    interface: u64,
    decl_hash: &str,
    alg_roots: &[&str],
    source: &str,
    template: &str,
) -> Result<String, String> {
    key(&Inst {
        op_id,
        interface,
        decl_hash,
        alg_roots,
        source,
        template,
    })
}

pub fn info_of_facts(
    op_id: &str,
    interface: u64,
    decl_hash: &str,
    alg_roots: &[&str],
    source: &str,
    template: &str,
) -> InstInfo {
    InstInfo {
        op_id: op_id.to_string(),
        interface,
        decl_hash: decl_hash.to_string(),
        alg_roots: alg_roots.iter().map(|root| root.to_string()).collect(),
        source: source.to_string(),
        template: template.to_string(),
        library: library_path(
            &key_of_facts(op_id, interface, decl_hash, alg_roots, source, template)
                .unwrap_or_default(),
        )
        .display()
        .to_string(),
    }
}

pub fn key(inst: &Inst<'_>) -> Result<String, String> {
    let root = crate::workspace_root();
    let mut roster = px_fingerprint::Roster::new();
    for alg in inst.alg_roots {
        let crate_dir = root.join(alg);
        let files = px_fingerprint::roster(&crate_dir);
        if files.is_empty() {
            return Err(format!(
                "实例 key：{} 里没有可数的源码（recipe 的 `roots` 那一栏把 crate 名字写错了？）",
                crate_dir.display()
            ));
        }
        for (label, path) in files {
            roster.insert(format!("{alg}::{label}"), path);
        }
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(VERSION);
    field(&mut hasher, px_graph_schema::TOOLCHAIN_HASH);
    field(&mut hasher, inst.decl_hash);
    field(&mut hasher, &px_fingerprint::hash(&roster));
    hasher.update(&inst.interface.to_le_bytes());
    field(&mut hasher, &normalize_template(inst.template));

    let source_path = root.join(inst.source);
    let source = std::fs::read(&source_path).map_err(|err| {
        format!(
            "实例 key：读不了泛型参数源 {}：{err}",
            source_path.display()
        )
    })?;
    hasher.update(&(source.len() as u64).to_le_bytes());
    hasher.update(&source);

    Ok(hasher.finalize().to_hex().to_string())
}

pub fn library_path(key: &str) -> PathBuf {
    crate::workspace_root()
        .join("target/pcg/inst")
        .join(format!("{key}{}", std::env::consts::DLL_SUFFIX))
}

pub fn generated_dir(key: &str) -> String {
    crate::workspace_root()
        .join("target/jit")
        .join(key)
        .display()
        .to_string()
        .replace('\\', "/")
}

pub fn symbol(decl: &str) -> String {
    format!("{PACKAGE}__{decl}")
}

pub trait InstNode {
    fn info() -> InstInfo;
}

#[derive(Default)]
pub struct BuildGraph {
    nodes: Vec<InstInfo>,
}

impl BuildGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inst<T: InstNode>(&mut self) -> &mut Self {
        let info = T::info();
        self.push(info)
    }

    pub fn facts(
        &mut self,
        name: &str,
        op_id: &str,
        interface: u64,
        decl_hash: &str,
        roots: &[&str],
        source: &str,
        body: &str,
    ) -> &mut Self {
        assert!(
            !self.nodes.iter().any(|seen| seen.op_id == op_id),
            "build graph 里有两条同 op id 的实例：{op_id}（{name}；id 必须唯一）",
        );
        self.push(info_of_facts(
            op_id, interface, decl_hash, roots, source, body,
        ))
    }

    fn push(&mut self, info: InstInfo) -> &mut Self {
        assert!(
            !self.nodes.iter().any(|seen| seen.op_id == info.op_id),
            "build graph 里有两条同 op id 的实例：{}（id 必须唯一）",
            info.op_id,
        );
        self.nodes.push(info);
        self
    }

    pub fn nodes(&self) -> &[InstInfo] {
        &self.nodes
    }

    pub fn into_nodes(self) -> Vec<InstInfo> {
        self.nodes
    }

    pub fn instances(&self) -> &[InstInfo] {
        &self.nodes
    }

    pub fn missing(&self) -> Plan {
        let mut absent = Vec::new();
        let mut present = 0_u32;
        for info in &self.nodes {
            if Path::new(&info.library).is_file() {
                present += 1;
            } else {
                absent.push(Missing {
                    op_id: info.op_id.clone(),
                    key: key_of_info(info),
                    library: info.library.clone(),
                });
            }
        }
        Plan {
            total: self.nodes.len(),
            present,
            missing: absent,
        }
    }

    pub fn compile_missing(&self, catalogue: &InstCatalogue) -> Result<Stage1, String> {
        let mut built = Vec::new();
        let mut already = 0_u32;
        for info in &self.nodes {
            let key = key_of_info(info);
            if Path::new(&info.library).is_file() {
                already += 1;
                continue;
            }
            let codegen = catalogue.for_op(&info.op_id)?;
            compile_one(info, &key, codegen)?;
            built.push(Missing {
                op_id: info.op_id.clone(),
                key,
                library: info.library.clone(),
            });
        }
        Ok(Stage1 {
            total: self.nodes.len(),
            built,
            already,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub total: usize,
    pub present: u32,
    pub missing: Vec<Missing>,
}

impl Plan {
    pub fn complete(&self) -> bool {
        self.missing.is_empty()
    }

    pub fn hint(&self, graph: &str) -> String {
        let keys: Vec<&str> = self
            .missing
            .iter()
            .map(|absent| absent.key.as_str())
            .collect();
        let driver = driver_command();
        format!(
            "{}\n  {}\n  \
             ⇒ 编它们（只跑 stage 1）：{driver} build\
             \n     或者两个 stage 连着跑：{driver} run {graph} --build",
            crate::fault::line(
                "missing-instance",
                &format!("graph={graph}"),
                &format!("缺 {} 条实例库（共 {} 条）", self.missing.len(), self.total),
            ),
            keys.join("\n  "),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Missing {
    pub op_id: String,
    pub key: String,
    pub library: String,
}

#[derive(Debug, Clone)]
pub struct Stage1 {
    pub total: usize,
    pub built: Vec<Missing>,
    pub already: u32,
}

impl Stage1 {
    pub fn summary(&self, failed: u32) -> String {
        format!(
            "共 {} 条：已编 {}、已有 {}、失败 {failed}",
            self.total,
            self.built.len(),
            self.already,
        )
    }
}

/// The pre-flight hard gate a graph program can call at startup. Callers are the
/// driver / graph-exe side only: implementation libraries must not reach that
/// `px_cook` at all, or the gate itself drags `px_cook/src` into every library's
/// roster and stops being free (same rule as the repair vocabulary, docs/invariants.md).
///
/// Two tiers, per docs/graph.md:
/// * Instance tier: `BuildGraph::missing()` is today's classification = the artifact
///   exists on disk. "Present but compiled against a different contract" is not
///   caught yet — that half needs the loader window and lands there.
/// * Named tier: `source_hash(lib)` is the full `open()` handshake (dlopen +
///   contract + toolchain), so a stale named library is refused BEFORE the first
///   cooked node, with the same memoized entry reused when cooking actually loads.
///   Caller passes the named libraries this graph uses; a library left out falls
///   back to today's mid-run error, never to a wrong answer.
pub fn gate_ready(graph: Option<(&str, &BuildGraph)>, libs: &[&'static str]) -> Result<(), String> {
    let mut failures: Vec<String> = Vec::new();
    if let Some((name, build_graph)) = graph {
        let plan = build_graph.missing();
        if !plan.complete() {
            failures.push(plan.hint(name));
        }
    }
    for lib in libs {
        if let Err(err) = px_graph_schema::ops::source_hash(lib) {
            failures.push(format!("{lib}: {err}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "archive gate:{}
",
            failures.join(
                "
"
            )
        ))
    }
}

pub fn live_keys(graph: &BuildGraph) -> Vec<String> {
    graph
        .instances()
        .iter()
        .map(|info| key_of_info(info))
        .collect()
}

pub fn key_of_info(info: &InstInfo) -> String {
    Path::new(&info.library)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string()
}

#[derive(Debug, Clone)]
pub enum InstKind {
    Decl {
        schema: &'static str,
        module: &'static str,
        decl: &'static str,
    },
    Element {
        ty: &'static str,
        params: &'static str,
        inputs: &'static str,
        payload: &'static str,
    },
}

impl InstKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Decl { decl, .. } => decl,
            Self::Element { ty, .. } => ty,
        }
    }

    pub fn decl_crate(&self) -> &'static str {
        match self {
            Self::Decl { schema, .. } => schema,
            Self::Element { .. } => "px_elem",
        }
    }

    pub fn schema(&self) -> Option<&'static str> {
        match self {
            Self::Decl { schema, .. } => Some(schema),
            Self::Element { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstCodegen {
    pub op_id: &'static str,
    pub kind: InstKind,
    pub source: &'static str,
    pub body: &'static str,
    pub recipe_line: u32,
    pub recipe: &'static str,
}

#[derive(Debug, Clone)]
pub struct InstCatalogue {
    pub entries: Vec<InstCodegen>,
}

impl InstCatalogue {
    pub fn from_generated(entries: &[InstCodegen]) -> Self {
        Self {
            entries: entries.to_vec(),
        }
    }

    pub fn insert(&mut self, codegen: InstCodegen) {
        assert!(
            !self.entries.iter().any(|seen| seen.op_id == codegen.op_id),
            "catalogue 里有两条同 op id 的记录：{}（id 必须唯一）",
            codegen.op_id,
        );
        self.entries.push(codegen);
    }

    pub fn for_op(&self, op_id: &str) -> Result<&InstCodegen, String> {
        self.entries
            .iter()
            .find(|entry| entry.op_id == op_id)
            .ok_or_else(|| {
                format!(
                    "catalogue 里没有 op id 为 {op_id} 的记录\
                     \n  ⇒ `px_graphs::insts::codegen()` 没登记它：声明那一档来自 `{RECIPE}`\
                     \n     （改过表就跑 `cargo build` 重新生成），element 那一档来自 \
                     `px_elem::ELEM_SPECS`"
                )
            })
    }
}

pub fn compile_one(info: &InstInfo, key: &str, codegen: &InstCodegen) -> Result<(), String> {
    match compile_generated(info, key, codegen) {
        Ok(()) => Ok(()),
        Err(err) => Err(format!(
            "✗ 这个实例编不过：op id {}\
             \n  · 体（body）来自 {}:{}（recipe 里那一条的 `body` 一栏）\
             \n  · 参数文件 {}（生成物里是 include! 进去的 ⇒ 报告里的 \
             target/jit/{key}/src/lib.rs:<行> 对应它）\
             \n  · 生成物：{}（留着，不删）\
             \n{err}",
            codegen.op_id,
            codegen.recipe,
            codegen.recipe_line,
            codegen.source,
            generated_dir(key),
        )),
    }
}

fn compile_generated(info: &InstInfo, key: &str, codegen: &InstCodegen) -> Result<(), String> {
    let root = crate::workspace_root();
    let symbol = symbol(codegen.kind.name());
    let dir = root.join("target/jit").join(key);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).map_err(|err| format!("建不了 {}：{err}", src.display()))?;

    let manifest = manifest_text(&root, &info.alg_roots, &codegen.kind)?;
    write(&dir.join("Cargo.toml"), &manifest)?;
    write(&src.join("lib.rs"), &lib_text(&root, codegen)?)?;
    write(&dir.join("build.rs"), BUILD_RS)?;

    println!(
        "生成 {}（体来自 {}:{}）",
        dir.display(),
        codegen.recipe,
        codegen.recipe_line,
    );
    cargo_build(&dir)?;

    let name = format!(
        "{}px_inst{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let nested = root.join("target/jit/target");
    let produced = [
        nested.join(profile_dir()),
        nested.join(inst_env("PX_TARGET")).join(profile_dir()),
    ]
    .into_iter()
    .map(|dir| dir.join(&name))
    .find(|path| path.is_file());
    let Some(produced) = produced else {
        return Err(format!(
            "编译过了，但 {} 不在盘上（`--target`/profile 那一层目录写错了？）",
            nested.join(profile_dir()).join(&name).display()
        ));
    };
    let library = PathBuf::from(&info.library);
    if let Some(parent) = library.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("建不了 {}：{err}", parent.display()))?;
    }
    std::fs::copy(&produced, &library).map_err(|err| {
        format!(
            "拷 {} → {} 失败：{err}",
            produced.display(),
            library.display()
        )
    })?;

    let sidecar = library.with_extension("json");
    write(&sidecar, &sidecar_text(info, key, &symbol, codegen))?;
    Ok(())
}

pub fn manifest_text(root: &Path, roots: &[String], kind: &InstKind) -> Result<String, String> {
    if roots.is_empty() {
        return Err(
            "这条实例没登记任何根（`inst::InstInfo::alg_roots` 是空的）—— 泛型体住哪个 crate？\
             （那就是 `docs/operators.md` 里的**边**）"
                .to_string(),
        );
    }
    let graph_schema = slash(&crate_path(root, "px_graph_schema")?);
    let fingerprint = slash(&crate_path(root, "px_fingerprint")?);
    let schema = kind.schema();
    let schema_line = match schema {
        Some(name) => format!(
            "{name} = {{ path = \"{}\" }}\n",
            slash(&crate_path(root, name)?)
        ),
        None => String::new(),
    };
    let mut deps = String::new();
    for name in roots {
        if name != "px_graph_schema" && Some(name.as_str()) != schema {
            deps.push_str(&format!(
                "{name} = {{ path = \"{}\" }}\n",
                slash(&crate_path(root, name)?)
            ));
        }
    }
    Ok(format!(
        "# 生成物（`px build` 写的）。别手改 —— 改 art/inst 里的源文件，然后重跑。\n\
         [package]\n\
         name = \"{PACKAGE}\"\n\
         version = \"0.0.0\"\n\
         edition = \"2024\"\n\
         \n\
         [lib]\n\
         crate-type = [\"dylib\"]\n\
         \n\
         [dependencies]\n\
         px_graph_schema = {{ path = \"{graph_schema}\" }}\n\
         {schema_line}{deps}\
         \n\
         [build-dependencies]\n\
         px_fingerprint = {{ path = \"{fingerprint}\" }}\n\
         \n\
         # 自成 workspace 根：别挂进主 workspace、也别共用它的 target 目录。\n\
         [workspace]\n",
    ))
}

pub fn lib_text(root: &Path, codegen: &InstCodegen) -> Result<String, String> {
    let source = root.join(codegen.source);
    if !source.is_file() {
        return Err(format!("泛型参数源不在盘上：{}", source.display()));
    }
    let (head, body_block) = match &codegen.kind {
        InstKind::Decl {
            schema,
            module,
            decl,
        } => (
            format!(
                "// 复用的那个声明（`{schema}::{module}::{decl}`）——引进来当 `{decl}`：\n\
                 #[allow(unused_imports)]\n\
                 pub use {schema}::{module}::{decl};\n"
            ),
            format!(
                "px_graph_schema::px_body! {{\n\
                 \x20   {decl},\n\
                 \x20   |p, i| {}\n\
                 }}\n",
                codegen.body,
            ),
        ),
        InstKind::Element {
            ty,
            params,
            inputs,
            payload,
        } => (
            String::new(),
            format!(
                "// ⚠ **裸体**：不引任何算子类型（引了就把驱动链进来）——符号名与三个类型直接给。\n\
                 px_graph_schema::px_body_raw! {{\n\
                 \x20   {ty},\n\
                 \x20   {params},\n\
                 \x20   {inputs},\n\
                 \x20   {payload},\n\
                 \x20   |p, i| {}\n\
                 }}\n",
                codegen.body,
            ),
        ),
    };
    Ok(format!(
        "//! 生成物（`px build` 写的）。别手改 —— 改 {} 里的源文件，然后重跑。\n\
         \n\
         {head}\
         include!(\"{}\");\n\
         \n\
         {body_block}\
         \n\
         px_graph_schema::px_impl_lib!();\n",
        codegen.source,
        slash(&source),
    ))
}

const BUILD_RS: &str = "//! 实例库自己的身份（源码指纹 + 工具链指纹）—— 与各实现库同一份算法。\n\
                        fn main() {\n\
                        \x20   px_fingerprint::cargo_fingerprint_for_crate();\n\
                        }\n";

/// The sidecar is consumed by tools that read it back as JSON (`px list` compares the recorded
/// `toolchain` against the current build), so it must parse: the comma goes **between** fields and
/// the closing brace is bare.
pub fn sidecar_text(info: &InstInfo, key: &str, symbol: &str, codegen: &InstCodegen) -> String {
    let fields = [
        ("key", key.to_string()),
        ("op_id", info.op_id.clone()),
        ("decl", codegen.kind.name().to_string()),
        ("decl_crate", codegen.kind.decl_crate().to_string()),
        ("alg", info.alg_roots.join(",")),
        ("source", info.source.clone()),
        ("template", codegen.body.to_string()),
        ("toolchain", px_graph_schema::TOOLCHAIN_HASH.to_string()),
        ("symbols", symbol.to_string()),
    ];
    let mut out = String::from("{\n");
    for (index, (name, value)) in fields.iter().enumerate() {
        let comma = if index + 1 == fields.len() { "" } else { "," };
        out.push_str(&format!("  \"{name}\": {}{comma}\n", json_string(value)));
    }
    out.push_str("}\n");
    out
}

pub fn profile_dir() -> &'static str {
    match inst_env("PX_PROFILE") {
        "debug" => "debug",
        other => other,
    }
}

pub fn inst_env(name: &str) -> &'static str {
    match name {
        "PX_PROFILE" => env!("PX_PROFILE"),
        "PX_TARGET" => env!("PX_TARGET"),
        "PX_RUSTFLAGS" => env!("PX_RUSTFLAGS"),
        "PX_CARGO" => env!("PX_CARGO"),
        other => panic!(
            "px_cook::inst 只认识 PX_PROFILE / PX_TARGET / PX_RUSTFLAGS / PX_CARGO，不认识 {other}"
        ),
    }
}

pub fn cargo_build(dir: &Path) -> Result<(), String> {
    let manifest = dir.join("Cargo.toml");
    let target = dir.parent().unwrap_or(dir).join("target");
    let mut command = Command::new(inst_env("PX_CARGO"));
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .arg("--profile")
        .arg(match inst_env("PX_PROFILE") {
            "debug" => "dev",
            other => other,
        });
    let triple = inst_env("PX_TARGET");
    let host = env!("PX_RUSTC_VERSION")
        .lines()
        .find_map(|line| line.trim().strip_prefix("host: "))
        .unwrap_or_default()
        .to_string();
    if !triple.is_empty() && triple != host {
        command.arg("--target").arg(triple);
    }
    let rustflags = inst_env("PX_RUSTFLAGS");
    if !rustflags.is_empty() {
        command.env("RUSTFLAGS", rustflags);
    }
    let output = command
        .output()
        .map_err(|err| format!("跑不了 cargo：{err}"))?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(format!(
            "cargo build 失败（{}）：{}",
            output.status,
            manifest.display()
        ));
    }
    Ok(())
}

pub fn missing_hint(key: &str) -> String {
    format!(
        "{}\n     库：{}",
        crate::fault::line(
            "missing-instance",
            &format!("key={key}"),
            &format!(
                "这条实例还没编 ⇒ `{driver} build`（只编缺的那几条）",
                driver = driver_command()
            ),
        ),
        library_path(key).display(),
    )
}

pub fn driver_command() -> String {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
        .map(|dir| dir.join(format!("px{}", std::env::consts::EXE_SUFFIX)))
        .filter(|path| path.is_file());
    match sibling {
        Some(path) => path.display().to_string(),
        None => "cargo run -q -p px_graphs --bin px --".to_string(),
    }
}

pub fn recipe_line_of(body: &str) -> u32 {
    let path = crate::workspace_root().join(RECIPE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    for (index, line) in text.lines().enumerate() {
        if line.contains(body) {
            return index as u32 + 1;
        }
    }
    0
}

fn crate_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root.join(name);
    if !path.join("Cargo.toml").is_file() {
        return Err(format!(
            "{} 不是一个 workspace crate（`px_inst` 里的 crate 名写错了？）",
            path.display()
        ));
    }
    Ok(path)
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|err| format!("写不了 {}：{err}", path.display()))
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn field(hasher: &mut blake3::Hasher, text: &str) {
    hasher.update(&(text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
}
