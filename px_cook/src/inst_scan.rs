//! **按文本读源码的工具**（扫宏调用 / 收 `.rs` / 读 workspace 成员）——**一份词法口径**。
//!
//! ⚠ **它在图侧不再被任何执行路径用了**（`docs/system/codegen-types.md`）：体从图侧源码搬进了
//!   `px_graphs/src/inst_recipe.rs` 那张数据表，`px build` 读生成器落下的
//!   `OUT_DIR/insts_gen_catalogue.rs`，两个扫描点（`claim` / `check_template`）都删掉了。
//!   今天真正在用它的只有两处：
//!
//!   * **声明表那道门**（`px_decls/tests/inst_gate.rs`）：`count_named(…, "px_op!")`
//!     == `px_decls::decl` 表里的条数；
//!   * **生成器**（`px_graphs/build.rs`）：`collect_rs`（给要盯的 `.rs` 发 `rerun-if-changed`）
//!     与 `workspace_members`。
//!
//! ⚠ 历史上那条"找 `px_inst` 调用"的路（`scan` / `find_calls` / `count`）**今天零调用点**：
//!   图侧不再有 `px_inst!` 宏（定义也删了）。留着它是因为它本身就是**这份口径的正文**、
//!   且 `find_named` 是共用的那一个（`px_op!` 走它）。
//!
//! ⚠ **一份口径，不许两份**：从前 `px_jit`（那个 bin 今天叫 `px`）与 `inst_gate` 各写了一份扫描（一份跳注释、
//!   一份 `match_indices`），于是同一份源码在两个解析器眼里"调用"的数目不一样（实测 21 vs 1）
//!   —— 那道门就是这么失效的。所有地方从此都走这里。
//!
//! ⚠ **注释与字符串里的那个 token 不算调用**（这一层会跳过 `//`、`/* */`（可嵌套）、
//!   `"…"`，以及 `'x'` 那种**字符字面量**）。不跳的话会踩两个坑（都实测过）：
//!
//!   1. 文档注释里提到宏名，扫描器就从注释里开始配平括号，最后报"`…` 没有配平的 `}`"；
//!   2. 源码里写 `'"'`（比如 JSON 转义那张表）—— `'` 后面那个 `"` 会被当成**字符串的开头**，
//!      于是一路扫到文件尾，报"字符串字面量没有收尾"。
//!
//!   ⇒ 顺带一条约束：**注释/字符串里别写那个 token 后面紧跟括号的写法**，写了也不会被当成调用
//!   （这是好事），但会把"只剩最后一份口径"这件事搞混；真要提它，用反引号包起来即可。

use std::path::{Path, PathBuf};

/// 一处**历史**扫描路径上的调用：`{ 名字, "id", 声明, [链的 crate…], "源", |p, i, g, ARG| 模板 }`
/// （`20` §184 的六段；前五段 + 一段**体模板**）。
///
/// ⚠ 那条路径（图侧写宏、工具按文本读它）**今天零调用点**：图侧改成 recipe 数据表 +
///   生成器之后，体与声明事实都从 `px_graphs/build.rs` 那一侧来（见文件头）。
///   这一份留在原处是因为它是**体模板口径**的正文（`ARG` 那一栏仍活在 recipe 里）。
#[derive(Debug, Clone)]
pub struct Invocation {
    /// 它写在哪个文件里（报错时说出来）。
    pub file: PathBuf,
    /// 图侧算子类型名（`Band`）。
    pub name: String,
    /// 这一条实例的 op id 字面量（`"cloud.coarse/band"`）。
    pub op_id: String,
    /// 复用的声明**原文**（新写法是裸类型名 `CloudCoarse`；老写法 `volume::CloudCoarse` 也认）。
    pub decl: String,
    /// 泛型参数住的源文件（`art/inst/band.rs`）。
    pub source: String,
    /// 体模板里的占位符名（模板 `|p, i, g, ARG|` 那个 `ARG`）。
    pub placeholder: String,
    /// 体模板本身（`px_volume_alg::coarse_with(p, i.coverage.value(), ARG)`）。
    pub template: String,
}

impl Invocation {
    /// 类型名 = 声明那一段的**最后一段**（`volume::CloudCoarse` ⇒ `CloudCoarse`）。
    pub fn decl_name(&self) -> &str {
        self.decl
            .rsplit("::")
            .next()
            .unwrap_or(self.decl.as_str())
            .trim()
    }
}

/// 一棵源码树（通常就是图程序 crate 的 `src/`）里每一处**历史那条**宏调用
/// （按文件路径排序 ⇒ 结果确定）。⚠ 图侧今天没有那个宏了 ⇒ 零调用点，见文件头。
pub fn scan(dir: &Path) -> Result<Vec<Invocation>, String> {
    let mut files = Vec::new();
    collect_rs(dir, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .map_err(|err| format!("读不了 {}：{err}", file.display()))?;
        for call in find_named(&text, "px_inst!")? {
            found.push(parse_call(&file, &call)?);
        }
    }
    Ok(found)
}

/// 数一棵目录树里**历史那条**宏路径的真调用处数。⚠ 零调用点，见文件头。
///
/// ⚠ 数是按文本数的：它只能告诉你"源码里写了几处"，**不能**告诉你那几处编不编得过
///   （重复的 op id、写错的 crate 名都在别处报）。
pub fn count(dir: &Path) -> Result<usize, String> {
    count_named(dir, "px_inst!")
}

/// 数一棵目录树里 **`macro_name` 真调用**的处数。
///
/// ⚠ 今天的主要用途是 `px_op!`（`px_decls` 那道"声明表与声明处数一致"的门）——
///   两个宏名共用**同一份**词法口径（跳注释 / 字符串 / 字符字面量），
///   于是"两份解析器数出不同的数"那条旧病不可能再犯（`19` §180）。
pub fn count_named(dir: &Path, macro_name: &str) -> Result<usize, String> {
    let mut files = Vec::new();
    collect_rs(dir, &mut files);
    files.sort();
    let mut total = 0;
    for file in files {
        let text = std::fs::read_to_string(&file)
            .map_err(|err| format!("读不了 {}：{err}", file.display()))?;
        total += find_named(&text, macro_name)?.len();
    }
    Ok(total)
}

/// **历史路径**：图侧那个宏调用之后**配平**的那一段（`{ … }`；`(` / `[` 同样收）。
///
/// ⚠ 图侧今天没有那个宏了（见文件头）⇒ 这一条零调用点；要扫宏调用请直接用
///   [`find_named`]（`px_op!` 那道门走的就是它）。
pub fn find_calls(text: &str) -> Result<Vec<String>, String> {
    find_named(text, "px_inst!")
}

/// 找 `macro_name` 之后配平的那一段，正文**不含**最外层那对括号。
pub fn find_named(text: &str, macro_name: &str) -> Result<Vec<String>, String> {
    let needle = macro_name;
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        // ⚠ 先跳过注释与字符串：它们里面**提到**宏不算调用（文件头那两条坑）。
        if bytes[at..].starts_with(b"//") {
            match text[at..].find('\n') {
                Some(offset) => at += offset + 1,
                None => break,
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at = skip_block_comment(text, at)?;
            continue;
        }
        if bytes[at] == b'"' {
            at = skip_string(text, at)?;
            continue;
        }
        // ⚠ 单引号也要认：`'"'` 这种**字符字面量**里就有一个 `"`，不先跳过去的话，
        //   下一个字节会被当成字符串的开头，一路扫到文件尾报"没有收尾"。
        if bytes[at] == b'\'' {
            at = skip_quote(text, at);
            continue;
        }
        if bytes[at..].starts_with(needle.as_bytes()) {
            let before = text[..at].chars().next_back();
            if !before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '!') {
                let rest = &text[at + needle.len()..];
                let skip = rest
                    .find(|ch: char| !ch.is_whitespace())
                    .ok_or_else(|| format!("`{macro_name}` 之后什么都没有"))?;
                let tail = &rest[skip..];
                let Some(open) = tail.chars().next() else {
                    return Err(format!("`{macro_name}` 之后什么都没有"));
                };
                if let Some(close) = match open {
                    '{' => Some('}'),
                    '(' => Some(')'),
                    '[' => Some(']'),
                    _ => None,
                } {
                    let (body, end) = balanced(tail, open, close)?;
                    out.push(body);
                    at += needle.len() + skip + end;
                    continue;
                }
                // 后面不是三对括号之一 ⇒ 只是"提了一嘴"，继续往后扫。
            }
        }
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    Ok(out)
}

/// 跳过一段块注释（Rust 允许嵌套），回它之后的位置。
fn skip_block_comment(text: &str, at: usize) -> Result<usize, String> {
    let mut depth = 0_u32;
    let mut index = at;
    let bytes = text.as_bytes();
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            depth += 1;
            index += 2;
        } else if bytes[index..].starts_with(b"*/") {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return Ok(index);
            }
        } else {
            index += text[index..].chars().next().map_or(1, char::len_utf8);
        }
    }
    Err("块注释没有收尾（`*/`）".to_string())
}

/// 跳过一段字符串字面量，回它之后的位置（`at` 处是那个开引号）。
fn skip_string(text: &str, at: usize) -> Result<usize, String> {
    let mut index = at + 1;
    while index < text.len() {
        match text[index..].chars().next() {
            Some('\\') => index = skip_escape(text, index),
            Some('"') => return Ok(index + 1),
            Some(other) => index += other.len_utf8(),
            None => break,
        }
    }
    Err("字符串字面量没有收尾（`\"`）".to_string())
}

/// 跳过 `'` 开头的那两种东西：**字符字面量**（`'"'` / `'\n'` / `'\u{…}'`）与**生命周期**（`'a`）。
///
/// ⚠ 判据就是"收尾的单引号在不在一个字符之内"：在 ⇒ 字符字面量；不在 ⇒ 生命周期。
///   两者都得跳过那一个 `'`，否则下一个字节会被认成字符串开头（`'"'` 就是这样踩的）。
fn skip_quote(text: &str, at: usize) -> usize {
    if let Some(end) = quote_end(text, at + 1) {
        return end + 1;
    }
    at + 1
}

/// 从 `from` 起找**同一个字符字面量**的收尾单引号（找不到就回 `None` = 这是个生命周期）。
fn quote_end(text: &str, from: usize) -> Option<usize> {
    let mut index = from;
    let limit = (from + 16).min(text.len());
    while index < limit {
        let ch = text[index..].chars().next()?;
        match ch {
            '\'' => return Some(index),
            '\\' => index = skip_escape(text, index),
            other if other.is_whitespace() => return None,
            other => index += other.len_utf8(),
        }
    }
    None
}

/// 跳过一个转义序列（`\n` / `\'` / `\u{1F600}`），回它之后的位置。
fn skip_escape(text: &str, at: usize) -> usize {
    let after = &text[(at + 1).min(text.len())..];
    match after.chars().next() {
        // `\u{…}`：转义本身有好几个字符（多字节字符在 `u{` 之后，不额外消耗）。
        Some('u') if after.starts_with("u{") => match after.find('}') {
            Some(close) => at + 1 + close + 1,
            None => at + 2,
        },
        Some(escaped) => at + 1 + escaped.len_utf8(),
        // 反斜杠正好是最后一个字节（被截断的源文件）：至少挪一格，别原地打转。
        None => at + 1,
    }
}

/// 从 `tail` 的第一个 `open` 起，取到与它配平的那个 `close`（跳过字符串/字符/注释里的括号）。
///
/// 回 `(不含最外层括号的正文, 消费掉的字节数)`。
fn balanced(tail: &str, open: char, close: char) -> Result<(String, usize), String> {
    let mut depth = 0_i64;
    let mut state = State::Code;
    let mut iter = tail.char_indices().peekable();
    while let Some((index, ch)) = iter.next() {
        match state {
            State::Code => {
                if ch == '"' {
                    state = State::Text;
                } else if ch == '\'' {
                    // ⚠ `'a`（生命周期）与 `'x'`（字符）都可能出现：只看它是不是
                    //    "一个转义 + 一个引号" 那种字符字面量。
                    state = State::Quote;
                } else if ch == '/' && iter.peek().is_some_and(|(_, next)| *next == '/') {
                    state = State::Line;
                } else if ch == '/' && iter.peek().is_some_and(|(_, next)| *next == '*') {
                    state = State::Block;
                } else if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        return Ok((tail[1..index].to_string(), index + ch.len_utf8()));
                    }
                }
            }
            State::Text => {
                if ch == '\\' {
                    iter.next();
                } else if ch == '"' {
                    state = State::Code;
                }
            }
            State::Quote => {
                if ch == '\\' {
                    iter.next();
                } else if ch == '\'' {
                    state = State::Code;
                } else if !ch.is_alphanumeric() && ch != '_' {
                    // 不是字符字面量（多半是生命周期 `'a`）⇒ 回到代码态。
                    state = State::Code;
                }
            }
            State::Line => {
                if ch == '\n' {
                    state = State::Code;
                }
            }
            State::Block => {
                if ch == '*' && iter.peek().is_some_and(|(_, next)| *next == '/') {
                    iter.next();
                    state = State::Code;
                }
            }
        }
    }
    Err(format!("`{open}` 没有配平的 `{close}`"))
}

#[derive(PartialEq)]
enum State {
    Code,
    Text,
    Quote,
    Line,
    Block,
}

/// 按**顶层逗号**切（字符串 / 字符 / 注释 / 括号 / **闭包形参表 `|…|`** 里的逗号不算）。
///
/// ⚠ 体模板最后一段长这样：`|p, i, g, ARG| px_volume_alg::coarse_with(…)` —— 那两根竖线
///   把形参表夹在中间，里面的逗号**不是**分段用的逗号。把它们当中缀 `|`（按位或）看会多切出
///   三段（实测就是这么踩的）。判据：`|` 前面（当前这一段）只有空白 ⇒ 它是**开**的那根。
pub fn split_top_level(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0_i64;
    let mut closed = 0_i64;
    let mut state = State::Code;
    let mut iter = body.chars().peekable();
    while let Some(ch) = iter.next() {
        match state {
            State::Code => {
                if ch == '"' {
                    state = State::Text;
                } else if ch == '\'' {
                    state = State::Quote;
                } else if ch == '/' && iter.peek() == Some(&'/') {
                    state = State::Line;
                } else if ch == '/' && iter.peek() == Some(&'*') {
                    state = State::Block;
                } else if ch == '{' || ch == '(' || ch == '[' {
                    depth += 1;
                } else if ch == '}' || ch == ')' || ch == ']' {
                    depth -= 1;
                } else if ch == '|' && current.trim().is_empty() {
                    // 闭包形参表的开头：从这儿到配对的那根 `|`，里面的逗号都不算分段。
                    closed += 1;
                } else if ch == '|' && closed > 0 {
                    closed -= 1;
                } else if ch == ',' && depth == 0 && closed == 0 {
                    parts.push(std::mem::take(&mut current));
                    continue;
                }
                current.push(ch);
            }
            State::Text => {
                current.push(ch);
                if ch == '\\' {
                    if let Some(next) = iter.next() {
                        current.push(next);
                    }
                } else if ch == '"' {
                    state = State::Code;
                }
            }
            State::Quote => {
                current.push(ch);
                if ch == '\\' {
                    if let Some(next) = iter.next() {
                        current.push(next);
                    }
                } else if ch == '\'' {
                    state = State::Code;
                } else if !ch.is_alphanumeric() && ch != '_' {
                    state = State::Code;
                }
            }
            State::Line => {
                current.push(ch);
                if ch == '\n' {
                    state = State::Code;
                }
            }
            State::Block => {
                current.push(ch);
                if ch == '*' && iter.peek() == Some(&'/') {
                    current.push(iter.next().unwrap_or(' '));
                    state = State::Code;
                }
            }
        }
    }
    parts.push(current);
    parts
}

/// 一处宏调用的正文 → [`Invocation`]（**历史路径**，见文件头）。
pub fn parse_call(file: &Path, body: &str) -> Result<Invocation, String> {
    // ⚠ 宏调用的**第一段**可以是文档注释（`{ /// … Band, … }`）—— 它按顶层逗号
    //   切开时是一段"只有注释"的正文，得先剥掉，否则第一段是注释而不是类型名。
    let parts: Vec<String> = split_top_level(&strip_leading_comments(body))
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty() && !only_comments(part))
        .collect();
    if parts.len() != 6 {
        return Err(format!(
            "{}：一处 `px_inst` 有 {} 段（要 6 段：类型名 / id / 声明 / alg / 源 / 体模板）",
            file.display(),
            parts.len()
        ));
    }
    let name = parts[0].clone();
    if !is_ident(&name) {
        return Err(format!(
            "{}：`px_inst` 第一段 `{name}` 不是标识符",
            file.display()
        ));
    }
    let op_id = unquote(&parts[1]).ok_or_else(|| {
        format!(
            "{}：`px_inst` 第二段 `{}` 不是字符串字面量（id）",
            file.display(),
            parts[1]
        )
    })?;
    let decl = parts[2].clone();
    if decl.is_empty() {
        return Err(format!(
            "{}：`px_inst` 第三段（声明）是空的",
            file.display()
        ));
    }
    let source = unquote(&parts[4]).ok_or_else(|| {
        format!(
            "{}：`px_inst` 第五段 `{}` 不是字符串字面量（泛型参数源文件）",
            file.display(),
            parts[4]
        )
    })?;
    let (placeholder, template) = template_of(&parts[5]).ok_or_else(|| {
        format!(
            "{}：`px_inst` 第六段 `{}` 不是 `|p, i, g, ARG| …` 形状的体模板",
            file.display(),
            parts[5]
        )
    })?;
    Ok(Invocation {
        file: file.to_path_buf(),
        name,
        op_id,
        decl,
        source,
        placeholder,
        template,
    })
}

/// 体模板 `|p, i, g, ARG| <expr>` → `(ARG, <expr>)`。
fn template_of(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let rest = text.strip_prefix('|')?;
    let close = rest.find('|')?;
    let params: Vec<&str> = rest[..close].split(',').map(str::trim).collect();
    let placeholder = params.last().copied().unwrap_or_default().to_string();
    if !is_ident(&placeholder) {
        return None;
    }
    Some((placeholder, rest[close + 1..].trim().to_string()))
}

/// 剥掉正文**最前面**的空白与注释（带文档注释的宏调用就长这样）。
pub fn strip_leading_comments(text: &str) -> String {
    let mut rest = text;
    loop {
        let trimmed = rest.trim_start();
        if let Some(after) = trimmed.strip_prefix("//") {
            match after.find('\n') {
                Some(at) => rest = &after[at + 1..],
                None => return String::new(),
            }
        } else if let Some(after) = trimmed.strip_prefix("/*") {
            match after.find("*/") {
                Some(at) => rest = &after[at + 2..],
                None => return String::new(),
            }
        } else {
            return trimmed.to_string();
        }
    }
}

/// 一段正文是不是**只有注释**（顶层切出来的碎片里会出现这种）。
pub fn only_comments(text: &str) -> bool {
    let mut rest = text;
    loop {
        let trimmed = rest.trim();
        if trimmed.is_empty() {
            return true;
        }
        if let Some(after) = trimmed.strip_prefix("//") {
            match after.find('\n') {
                Some(at) => rest = &after[at + 1..],
                None => return true,
            }
        } else if let Some(after) = trimmed.strip_prefix("/*") {
            match after.find("*/") {
                Some(at) => rest = &after[at + 2..],
                None => return true,
            }
        } else {
            return false;
        }
    }
}

fn is_ident(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_')
}

fn unquote(text: &str) -> Option<String> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then(|| inner.to_string())
}

/// 一棵目录树里的全部 `.rs`（跳过 `target/` 与 `.` 开头的目录）。
pub fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let name = dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if name == "target" || name.starts_with('.') {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// 主 `Cargo.toml` 的 `members = [ … ]`（够用就行：这个仓库只有那一行数组）。
pub fn workspace_members(manifest: &str) -> Vec<String> {
    let Some(at) = manifest.find("members") else {
        return Vec::new();
    };
    let rest = &manifest[at..];
    let Some(open) = rest.find('[') else {
        return Vec::new();
    };
    let Some(close) = rest[open..].find(']') else {
        return Vec::new();
    };
    rest[open + 1..open + close]
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// 声明住的 crate：在 workspace 里找 `px_op! { <名字> … }`（回 `(包名, 声明所在文件)`）。
///
/// ⚠ 为什么不解析 `px_inst` 里那一段：那里的 `volume`（老写法）是 `px_cook` 的 re-export
///   （`px_volume_schema::ops as volume`），与包名不是一回事；而**新写法是裸类型名**，
///   压根看不出 crate。反过来说，`px_op!` 一定写在声明住的那个 crate 里（见 `contract.rs`
///   的 `decl_hash`），所以"哪个 crate 里有这一行"就是答案 —— 而且这个答案是**从盘上量出来的**。
///
/// ⚠ 扫的是每个 workspace 成员**自己 crate 里的**源码（碰到带 `Cargo.toml` 的子目录就停）：
///   从盘上后来才出现的 `target/jit/<key>/` 是一个自成 workspace 根的 crate，不进去；
///   `.` 开头的目录（`.worktrees` 那种）也整棵跳过。
pub fn declaration_crate(root: &Path, name: &str) -> Result<Option<(String, PathBuf)>, String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    let members = workspace_members(&manifest);
    let mut crates: Vec<PathBuf> = vec![root.to_path_buf()];
    crates.extend(members.iter().map(|member| root.join(member)));
    let mut out = None;
    for crate_dir in &crates {
        let mut files = Vec::new();
        walk_crate(crate_dir, &mut files)?;
        files.sort();
        for file in files {
            let text = std::fs::read_to_string(&file)
                .map_err(|err| format!("读不了 {}：{err}", file.display()))?;
            for call in find_named(&text, "px_op!")? {
                let first = split_top_level(&strip_leading_comments(&call))
                    .into_iter()
                    .map(|part| part.trim().to_string())
                    .find(|part| !part.is_empty() && !only_comments(part));
                if first.as_deref() == Some(name) {
                    let package = crate_dir
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string();
                    out = Some((package, file.clone()));
                }
            }
        }
    }
    Ok(out)
}

/// 收一个 crate **自己的** `.rs`（跳过 `target/` 与 `.` 开头的目录；碰到带 `Cargo.toml`
/// 的子目录就停）。
fn walk_crate(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let name = dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if name == "target" || name.starts_with('.') {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.join("Cargo.toml").is_file() {
                continue;
            }
            walk_crate(&path, out)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// 一个源文件在它 crate 里的**模块路径**（`<crate>/src/ops.rs` ⇒ `ops`；
/// `<crate>/src/a/b.rs` 或 `<crate>/src/a/b/mod.rs` ⇒ `a::b`；`src/lib.rs` ⇒ 空）。
///
/// ⚠ 不解析 `mod` 声明、不 cargo 调元数据：只按**模块树的位置**推 —— 本仓的模块都是
///   文件模块（没有 `#[path]`），这一条就够，而且不用起 cargo。
pub fn module_of(file: &Path, crate_dir: &Path) -> String {
    let relative = file.strip_prefix(crate_dir.join("src")).unwrap_or(file);
    let mut parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(last) = parts.pop() {
        let stem = last.strip_suffix(".rs").unwrap_or(&last).to_string();
        if stem != "lib" && stem != "mod" {
            parts.push(stem);
        }
    }
    parts.join("::")
}
