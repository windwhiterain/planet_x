// **图侧的实例 recipe**：一张**数据表**（`21-codegen-types.md` 的"四件新东西 ①"）。
//
// 图侧今天**一个字面量宏调用都没有**（"图侧零宏"）：要编哪些代码单元、它们复用哪个声明、
// 泛型参数住哪、体是什么 —— 全在这张表里。`src/insts.rs` 把 build script 按它生成出来的
// **类型**（`insts_gen.rs`）直接 `include!` 进来。
//
// ⚠ **这个文件只许有类型定义 + 字面量**：`px_graphs/build.rs` 会把它 `include!` 进一个模块里、
//   在 build script 里**执行**（它要读这张表）。任何逻辑都会在那一侧跑一遍。
//   ⚠ 这一段的注释必须是 `//` 而不是 `//!`：同一个文件在 build script 那一侧被 `include!`
//   进模块体（内层文档注释在那儿报 `expected outer doc comment`，实测），在那一边它排在
//   `use` 之后 ⇒ 只能是**普通注释**。
// ⚠ `body` 那一栏是**体表达式原文**，用**前一份实现**（图侧那个宏）当年的口径：泛型参数
//   那一位写**占位符 `ARG`**（今天仍是同一个词 —— 它是 **key 的一轴**，写别的就换 key，见
//   `19-generic-inst.md` §179.1）。
//   生成物 `include!` 了 `source`，所以把 `ARG` 换成**真类型名**（`&Waves`）之后那个名字就在
//   作用域里 ⇒ **不需要跨 crate 的宏管线**，只需要一次**整词替换**（[`InstRecipe::generated_body`]）。
//   为什么不让 recipe 直接写 `&Waves`：那样 key 就变了（实测 `96d4feb75ec4` → `8fcd505c8000`）
//   —— key 的算法一个字都不许改（`21-codegen-types.md` 的铁律 3）。
// ⚠ `decl` 是**声明名**（不是路径）：由 `px_decls`（声明表）解析成类型级事实
//   （三个类型路径 / `interface()` / `decl_hash()`）。
// ⚠ 改 `source` 指向的文件 ⇒ 那一条实例的 key 换、只有它要重编，**这张表与生成物一个字节
//   不变** ⇒ 七个图 exe 不重编（R1，见 `20-build-graph.md` §186 的读数）。

/// 一条实例：**op id + 声明 + 图侧类型名 + 根 + 源文件 + 体**。
pub struct InstRecipe {
    /// 这一条实例的 op id（进 key、进读数）。
    pub op_id: &'static str,
    /// 复用哪个**声明**（`px_decls` 表里的名字，如 `"FieldRemap"`）。
    pub decl: &'static str,
    /// 图侧**生成出来的类型**叫什么（`px_graphs::insts::<它>`）—— 生成器照它写
    /// `pub struct <它>;`。⚠ 它必须与 `source` 里那个类型**同名**：生成物两处引用它
    /// （`PxOp` impl 与体里的 `&<它>`），而 `include!` 进来的那个类型是唯一的来源。
    pub type_name: &'static str,
    /// 这条实例**编译时链的 crate**（目录名，相对 workspace 根）—— build graph 里的**边**
    /// （`20-build-graph.md` §183）：每一个都会进生成物的 `[dependencies]`，且它的源码名册进 key。
    pub roots: &'static [&'static str],
    /// 泛型参数住哪个源文件（相对 workspace 根）。**内容**进 key，路径不进。
    pub source: &'static str,
    /// 体表达式**原文**（用前一份实现当年那个口径：泛型参数那一位写占位符 `ARG`）。
    ///
    /// ⚠ 它是 **key 的一轴**（原样、归一化之后进 key）⇒ **不许改这个口径**。
    ///   生成物里要的是把 `ARG` 换成真类型名的那一份 —— 走
    ///   [`InstRecipe::generated_body`]（整词替换，与当年 `px_cook` 那一份逐字相同）。
    pub body: &'static str,
}

impl InstRecipe {
    /// 抄进生成物的体：`ARG` 整词换成 `&<图侧类型名>`。
    ///
    /// ⚠ 替换是**整词**的（与 `px_cook::inst_scan` 当年那一份逐字相同）：`ARG` 换掉，
    ///   而 `ARGS` 这类词不动。
    pub fn generated_body(&self) -> String {
        let mut out = String::with_capacity(self.body.len());
        let bytes = self.body.as_bytes();
        let needle = b"ARG";
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at..].starts_with(needle) {
                let before = at.checked_sub(1).map(|index| bytes[index]);
                let after = bytes.get(at + needle.len()).copied();
                let left = before.is_none_or(|ch| !is_word_byte(ch));
                let right = after.is_none_or(|ch| !is_word_byte(ch));
                if left && right {
                    out.push('&');
                    out.push_str(self.type_name);
                    at += needle.len();
                    continue;
                }
            }
            let ch = self.body[at..].chars().next().unwrap_or(' ');
            out.push(ch);
            at += ch.len_utf8();
        }
        out
    }
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// 今天的两条现役实例（`cloud.coarse/band` 与 `field.remap/waves`）。
///
/// ⚠ key 由 `px_graphs/build.rs` 按这张表的几栏 + `px_decls` 的事实算出来；
///   它与 `20-build-graph.md` §193 登记的那两个值**必须逐位相同**
///   （`96d4feb75ec4` / `caa8318cda1b`）。
pub const INSTANCES: &[InstRecipe] = &[
    InstRecipe {
        op_id: "cloud.coarse/band",
        decl: "CloudCoarse",
        type_name: "Band",
        roots: &["px_volume_alg"],
        source: "art/inst/band.rs",
        body: "px_volume_alg::coarse_with(p, i.coverage.value(), ARG)",
    },
    InstRecipe {
        op_id: "field.remap/waves",
        decl: "FieldRemap",
        type_name: "Waves",
        roots: &["px_field_alg"],
        source: "art/inst/waves.rs",
        body: "px_field_alg::remap_with(&px_field_alg::identity(), p, i.input.value(), g, ARG)",
    },
];
