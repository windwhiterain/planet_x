//! **泛型实例**：内容寻址的**代码**缓存 + 运行期装载（设计见 `.agents/notes/art/19-generic-inst.md`）。
//!
//! 一条实例 = 「复用一个算子的**声明** + 一段 agent 现写的**泛型参数**」编译出来的实现库。
//! 它的**文件名就是它的构建指纹** ⇒ 编译期写不进 `const LIB`，装载器必须运行期拿到它
//! （这正是契约层要有 `ops::load_at` 的原因）。
//!
//! ⚠ **key 由这一层算**，图程序（`px_graphs/build.rs` 生成的 `InstNode::info()` /
//!   `source_hash()`）与 `px` 工具调**同一个**函数 ⇒ 不可能对不上。算 key 的算法一变，
//!   全部 key 跟着变，旧实例库只是**孤儿**（不是"命中错的"）
//!   —— 这就是 §176 第 1 条：**没有清单，所以没有会烂的清单**。

use std::path::{Path, PathBuf};
use std::process::Command;

use px_graph_schema::PxOp;
use px_graph_schema::blake3;

/// 实例库的包名：生成的 crate 固定用它 ⇒ 符号名是**编译期字面量**（两边都写得出）。
pub const PACKAGE: &str = "px_inst";

/// **图的计划**要住哪：`px_graphs/src/inst_recipe.rs`（相对 workspace 根）。
///
/// ⚠ 它是**数据表**（`20-build-graph.md` 那之后这一轮：`21-codegen-types.md`）：图侧一个字面量都不写，
///   事实（op id / 声明名 / 根 / 源文件 / 体）全在这里。
pub const RECIPE: &str = "px_graphs/src/inst_recipe.rs";

/// key 的域分隔符。⚠ 与键的 `px_cook/v1` 同理：**不许顺手改**（改 = 全部实例失效）。
const VERSION: &[u8] = b"px_inst/v1";

/// 一个实例的构建指纹的全部输入（少一样就是"陈旧复用"，§177）。
pub struct Inst<'a> {
    /// 这个实例的 op id（进产物键、进读数）。
    pub op_id: &'a str,
    /// `<声明 as PxOp>::interface()` —— 三个类型名。
    pub interface: u64,
    /// `<声明 as PxOp>::decl_hash()` —— **声明所在 crate** 的源码指纹。
    /// ⚠ 接口哈希只有类型名、不含字段布局；少了这一样，给 `Inputs` 加一个字段就会复用
    ///   按旧布局编出来的 DLL。
    pub decl_hash: &'a str,
    /// 泛型体住的 crate（**目录名**，相对 workspace 根）：它的源码名册要进 key。
    pub alg_roots: &'a [&'a str],
    /// agent 写的泛型参数源文件（相对 workspace 根）。**内容**进 key，路径不进。
    pub source: &'a str,
    /// 体模板（recipe 里那段"泛型参数怎么装进声明"的表达式；泛型参数那一位写占位符 `ARG`）。
    ///
    /// ⚠ 它必须进 key：两处声明可能 `op_id` / 声明 / alg / 源文件**都一样、只有模板不同** ——
    ///   那编出来的是**不同的体**，共用一个 key 就是静默复用错的构件。
    pub template: &'a str,
}

/// 模板的**归一形式**：去掉所有空白。
///
/// ⚠ 两边必须同一个口径：图侧手里是 `stringify!` 的 token 串（`a :: b` 带空格），
///   工具手里是从源码里抠出来的原文 ⇒ 都去空白才对得上。
pub fn normalize_template(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 一条实例的全部信息 —— `px` 工具用它生成、`px_graphs/src/insts.rs` 用它登记。
#[derive(Debug, Clone)]
pub struct InstInfo {
    pub op_id: String,
    pub interface: u64,
    pub decl_hash: String,
    pub alg_roots: Vec<String>,
    pub source: String,
    /// 体模板（归一前的原文；key 里用的是去空白那一份）。
    pub template: String,
    /// 编译期算不出来的那部分（key）——`info` 只描述"要什么"。
    pub library: String,
}

/// 从**类型本身**取接口哈希与声明指纹（图侧唯一能算这两样的地方）。
///
/// ⚠ `template` 用 `<T as …>::INST_TEMPLATE`（宏发出来的那份），别手抄。
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

/// 图程序那一侧：从算子类型取两个指纹再算 key（生成物的 `source_hash()` 走这一条）。
///
/// ⚠ 今天是**生成物**调它（`px_graphs/build.rs` 把 `<T as PxOp>::interface()` /
///   `decl_hash()` 算成字面量写进 `insts_gen.rs`，`InstNode::info()` 里那几个常量就是它们）。
///   ⇒ 它仍然只走 [`key_of_facts`]（一份算法，见那一份的文档）。
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

/// **按显式事实算 key** —— 生成器（`px_graphs/build.rs`）那一侧的唯一入口。
///
/// ⚠ 它存在的理由：`px_graphs/build.rs` **编译不了**算子类型（它只被 `[build-dependencies]`
///   看见），而 key 的输入里有两样只有编译过类型的那一侧算得出（`interface()` / `decl_hash()`）
///   ⇒ 由 `px_decls`（一个小 rlib，**不进任何实现库的名册**）把这两样交出来，
///   这一层只负责"照同一套输入算同一个 key"。见 `.agents/notes/art/21-codegen-types.md`。
///
/// ⚠ **算法一个字都不在这儿**：它转手就构造 [`Inst`] 走 [`key`] —— 生成器与图程序
///   （生成的 `InstNode::info()`）拿到的是**同一个函数**算出来的 key，不可能对不上。
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

/// 生成物那一侧：同样按**显式事实**交回一份 [`InstInfo`]（`InstNode::info()` 就是一层转发）。
///
/// ⚠ `library` 这一栏**每次都要重算**（它 = `target/pcg/inst/<key>.dll` 而 key 要读源文件字节）：
///   写死进生成物就不可能了 —— 而"写死"正是 R1 会破的那条路（改 `art/inst/*.rs` 会让
///   生成物变 ⇒ 图程序重编）。生成物里只有**不随源码字节变**的那几样是 const。
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

/// 算一个实例的 key。**工具与图程序必须调这一个**。
pub fn key(inst: &Inst<'_>) -> Result<String, String> {
    let root = crate::workspace_root();
    let mut roster = px_fingerprint::Roster::new();
    for alg in inst.alg_roots {
        let crate_dir = root.join(alg);
        let files = px_fingerprint::roster(&crate_dir, &[]);
        if files.is_empty() {
            return Err(format!(
                "实例 key：{} 里没有可数的源码（recipe 的 `roots` 那一栏把 crate 名字写错了？）",
                crate_dir.display()
            ));
        }
        for (label, path) in files {
            // ⚠ 前缀带上 alg 目录名：两个 alg crate 里同名文件不该撞。
            roster.insert(format!("{alg}::{label}"), path);
        }
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(VERSION);
    field(&mut hasher, px_graph_schema::TOOLCHAIN_HASH);
    field(&mut hasher, px_graph_schema::SOURCE_HASH);
    field(&mut hasher, inst.decl_hash);
    field(&mut hasher, &px_fingerprint::hash(&roster));
    hasher.update(&inst.interface.to_le_bytes());
    field(&mut hasher, inst.op_id);
    // ⚠ 模板（去空白后）也是身份的一部分：它决定"泛型参数怎么装进声明"。
    field(&mut hasher, &normalize_template(inst.template));

    // ⚠ 泛型参数：读**内容**（不是路径）—— 改它必换 key，把它挪个位置不换。
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

/// 实例库收在哪：`target/pcg/inst/<key>.dll`（机器本地、可删、不进 git —— 与数据 CAS 同性质）。
pub fn library_path(key: &str) -> PathBuf {
    crate::workspace_root()
        .join("target/pcg/inst")
        .join(format!("{key}{}", std::env::consts::DLL_SUFFIX))
}

/// 生成的 crate 里 `px_body!` 拼出来的符号名（`concat!(包名, "__", 声明类型名)`）。
pub fn symbol(decl: &str) -> String {
    format!("{PACKAGE}__{decl}")
}

/// **一个实例的类型化把手**：生成物（`OUT_DIR/insts_gen.rs`）给每个实例实现它。
///
/// ⚠ 名字叫 `InstNode` 而不是 `Inst`：`Inst` 已经是 key 的输入结构（上面那个），
///   而这一位是"build graph 里的一个**节点**"（`20-build-graph.md` §183）。
///
/// ⚠ 存在的理由是"**描述只在一处**"（`20` §184）：op id / 根 / 源文件 / 体
///   都只写在 `px_graphs/src/inst_recipe.rs` 那一行里，任何需要这些事实的地方
///   （build graph、工具、门）都**通过生成物**取，而不是再抄一遍。
pub trait InstNode {
    fn info() -> InstInfo;
}

/// **stage 1（build graph）的图**：节点 = 代码单元，边 = "它编译时链了谁"。
///
/// ⚠ 它是 `20-build-graph.md` §187 的 B2 后半那一层：`missing()` 说"缺哪些"、
///   `compile_missing()` **真的编**（生成 → cargo → 收库 + sidecar）。工具（`px`）
///   与两阶段入口（`px run`）都只调这两件事，不再各写一遍。
///
/// ⚠ **它不认识任何具体图程序**：要被扫的源码目录由调用方给（`compile_missing(dir, …)`）——
///   于是这一层不依赖 `px_graphs`（依赖方向仍是 `px_graphs → px_cook`）。
#[derive(Default)]
pub struct BuildGraph {
    nodes: Vec<InstInfo>,
}

impl BuildGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// 声明一条实例节点（边 = `T::info().alg_roots`，那是"编译时链的 crate"）。
    ///
    /// ⚠ 同一 op id 声明两次 = 图里有两条同 id 的节点 ⇒ **当场拒**（id 必须唯一：
    ///   工具按 id 认领源码里那一处调用）。
    pub fn inst<T: InstNode>(&mut self) -> &mut Self {
        let info = T::info();
        self.push(info)
    }

    /// 按**显式事实**声明一条实例节点（`px_graphs/src/insts.rs` 的 `build()` 走这一条）。
    ///
    /// ⚠ 为什么不是 `inst::<T>()`：今天图侧那些类型是**生成物**（`OUT_DIR/insts_gen.rs`），
    ///   而"要注册哪些"是 `inst_recipe.rs` 那张**数据表** —— 表里写不出泛型调用
    ///   （它只许有类型定义与字面量）。⇒ 由表交出事实、这里把它们登记成节点，
    ///   事实仍只有一个来源（表 + `px_decls`）。
    /// ⚠ `name` 只给**报错**用（说得出是哪一条 recipe 不合规），不进任何 key。
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

    /// ⚠ 与 [`Self::nodes`] 同一个东西，换个说得更准的名字：图里是**节点**，
    ///   而"节点"在 `20` §183 里是**代码单元**那一档（这里今天只有 `Inst` 一种）。
    pub fn instances(&self) -> &[InstInfo] {
        &self.nodes
    }

    /// **stage 1 的计划**：图上每条节点在盘上有没有库。
    ///
    /// ⚠ 这一步**不编译、不写盘**（`20` §186：stage 1 的只读部分）；它只 stat 那些路径。
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

    /// **执行 stage 1**：对每条缺库的节点，生成 → 编译 → 收库（+ sidecar）。
    ///
    /// ⚠ **只有这一件事会写盘**（`20` §186）：`Crate` / `Toolchain` / `Contract` 是只读指纹
    ///   节点，它们在 key 里而不在盘上。
    /// ⚠ `catalogue` 是**先生成好的那份生成物事实**（模板原文与它住哪）—— 从
    ///   `px_graphs/src/inst_recipe.rs` 来（`21-codegen-types.md`）：它不认识任何具体图程序，
    ///   这一层不依赖 `px_graphs`。
    /// ⚠ cargo 的 stdout/stderr **原样透出**，失败**不写库**、不静默（`19` §176 第 3 条）。
    ///
    /// ⚠ 它是"第一条失败就回"的那一档；`px build` 要的是"一条一条编、剩下的也编完"，
    ///   所以那边自己循环 `compile_one`（**同一个** `compile_one`）。
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

/// **stage 1 的计划**（`BuildGraph::missing`）：谁在盘上、谁不在。⚠ 不含任何编译。
#[derive(Debug, Clone)]
pub struct Plan {
    pub total: usize,
    /// 已经在盘上的条数。
    pub present: u32,
    /// 缺的那些（按图里的顺序）。
    pub missing: Vec<Missing>,
}

impl Plan {
    /// 全都在盘上 ⇒ stage 2 可以直接跑（这是"运行只读"那条默认值的判据）。
    pub fn complete(&self) -> bool {
        self.missing.is_empty()
    }

    /// 缺实例时那句"该跑什么命令"（`19` §176 第 3 条：报错必须带命令）。
    ///
    /// ⚠ 命令里的**图名由调用方给**：`px run <图> …` 的第一个参数就是图名 —— 两种都写出来，
    ///   抄一条就能跑。⚠ 这两个词就是 bin `px` 的子命令（`20-build-graph.md` §182）。
    /// ⚠ 命令前缀走 [`driver_command()`]：**直接跑 driver exe**，不绕 `cargo run`
    ///   （理由在那一条的文档里）。
    pub fn hint(&self, graph: &str) -> String {
        let keys: Vec<&str> = self
            .missing
            .iter()
            .map(|absent| absent.key.as_str())
            .collect();
        let driver = driver_command();
        format!(
            "缺 {} 条实例库（共 {} 条）：\n  {}\n  \
             ⇒ 编它们（只跑 stage 1）：{driver} build\
             \n     或者两个 stage 连着跑：{driver} run {graph} --build",
            self.missing.len(),
            self.total,
            keys.join("\n  "),
        )
    }
}

/// 一条缺的实例。
#[derive(Debug, Clone)]
pub struct Missing {
    pub op_id: String,
    pub key: String,
    pub library: String,
}

/// **stage 1 的执行读数**（口径与 `px build` 那行汇总一致：`共 N 条：已编 a、已有 b、失败 c`）。
#[derive(Debug, Clone)]
pub struct Stage1 {
    pub total: usize,
    /// 这一趟**真的编出来**的那些。
    pub built: Vec<Missing>,
    /// 本来就在盘上的条数。
    pub already: u32,
}

impl Stage1 {
    /// 摘要那一行（`共 N 条：已编 a、已有 b、失败 c`）。
    ///
    /// ⚠ `failed` 由调用方给：本层遇到失败是**立刻回 `Err`**（不继续编，也不写库），
    ///   所以"编到一半失败"的读数在 driver 那一侧拼 —— `px` 一条一条编，好把剩下的也编完。
    pub fn summary(&self, failed: u32) -> String {
        format!(
            "共 {} 条：已编 {}、已有 {}、失败 {failed}",
            self.total,
            self.built.len(),
            self.already,
        )
    }
}

/// 活键集（`gc` 的判据）：本图的每条节点在盘上的**文件名**（= key）。
pub fn live_keys(graph: &BuildGraph) -> Vec<String> {
    graph
        .instances()
        .iter()
        .map(|info| key_of_info(info))
        .collect()
}

/// 一条节点的 key（`info.library` 的文件名 —— 那就是装载器会去找的那一个）。
pub fn key_of_info(info: &InstInfo) -> String {
    Path::new(&info.library)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string()
}

/// 一条实例的**生成物事实**：生成器（`px_graphs/build.rs`）算好后写进
/// `OUT_DIR/insts_gen_catalogue.rs`，`px build` / `px run --build` / 端到端测试都读它。
///
/// ⚠ 为什么要有这一份：`compile_one` 需要「体表达式原文 + 它住哪」，而今天体**不在**图侧源码里
///   （它是 recipe 里的一段字面量）⇒ 谁也扫不出来。生成器知道它（它就是生成器读的那张表）⇒
///   由生成器落一份下来。**只读**，不参与任何 key。
#[derive(Debug, Clone)]
pub struct InstCodegen {
    /// 这条实例的 op id（认领源码里的记录、报错时点名）。
    pub op_id: &'static str,
    pub key: &'static str,
    /// 声明住的 schema crate（`px_volume_schema`）—— 生成物的 `[dependencies]` 与 `use` 用它。
    pub schema: &'static str,
    /// 声明在它 crate 里的**模块路径**（`ops`）—— 生成物那句 `pub use <schema>::<module>::<decl>;`。
    pub module: &'static str,
    /// 声明名（`CloudCoarse`）—— 符号名 `px_inst__<它>`。
    pub decl: &'static str,
    /// 泛型参数源文件（相对 workspace 根）。
    pub source: &'static str,
    /// 体表达式**原文**（recipe 里那一栏）—— 逐字抄进生成物。
    pub body: &'static str,
    /// `body` 在 recipe 文件里的行号（**错误映射**用：编不过时报它）。
    pub recipe_line: u32,
    /// recipe 文件（相对 workspace 根）。
    pub recipe: &'static str,
    /// 生成物 `target/jit/<key>/`（**留着不删** —— 报错带它）。
    pub generated: &'static str,
}

/// 全部实例的生成物事实（`for_op` 按 op id 取一条）。
pub struct InstCatalogue {
    pub entries: &'static [InstCodegen],
}

impl InstCatalogue {
    pub fn for_op(&self, op_id: &str) -> Result<&'static InstCodegen, String> {
        self.entries
            .iter()
            .find(|entry| entry.op_id == op_id)
            .ok_or_else(|| {
                format!(
                    "生成物里没有 op id 为 {op_id} 的记录\
                     \n  ⇒ `{}` 与 `px_graphs/src/insts.rs` 里那张图不同步？跑 `cargo build` 重新生成",
                    RECIPE
                )
            })
    }
}

/// **编一条**：生成 → cargo → 收库 + sidecar（`19` §179.4 的第 1–5 步）。
///
/// ⚠ 失败时把**这条编译错是哪一段代码的**说清楚（`21-codegen-types.md` 的"错误映射"）：
///   体来自 recipe 第几行、参数文件是谁、生成物在哪。生成物**留着不删**。
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
            codegen.op_id, codegen.recipe, codegen.recipe_line, codegen.source, codegen.generated,
        )),
    }
}

/// [`compile_one`] 的正文（错误映射在外面套一层）。
fn compile_generated(info: &InstInfo, key: &str, codegen: &InstCodegen) -> Result<(), String> {
    let root = crate::workspace_root();
    // ⚠ 生成的 crate 一律叫 `px_inst` ⇒ 符号名是编译期字面量（`PACKAGE` 那一份）。
    let symbol = symbol(codegen.decl);
    // 生成的 crate 里怎么把那个声明引进来：声明住的 crate 与模块**由声明表给**
    // （`px_decls::decl` 是编译过类型的那一侧，路径不会抄错）。
    let decl_path = format!("{}::{}", codegen.schema, codegen.module);
    let dir = root.join("target/jit").join(key);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).map_err(|err| format!("建不了 {}：{err}", src.display()))?;

    let manifest = manifest_text(&root, &info.alg_roots, codegen.schema)?;
    write(&dir.join("Cargo.toml"), &manifest)?;
    write(&src.join("lib.rs"), &lib_text(&root, codegen, &decl_path)?)?;
    write(&dir.join("build.rs"), BUILD_RS)?;

    println!(
        "生成 {}（体来自 {}:{}）",
        dir.display(),
        codegen.recipe,
        codegen.recipe_line,
    );
    cargo_build(&dir)?;

    // ⚠ 产物在哪：cargo 的 profile 目录叫 `debug`/`release`（自定义 profile 就用它的名字），
    //   而**传了 `--target`** 时会多一层 `<triple>/` ⇒ 两处都找，别写死一条。
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

    // sidecar 与库**并排**（`target/pcg/inst/<key>.json`）：库删了它也就没有意义。
    let sidecar = library.with_extension("json");
    write(&sidecar, &sidecar_text(info, key, &symbol, codegen))?;
    Ok(())
}

/// 生成的 crate 的 `Cargo.toml`。
///
/// ⚠ 路径一律**绝对**：生成物住在 `target/` 下，相对路径会飘（`19` §179.4）。
/// ⚠ `[workspace]` 是**必须的**：它是一个自成的 workspace 根，否则 cargo 会往上找到主
///   workspace 的 `Cargo.toml`、把它当成员、还会共用主 target 目录。
pub fn manifest_text(root: &Path, roots: &[String], schema: &str) -> Result<String, String> {
    if roots.is_empty() {
        return Err(
            "这条实例没登记任何根（`inst::InstInfo::alg_roots` 是空的）—— 泛型体住哪个 crate？\
             （那就是 `20-build-graph.md` §183 里的**边**）"
                .to_string(),
        );
    }
    let graph_schema = slash(&crate_path(root, "px_graph_schema")?);
    let schema_path = slash(&crate_path(root, schema)?);
    let fingerprint = slash(&crate_path(root, "px_fingerprint")?);
    // ⚠ **每一个根都要写进去**，不只第一个：这一栏 = "这个实例编译时链了谁"（`20` §183 的边）。
    //   只写第一个 ⇒ 泛型参数 `use` 不到第二个 crate（而 key 已经覆盖了它 —— 身份对、manifest 没给）。
    let mut deps = String::new();
    for name in roots {
        if name != "px_graph_schema" && name != schema {
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
         {schema} = {{ path = \"{schema_path}\" }}\n\
         {deps}\
         \n\
         [build-dependencies]\n\
         px_fingerprint = {{ path = \"{fingerprint}\" }}\n\
         \n\
         # 自成 workspace 根：别挂进主 workspace、也别共用它的 target 目录。\n\
         [workspace]\n",
    ))
}

/// 生成的 `src/lib.rs`（形状见 `19` §179.1）。
///
/// ⚠ `include!` 用**绝对路径**：生成物在 `target/` 下，而泛型参数的源在 `art/inst/` 里。
/// ⚠ `px_body!` 的第一段是 **`$name:ident`**（不是 `$decl:ty`），所以 `19` 里写的
///   `px_body! { volume::CloudCoarse, … }` 宏匹配不过（实测 `error: no rules expected ::`）。
///   生成物因此先把声明**按原名引进作用域**（`pub use <声明的真路径>;`），再写裸名。
/// ⚠ 体（`|p, i, g| …` 那一段）**逐字**取自 recipe 那一栏 —— 里面写的是**真类型名**
///   （`&Waves`），因为生成物 `include!` 了 `source` ⇒ `Waves` 就在作用域里
///   （`21-codegen-types.md`：占位符与文本替换都没了）。
pub fn lib_text(
    root: &Path,
    codegen: &InstCodegen,
    declaration_path: &str,
) -> Result<String, String> {
    let source = root.join(codegen.source);
    if !source.is_file() {
        return Err(format!("泛型参数源不在盘上：{}", source.display()));
    }
    Ok(format!(
        "//! 生成物（`px build` 写的）。别手改 —— 改 {} 里的源文件，然后重跑。\n\
         \n\
         // 复用的那个声明（`{}`）——引进来当 `{}`：\n\
         #[allow(unused_imports)]\n\
         pub use {declaration_path}::{};\n\
         \n\
         include!(\"{}\");\n\
         \n\
         px_graph_schema::px_body! {{\n\
         \x20   {},\n\
         \x20   |p, i, g| {}\n\
         }}\n\
         \n\
         px_graph_schema::px_impl_lib!();\n",
        codegen.source,
        codegen.decl,
        codegen.decl,
        codegen.decl,
        slash(&source),
        codegen.decl,
        codegen.body,
    ))
}

const BUILD_RS: &str = "//! 实例库自己的身份（源码指纹 + 工具链指纹）—— 与各实现库同一份算法。\n\
                        fn main() {\n\
                        \x20   px_fingerprint::cargo_fingerprint_for_crate(&[]);\n\
                        }\n";

/// sidecar：`{key, op_id, decl, decl_crate, alg, source, template, toolchain, symbols}`。
///
/// ⚠ `template` 记的是**体表达式原文**（recipe 里那一份）：归一那一步只是 key 的口径，
///   给人看的时候要的是它能读。
pub fn sidecar_text(info: &InstInfo, key: &str, symbol: &str, codegen: &InstCodegen) -> String {
    let fields = [
        ("key", key.to_string()),
        ("op_id", info.op_id.clone()),
        ("decl", codegen.decl.to_string()),
        ("decl_crate", codegen.schema.to_string()),
        ("alg", info.alg_roots.join(",")),
        ("source", info.source.clone()),
        ("template", codegen.body.to_string()),
        ("toolchain", px_graph_schema::TOOLCHAIN_HASH.to_string()),
        ("symbols", symbol.to_string()),
    ];
    let mut out = String::from("{\n");
    for (name, value) in &fields {
        out.push_str(&format!("  \"{name}\": {},\n", json_string(value)));
    }
    out.push_str("}\n");
    out
}

/// cargo 的产物目录名：build script 看到的 `PROFILE` 是 `debug`/`release`，
/// 而 `--profile dev` 的目录叫 `debug`。
pub fn profile_dir() -> &'static str {
    match inst_env("PX_PROFILE") {
        "debug" => "debug",
        other => other,
    }
}

/// **这一份构建**的工具链四原料之一（`px_cook/build.rs` 发的）。
///
/// ⚠ 为什么不读 `std::env::var`：那四样要的是**编译这一份 `px_cook` 时**的值，而不是
///   运行时的环境（`cargo:rustc-env` 不跨 crate 传播，所以只能自己发一份；见 `px_cook/build.rs`）。
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

/// 跑 cargo，**原样**把它的 stdout/stderr 透出去。
///
/// ⚠ **必须把图程序这一份构建的四个原料原样转给它**：实例库要导出 `__toolchain_hash`，
///   而那个哈希正是这四个原料算出来的（`px_fingerprint::toolchain_hash`）。不转的话，
///   主构建一旦带 `RUSTFLAGS`，实例库与图程序就**必然对不上**，装载层会（正确地）把每条实例
///   都拒掉 —— 拒绝没错，但得让人有办法改对，所以这里把 profile / target / RUSTFLAGS 一起传下去。
///
/// ⚠ **它只在 stage 1 被调**（`20-build-graph.md` §182）：stage 2（`cached` 那条路）**只读装载**，
///   永远走不到这里 ⇒ 图程序里这一段是**死代码**（它随 `px_cook` 链进图 exe，但一个字节都不会跑）。
///   把"会跑 rustc"这件事放进一个 stage 2 也链着的 crate，代价就是这一份死代码 —— 换来的是
///   "执行住在 `BuildGraph` 上"（B2 后半那一条），而 R1（改 art/inst ⇒ 七个 exe 一位不动）
///   与"运行只读"都靠"stage 2 不调它"守着，不靠它不在场。
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
        // profile：`dev` 这个名字在 CLI 上对应 build script 眼里的 `debug`。
        .arg("--profile")
        .arg(match inst_env("PX_PROFILE") {
            "debug" => "dev",
            other => other,
        });
    // target：只在**与 host 不同**时才传（传了 cargo 会多一层 `<triple>/` ⇒ 上面两处都找了）。
    let triple = inst_env("PX_TARGET");
    let host = env!("PX_RUSTC_VERSION")
        .lines()
        .find_map(|line| line.trim().strip_prefix("host: "))
        .unwrap_or_default()
        .to_string();
    if !triple.is_empty() && triple != host {
        command.arg("--target").arg(triple);
    }
    // RUSTFLAGS：原样给它（空就不设，免得把环境里别的值盖掉）。
    let rustflags = inst_env("PX_RUSTFLAGS");
    if !rustflags.is_empty() {
        command.env("RUSTFLAGS", rustflags);
    }
    let output = command
        .output()
        .map_err(|err| format!("跑不了 cargo：{err}"))?;
    // ⚠ agent 唯一有用的东西就是这两股原文 ⇒ 不做任何加工。
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

/// 缺实例时那句"该跑什么命令"。
pub fn missing_hint(key: &str) -> String {
    format!(
        "\n  ⚠ 这条实例还没编：`{driver} build`（只编缺的那几条）\
         \n     实例 key：{key}\n     库：{}",
        library_path(key).display(),
        driver = driver_command(),
    )
}

/// **该跑哪条命令**：优先给出**直接跑 driver exe** 的那一条（`<exe 同目录>/px[.exe]`）。
///
/// ⚠ 不推荐 `cargo run -p px_graphs --bin px -- …` 的原因不是"更短"，而是**它会改变量到的东西**：
///   `cargo run` 会把 `px_graphs` 按另一套特性合并重链一遍（`19` §180 的量法坑），
///   而本仓的口径一直是"**读 key 直接跑 `target/debug/px.exe`**"。
/// ⚠ 回退到 `cargo run` 只在"exe 不在图 exe 旁边"时发生（比如从别处起的进程）——
///   宁可给一条**能跑**的慢命令，也不要给一条指不到东西的快命令。
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

/// **错误映射**用：recipe 文件里第几行是这条体（1 开始；找不到就回 0）。
///
/// ⚠ recipe 是**数据表**（不是宏调用），所以"哪一行"只能按那一栏的原文找 —— 而它逐字
///   抄进了生成物（[`lib_text`]），两边本来就该是同一个字符串。
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

/// 生成物里写的路径：**正斜杠**。
///
/// ⚠ Windows 的反斜杠在两种地方都是**转义**：TOML 的字符串（`"C:\resource\…"` ⇒
///   `missing escaped value`）与 Rust 的字符串字面量（`include!("C:\…")`）。cargo 与 rustc
///   都吃正斜杠，所以生成物一律转一遍；盘上的路径仍用 `PathBuf`（Windows API 也吃正斜杠）。
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
