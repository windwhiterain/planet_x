//! **stage 1 的「计划」段** —— 图程序这一侧**代码生成**（`21-codegen-types.md`）。
//!
//! 这个 build script 干两件事，**都不调 cargo、都不编任何东西**（硬边界）：
//!
//! 1. 算**本图程序**这一份源码的指纹（`PX_SOURCE_HASH`）—— 从前就有的那一件；
//! 2. **执行 stage 1 的计划段**：读 `src/inst_recipe.rs`（一张数据表）→ 校验 → 用
//!    `px_decls`（声明表）拿类型级事实 → 算 key → 写 `OUT_DIR/insts_gen.rs`：
//!    **每条实例一个「生成出来的类型」**（`PxOp` / `InstNode` impl，事实为 const）。
//!
//! ⚠ **stage 2 用的类型 = 这一段生成出来的**（§目标）：`src/insts.rs` 把生成物并进本模块，
//!   于是"图侧类型"不再手写、不再有宏。
//! ⚠ **内容不变就不重写那个文件**（保 mtime）：改 `art/inst/*.rs` 时生成物必须**一个字节不变**
//!   —— 不然 cargo 会重编图程序、七个图 exe 全换 ⇒ R1 破。
//! ⚠ **绝不调 cargo**：编译实例只能由 `px build` / `px run --build` 触发（`20` §182）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use px_cook::inst::{self, RECIPE};

fn main() {
    // ① 图程序自己的源码指纹（从前就有；`px_local_op!` 那一档的身份用它）。
    px_fingerprint::cargo_fingerprint_for_crate(&[]);
    // ② 重跑条件：recipe 与声明表（`px_decls`）都要显式声明 —— cargo 只自动盯本 package
    //    自己的 `src/`，`[build-dependencies]` 的 path 依赖会重跑本脚本，但 recipe 不会自己盯。
    let root = workspace_root();
    println!("cargo:rerun-if-changed={}", root.join(RECIPE).display());
    let mut decl_files = Vec::new();
    px_cook::inst_scan::collect_rs(&root.join("px_decls"), &mut decl_files);
    decl_files.sort();
    for file in &decl_files {
        println!("cargo:rerun-if-changed={}", file.display());
    }

    let plan = plan_instances(&root).unwrap_or_else(|err| panic!("{err}"));
    let generated = insts_gen_text(&plan).unwrap_or_else(|err| panic!("{err}"));
    let catalogue = catalogue_text(&plan);

    // 生成物写到 `OUT_DIR`。⚠ `insts_gen_catalogue.rs` 是**给工具/门的旁挂件**（`px build` 的
    //   错误映射与端到端测试读它），不参与图程序的编译。
    write_if_changed(&out_dir().join("insts_gen.rs"), &generated);
    write_if_changed(&out_dir().join("insts_gen_catalogue.rs"), &catalogue);
}

/// workspace 根：build script 的 `CARGO_MANIFEST_DIR` 是 `px_graphs/`。
fn workspace_root() -> PathBuf {
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("没有 CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .expect("px_graphs 没有父目录")
}

fn out_dir() -> PathBuf {
    PathBuf::from(std::env::var("OUT_DIR").expect("没有 OUT_DIR"))
}

/// **内容不变就不写**（保 mtime）—— R1 的那一条硬要求。
fn write_if_changed(path: &Path, text: &str) {
    if let Ok(old) = std::fs::read_to_string(path) {
        if old == text {
            return;
        }
    }
    std::fs::write(path, text).unwrap_or_else(|err| panic!("写不了 {}：{err}", path.display()));
}

/// 一条实例的**全部**生成期事实（生成物与旁挂件都从它来）。
struct Planned {
    op_id: String,
    type_name: String,
    decl: String,
    roots: Vec<String>,
    source: String,
    /// recipe 那一栏**原文**（`…, ARG`）—— key 的一轴、也是 `InstInfo.template`。
    template: String,
    /// 抄进生成物的体（`ARG` 已换成 `&<类型名>`，见 [`InstRecipe::generated_body`]）。
    body: String,
    key: String,
    schema: String,
    module: String,
    params: String,
    inputs: String,
    payload: String,
    /// key 的一轴，也是生成物里那句 `{interface},` 的字面量。
    interface: u64,
    /// key 的一轴，也是生成物里 `decl_hash()` 的字面量。
    decl_hash: String,
    recipe_line: u32,
    generated: String,
}

/// **stage 1 的计划段**：读 recipe → 校验 → 用声明表算 key。
///
/// 任何一条不过就回 `Err`（`main` 把它 panic 出去）—— **报错必须点名是哪一条 recipe**。
fn plan_instances(root: &Path) -> Result<Vec<Planned>, String> {
    let recipe_path = root.join(RECIPE);
    let recipe_text = std::fs::read_to_string(&recipe_path)
        .map_err(|err| format!("读不了 {}：{err}", recipe_path.display()))?;
    let instances = parse_recipe(&recipe_text)?;

    let members: BTreeSet<String> = {
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
        px_cook::inst_scan::workspace_members(&manifest)
            .into_iter()
            .collect()
    };

    let mut planned = Vec::new();
    let mut seen_ids: BTreeSet<&str> = BTreeSet::new();
    for (index, item) in instances.iter().copied().enumerate() {
        let at = || format!("recipe 第 {} 条（{}）", index + 1, item.op_id);
        // ① op id 唯一。
        if !seen_ids.insert(item.op_id) {
            return Err(format!(
                "{}：op id 重复（每条实例的 id 必须唯一 —— 工具按它认领）",
                at()
            ));
        }
        // ② 声明在表里。
        let facts = px_decls::decl(item.decl).ok_or_else(|| {
            format!(
                "{}：声明 `{}` 不在声明表里（`px_decls`）\
                 \n  ⇒ 要么名字写错了，要么那是一个新的 `px_op!` —— 后者要先在 \
                 `px_decls/src/lib.rs` 的 `decl()` 里加一臂",
                at(),
                item.decl
            )
        })?;
        // ③ 根都是 workspace 成员目录。
        for root_name in item.roots {
            if !members.contains(*root_name) {
                return Err(format!(
                    "{}：根 `{root_name}` 不是 workspace 成员\
                     \n  ⇒ 它要住在这个 workspace 的 `members` 里（泛型参数 `use` 不到它）",
                    at()
                ));
            }
        }
        // ④ 源文件在盘上、且里面**真的定义了** `type_name`（文本检查：这就是"生成物引用的
        //    那个类型在不在"）。
        let source_path = root.join(item.source);
        if !source_path.is_file() {
            return Err(format!(
                "{}：源文件不在盘上：{}",
                at(),
                source_path.display()
            ));
        }
        let source_text = std::fs::read_to_string(&source_path)
            .map_err(|err| format!("{}：读不了 {}：{err}", at(), source_path.display()))?;
        if !defines(&source_text, item.type_name) {
            return Err(format!(
                "{}：{} 里没有定义类型 `{}`\
                 \n  ⇒ 生成物会把它并进实例库、再引用那个名字 ⇒ 编不过。改 recipe 或改那个文件",
                at(),
                item.source,
                item.type_name
            ));
        }
        // ⑤ 算 key —— 走 `px_cook::inst` 那一套**同一个**输入、**同一个**函数。
        //    ⚠ 进 key 的是 recipe 那一栏**原文**（`…, ARG`）：key 的算法一个字都不许改
        //      （`21-codegen-types.md` 铁律 3）。`&<类型名>` 那一份只进生成物（下一步）。
        let roots: Vec<&str> = item.roots.to_vec();
        let key = inst::key_of_facts(
            item.op_id,
            facts.interface,
            facts.decl_hash,
            &roots,
            item.source,
            item.body,
        )
        .map_err(|err| format!("{}：{err}", at()))?;
        let body = item.generated_body();
        // ⚠ 这个路径的口径只在 `px_cook::inst` 一处（[`inst::generated_dir`]）：element 那几条
        //   由 `px_graphs::insts::codegen()` 在运行期现造 `InstCodegen`，两处必须给同一个字符串
        //   —— 而且两边都从 `px_cook` 的 `workspace_root()` 出发（`root` 这一格是 build script
        //   自己按 `CARGO_MANIFEST_DIR` 算的，同一个目录）。
        let generated = inst::generated_dir(&key);
        planned.push(Planned {
            op_id: item.op_id.to_string(),
            type_name: item.type_name.to_string(),
            decl: item.decl.to_string(),
            roots: item.roots.iter().map(|root| root.to_string()).collect(),
            source: item.source.to_string(),
            template: item.body.to_string(),
            body,
            key,
            schema: facts.schema.to_string(),
            module: facts.module.to_string(),
            params: facts.params.to_string(),
            inputs: facts.inputs.to_string(),
            payload: facts.payload.to_string(),
            interface: facts.interface,
            decl_hash: facts.decl_hash.to_string(),
            recipe_line: recipe_text
                .lines()
                .position(|line| line.contains(item.body))
                .map(|at| at as u32 + 1)
                .unwrap_or(0),
            generated,
        });
    }
    Ok(planned)
}

/// **recipe 那一张表**：`InstRecipe` / `INSTANCES`。
///
/// ⚠ 它是**图侧的字面量源码** —— 当普通模块读进来（与 `src/insts.rs` 看见的是同一份）。
///   它**只许有类型定义 + 字面量**：有任何逻辑都会在这里跑一遍。
/// ⚠ 路径基准是 **`build.rs` 那一格**（`px_graphs/`）⇒ `src/inst_recipe.rs`；`src/insts.rs`
///   在 `src/` 里、写的是 `inst_recipe.rs` —— 同一份文件，两处基准不同。
/// ⚠ 走 `#[path]` 而不是把文件正文并进来：后者是"抄一份共享源码"的形状，`px_graph` 的
///   `source_hash.rs` 那道门正当地禁止它（共享助手是 crate，不许有副本）。`#[path]` 没有副本。
/// ⚠ 这个文件的注释一律 `//`（不是 `//!`）：在 `src/insts.rs` 那一侧它排在 `use` 之后
///   ⇒ 只能是普通注释（实测内层文档注释在那儿报 `expected outer doc comment`；
///   同一个原因，`#[path] mod` 在 `//!` 存在时也会报 `file not found for module recipe`）。
#[path = "src/inst_recipe.rs"]
mod recipe;

use recipe::{INSTANCES, InstRecipe};

/// 表非空那道校验（`INSTANCES` 由上面那个模块读进来 ⇒ 这里只做"是不是空的"）。
fn parse_recipe(_text: &str) -> Result<Vec<&'static InstRecipe>, String> {
    if INSTANCES.is_empty() {
        return Err(format!(
            "{RECIPE} 里一条实例都没有 —— 至少要有图侧真正在用的那些（`insts::build` 取的就是它）"
        ));
    }
    Ok(INSTANCES.iter().collect())
}

/// `text` 里有没有**定义** `name`（`struct X` / `enum X` / `type X` / `trait X` / `union X`）。
///
/// ⚠ 词典法检查（够用就行）：它要回答的是"被并进实例库之后 `X` 在不在作用域里"，
///   而那个文件是本仓自己写的一小段（`art/inst/*.rs`）。注释与字符串先跳掉。
fn defines(text: &str, name: &str) -> bool {
    let stripped = strip_comments(text);
    for keyword in ["struct", "enum", "union", "trait"] {
        if stripped.contains(&format!("{keyword} {name}")) {
            return true;
        }
        // `pub struct X` / `pub(crate) struct X` 都落上面那条；`struct X;`（单元结构体）也是。
        if stripped.contains(&format!("{keyword}\t{name}")) {
            return true;
        }
    }
    stripped.contains(&format!("type {name} ")) || stripped.contains(&format!("type {name}="))
}

/// 去注释（`//` 与 `/* */`）—— 只在定义检查里用。
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(b"//") {
            match text[at..].find('\n') {
                Some(offset) => at += offset + 1,
                None => break,
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            match text[at..].find("*/") {
                Some(offset) => {
                    at += offset + 2;
                    out.push(' ');
                }
                None => break,
            }
            continue;
        }
        let ch = text[at..].chars().next().unwrap_or(' ');
        out.push(ch);
        at += ch.len_utf8();
    }
    out
}

/// **`OUT_DIR/insts_gen.rs`**：每条实例一个**生成出来的类型**（`PxOp` / `InstNode` impl）。
///
/// ⚠ 形状照 `px_cook` 里 `px_inst!` 从前展开出来的那一份（`19` §179.1 的说明）：
///   `LIB` / `SYMBOL` 是字面量、三个关联类型是**全路径**、`decl_hash()` 是 const、
///   `source_hash()` = 实例 key（它要读源文件字节 ⇒ 只有运行期算得出）。
/// ⚠ `SYMBOL` 与 `inst::symbol` 必须同口径 —— 生成器用 `inst::symbol` **算出来**再写进去，
///   于是两边不可能不一致。
fn insts_gen_text(plan: &[Planned]) -> Result<String, String> {
    let mut out = String::from(
        "// **生成物**（`px_graphs/build.rs` 写的）—— 别手改。\n\
         //\n\
         // 每条实例一个「生成出来的类型」：它复用某个**声明**的接口，体来自 recipe，\n\
         // 身份（`source_hash`）= 实例 key。stage 2 用的类型就是这里这一份\n\
         // （`.agents/notes/art/21-codegen-types.md`）。\n\
         //\n\
         // ⚠ key 与源文件字节有关 ⇒ 它只能在**运行期**算；生成物里只有不随源码字节变的\n\
         // 那几样是 const（op id / 声明指纹 / 三个类型 / 根 / 源路径 / 体）。\n\
         //\n\
         // ⚠ 这里是 `//` 而不是 `//!`：这个文件是被并进 `px_graphs/src/insts.rs` 的一个**模块体**，\n\
         //   内层文档注释在那儿会报 `expected outer doc comment`（实测）。\n\
         \n",
    );
    for item in plan {
        let symbol = inst::symbol(&item.decl);
        let roots: String = item
            .roots
            .iter()
            .map(|root| format!("\"{root}\""))
            .collect::<Vec<String>>()
            .join(", ");
        out.push_str(&format!(
            "/// 实例 `{op_id}`（声明 `{decl}`）—— 体来自 `{recipe_file}`。\n\
             pub struct {type_name};\n\
             \n\
             impl {type_name} {{\n\
             \x20   pub const INST_ID: &'static str = \"{op_id}\";\n\
             \x20   pub const INST_DECL: &'static str = \"{decl}\";\n\
             \x20   pub const INST_SOURCE: &'static str = \"{source}\";\n\
             \x20   pub const INST_ROOTS: &'static [&'static str] = &[{roots}];\n\
             \x20   /// ⚠ `ARG` 已换成 `&{type_name}`（那才是能编的体）；key 那一轴用的是\n\
             \x20   ///   recipe 里那一份原文 —— 见 `INST_TEMPLATE`。\n\
             \x20   pub const INST_BODY: &'static str = \"{body}\";\n\
             \x20   /// recipe 里那一栏**原文**（`…, ARG`）：它是 **key 的一轴**（`InstInfo.template`）。\n\
             \x20   pub const INST_TEMPLATE: &'static str = \"{template}\";\n\
             }}\n\
             \n\
             impl ::px_cook::inst::InstNode for {type_name} {{\n\
             \x20   fn info() -> ::px_cook::inst::InstInfo {{\n\
             \x20       ::px_cook::inst::info_of_facts(\n\
             \x20           Self::INST_ID,\n\
             \x20           {interface}u64,\n\
             \x20           \"{decl_hash}\",\n\
             \x20           Self::INST_ROOTS,\n\
             \x20           Self::INST_SOURCE,\n\
             \x20           Self::INST_TEMPLATE,\n\
             \x20       )\n\
             \x20   }}\n\
             }}\n\
             \n\
             impl ::px_graph_schema::PxOp for {type_name} {{\n\
             \x20   const ID: &'static str = \"{op_id}\";\n\
             \x20   /// 空串：实现不在任何**预置**实现库里 —— 它是运行期按 key 装载的实例库。\n\
             \x20   const LIB: &'static str = \"\";\n\
             \x20   const SYMBOL: &'static str = \"{symbol}\";\n\
             \n\
             \x20   type Params = {params};\n\
             \x20   type Inputs = {inputs};\n\
             \x20   type Payload = {payload};\n\
             \n\
             \x20   fn new() -> Self {{\n\
             \x20       {type_name}\n\
             \x20   }}\n\
             \n\
             \x20   /// 接口形状从**复用的那三个关联类型**推，声明指纹取被复用声明那一份。\n\
             \x20   fn decl_hash() -> &'static str {{\n\
             \x20       \"{decl_hash}\"\n\
             \x20   }}\n\
             \n\
             \x20   /// 身份 = **实例 key**（图程序不重编也能看出泛型参数/alg/契约/工具链变了）。\n\
             \x20   /// ⚠ 算一次就记住（`OnceLock`）：key 要读盘数名册，别每次调都重算\n\
             \x20   ///   —— 这正是从前那份宏展开出来的形状。\n\
             \x20   fn source_hash() -> ::core::result::Result<&'static str, ::std::string::String> {{\n\
             \x20       static KEY: ::std::sync::OnceLock<\n\
             \x20           ::core::result::Result<::std::string::String, ::std::string::String>,\n\
             \x20       > = ::std::sync::OnceLock::new();\n\
             \x20       match KEY.get_or_init(|| {{\n\
             \x20           ::px_cook::inst::key_of_facts(\n\
             \x20               Self::INST_ID,\n\
             \x20               {interface}u64,\n\
             \x20               \"{decl_hash}\",\n\
             \x20               Self::INST_ROOTS,\n\
             \x20               Self::INST_SOURCE,\n\
             \x20               Self::INST_TEMPLATE,\n\
             \x20           )\n\
             \x20       }}) {{\n\
             \x20           ::core::result::Result::Ok(text) => ::core::result::Result::Ok(text.as_str()),\n\
             \x20           ::core::result::Result::Err(err) => ::core::result::Result::Err(err.clone()),\n\
             \x20       }}\n\
             \x20   }}\n\
             \n\
             \x20   fn render(\n\
             \x20       &self,\n\
             \x20       p: &Self::Params,\n\
             \x20       i: &Self::Inputs,\n\
             \x20   ) -> ::core::result::Result<Self::Payload, ::std::string::String> {{\n\
             \x20       let key = Self::source_hash()?;\n\
             \x20       let path = ::px_cook::inst::library_path(key);\n\
             \x20       if !path.is_file() {{\n\
             \x20           return ::core::result::Result::Err(format!(\n\
             \x20               \"{{}}{{}}\",\n\
             \x20               path.display(),\n\
             \x20               ::px_cook::inst::missing_hint(key)\n\
             \x20           ));\n\
             \x20       }}\n\
             \x20       let body = ::px_graph_schema::ops::load_at::<Self>(\n\
             \x20           path.to_string_lossy().as_ref(),\n\
             \x20           Self::SYMBOL,\n\
             \x20       )?;\n\
             \x20       body(p, i)\n\
             \x20   }}\n\
             }}\n\
             \n",
            op_id = item.op_id,
            decl = item.decl,
            recipe_file = RECIPE,
            type_name = item.type_name,
            source = item.source,
            body = item.body,
            template = item.template,
            interface = item.interface,
            decl_hash = item.decl_hash,
            symbol = symbol,
            params = item.params,
            inputs = item.inputs,
            payload = item.payload,
        ));
    }
    // 按**图侧类型名**取两个事实的小表：`insts::build()` 用它把 recipe 翻成图里的节点。
    // ⚠ 两个值都是上面那些 `const`（编译期就位）；`_` 那一臂指名道姓地说 recipe 写错了。
    out.push_str(
        "/// 图侧类型名 → `(interface, decl_hash)` —— recipe 与生成物之间那一条的落点。\n\
         ///\n\
         /// ⚠ 名字不认识就是 recipe 与生成物**不同步**（改了表没重新生成？）⇒ 当场说清楚。\n\
         pub fn facts_of(type_name: &str) -> (u64, &'static str) {\n\
         \x20   match type_name {\n",
    );
    for item in plan {
        out.push_str(&format!(
            "        \"{type_name}\" => ({interface}u64, \"{decl_hash}\"),\n",
            type_name = item.type_name,
            interface = item.interface,
            decl_hash = item.decl_hash,
        ));
    }
    out.push_str(
        "        other => panic!(\n\
         \x20           \"recipe 里那条实例的 `type_name` 是 `{other}`，而生成物里没有这个类型\\n  \
         ⇒ `inst_recipe.rs` 与 `insts_gen.rs` 不同步（跑 `cargo build` 重新生成？）\"\n\
         \x20       ),\n\
         \x20   }\n\
         }\n",
    );
    Ok(out)
}

/// **`OUT_DIR/insts_gen_catalogue.rs`**：给工具与门的旁挂件（`InstCatalogue`）。
///
/// ⚠ 它不进图程序（`insts.rs` 只并进 `insts_gen.rs`）；类型一律写**全路径**
///   ⇒ 图程序与 `tests/*.rs` 两边都编得过。
/// ⚠ 这里落的是**条目切片**（不是 `InstCatalogue` 本身）：element 那一档的条目**不住生成物里**
///   （它的规格住在 `px_elem` 里、内容键在运行期算）⇒ 由 `px_graphs::insts::codegen()` 把这一串
///   拷成一张可增长的表、再把 `ELEM_SPECS` 每条插进去。生成物只出"声明那一档"那几条。
fn catalogue_text(plan: &[Planned]) -> String {
    let mut out = String::from(
        "// **生成物的旁挂件**（`px_graphs/build.rs` 写的）：每条实例的 op id / key / 体住哪。\n\
         //\n\
         // 谁读它：`px build` / `px run --build`（**错误映射**：编不过时报`体来自 recipe 第几行`）\n\
         // 与端到端测试。⚠ 它**不参与任何 key**、也不进图程序。\n\
         //\n\
         // ⚠ 形状是**条目切片**：`px_graphs::insts::codegen()` 把这一串拷成可增长的表，\n\
         //   再把 `px_elem::ELEM_SPECS` 那几条（图侧没有生成物）插进去。\n\
         \n\
         pub static INST_CODEGEN: &[::px_cook::inst::InstCodegen] = &[\n",
    );
    for item in plan {
        out.push_str(&format!(
            "        ::px_cook::inst::InstCodegen {{\n\
             \x20           op_id: \"{op_id}\",\n\
             \x20           key: \"{key}\",\n\
             \x20           kind: ::px_cook::inst::InstKind::Decl {{\n\
             \x20               schema: \"{schema}\",\n\
             \x20               module: \"{module}\",\n\
             \x20               decl: \"{decl}\",\n\
             \x20           }},\n\
             \x20           source: \"{source}\",\n\
             \x20           body: \"{body}\",\n\
             \x20           recipe_line: {recipe_line},\n\
             \x20           recipe: \"{recipe}\",\n\
             \x20           generated: \"{generated}\",\n\
             \x20       }},\n",
            op_id = item.op_id,
            key = item.key,
            schema = item.schema,
            module = item.module,
            decl = item.decl,
            source = item.source,
            // ⚠ 旁挂件里记的是**能编的那一份**（`ARG` 已替换）：错误映射要能把
            //   `target/jit/<key>/src/lib.rs:<行>` 与 `art/inst/*.rs` 对上。
            body = item.body,
            recipe_line = item.recipe_line,
            recipe = RECIPE,
            generated = item.generated,
        ));
    }
    out.push_str("];\n");
    out
}
