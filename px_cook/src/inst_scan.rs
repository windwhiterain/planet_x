use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Invocation {
    pub file: PathBuf,
    pub name: String,
    pub op_id: String,
    pub decl: String,
    pub source: String,
    pub placeholder: String,
    pub template: String,
}

impl Invocation {
    pub fn decl_name(&self) -> &str {
        self.decl
            .rsplit("::")
            .next()
            .unwrap_or(self.decl.as_str())
            .trim()
    }
}

pub fn scan(dir: &Path) -> Result<Vec<Invocation>, px_graph_schema::Fault> {
    let mut files = Vec::new();
    collect_rs(dir, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).map_err(|err| {
            px_graph_schema::Fault::new(
                px_graph_schema::Kind::Operator,
                format!("读不了 {}：{err}", file.display()),
            )
        })?;
        for call in find_named(&text, "px_inst!")? {
            found.push(parse_call(&file, &call)?);
        }
    }
    Ok(found)
}

pub fn count(dir: &Path) -> Result<usize, String> {
    count_named(dir, "px_inst!").map_err(String::from)
}

pub fn count_named(dir: &Path, macro_name: &str) -> Result<usize, px_graph_schema::Fault> {
    let mut files = Vec::new();
    collect_rs(dir, &mut files);
    files.sort();
    let mut total = 0;
    for file in files {
        let text = std::fs::read_to_string(&file).map_err(|err| {
            px_graph_schema::Fault::new(
                px_graph_schema::Kind::Operator,
                format!("读不了 {}：{err}", file.display()),
            )
        })?;
        total += find_named(&text, macro_name)?.len();
    }
    Ok(total)
}

pub fn find_calls(text: &str) -> Result<Vec<String>, String> {
    find_named(text, "px_inst!").map_err(String::from)
}

pub fn find_named(text: &str, macro_name: &str) -> Result<Vec<String>, px_graph_schema::Fault> {
    let needle = macro_name;
    let bytes = text.as_bytes();
    let mut out = Vec::new();
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
            at = skip_block_comment(text, at)?;
            continue;
        }
        if bytes[at] == b'"' {
            at = skip_string(text, at)?;
            continue;
        }
        if bytes[at] == b'\'' {
            at = skip_quote(text, at);
            continue;
        }
        if bytes[at..].starts_with(needle.as_bytes()) {
            let before = text[..at].chars().next_back();
            if !before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '!') {
                let rest = &text[at + needle.len()..];
                let skip = rest.find(|ch: char| !ch.is_whitespace()).ok_or_else(|| {
                    px_graph_schema::Fault::new(
                        px_graph_schema::Kind::Operator,
                        format!("`{macro_name}` 之后什么都没有"),
                    )
                })?;
                let tail = &rest[skip..];
                let Some(open) = tail.chars().next() else {
                    return Err(px_graph_schema::Fault::new(
                        px_graph_schema::Kind::Internal,
                        format!("`{macro_name}` 之后什么都没有"),
                    ));
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
            }
        }
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    Ok(out)
}

fn skip_block_comment(text: &str, at: usize) -> Result<usize, px_graph_schema::Fault> {
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
    Err(px_graph_schema::Fault::new(
        px_graph_schema::Kind::Operator,
        "块注释没有收尾（`*/`）",
    ))
}

fn skip_string(text: &str, at: usize) -> Result<usize, px_graph_schema::Fault> {
    let mut index = at + 1;
    while index < text.len() {
        match text[index..].chars().next() {
            Some('\\') => index = skip_escape(text, index),
            Some('"') => return Ok(index + 1),
            Some(other) => index += other.len_utf8(),
            None => break,
        }
    }
    Err(px_graph_schema::Fault::new(
        px_graph_schema::Kind::Operator,
        "字符串字面量没有收尾（`\"`）",
    ))
}

fn skip_quote(text: &str, at: usize) -> usize {
    if let Some(end) = quote_end(text, at + 1) {
        return end + 1;
    }
    at + 1
}

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

fn skip_escape(text: &str, at: usize) -> usize {
    let after = &text[(at + 1).min(text.len())..];
    match after.chars().next() {
        Some('u') if after.starts_with("u{") => match after.find('}') {
            Some(close) => at + 1 + close + 1,
            None => at + 2,
        },
        Some(escaped) => at + 1 + escaped.len_utf8(),
        None => at + 1,
    }
}

fn balanced(
    tail: &str,
    open: char,
    close: char,
) -> Result<(String, usize), px_graph_schema::Fault> {
    let mut depth = 0_i64;
    let mut state = State::Code;
    let mut iter = tail.char_indices().peekable();
    while let Some((index, ch)) = iter.next() {
        match state {
            State::Code => {
                if ch == '"' {
                    state = State::Text;
                } else if ch == '\'' {
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
    Err(px_graph_schema::Fault::new(
        px_graph_schema::Kind::Operator,
        format!("`{open}` 没有配平的 `{close}`"),
    ))
}

#[derive(PartialEq)]
enum State {
    Code,
    Text,
    Quote,
    Line,
    Block,
}

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

pub fn parse_call(file: &Path, body: &str) -> Result<Invocation, px_graph_schema::Fault> {
    let parts: Vec<String> = split_top_level(&strip_leading_comments(body))
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty() && !only_comments(part))
        .collect();
    if parts.len() != 6 {
        return Err(px_graph_schema::Fault::new(
            px_graph_schema::Kind::Params,
            format!(
                "{}：一处 `px_inst` 有 {} 段（要 6 段：类型名 / id / 声明 / alg / 源 / 体模板）",
                file.display(),
                parts.len()
            ),
        ));
    }
    let name = parts[0].clone();
    if !is_ident(&name) {
        return Err(px_graph_schema::Fault::new(
            px_graph_schema::Kind::Params,
            format!("{}：`px_inst` 第一段 `{name}` 不是标识符", file.display()),
        ));
    }
    let op_id = unquote(&parts[1]).ok_or_else(|| {
        px_graph_schema::Fault::new(
            px_graph_schema::Kind::Params,
            format!(
                "{}：`px_inst` 第二段 `{}` 不是字符串字面量（id）",
                file.display(),
                parts[1]
            ),
        )
    })?;
    let decl = parts[2].clone();
    if decl.is_empty() {
        return Err(px_graph_schema::Fault::new(
            px_graph_schema::Kind::Params,
            format!("{}：`px_inst` 第三段（声明）是空的", file.display()),
        ));
    }
    let source = unquote(&parts[4]).ok_or_else(|| {
        px_graph_schema::Fault::new(
            px_graph_schema::Kind::Params,
            format!(
                "{}：`px_inst` 第五段 `{}` 不是字符串字面量（泛型参数源文件）",
                file.display(),
                parts[4]
            ),
        )
    })?;
    let (placeholder, template) = template_of(&parts[5]).ok_or_else(|| {
        px_graph_schema::Fault::new(
            px_graph_schema::Kind::Params,
            format!(
                "{}：`px_inst` 第六段 `{}` 不是 `|p, i, g, ARG| …` 形状的体模板",
                file.display(),
                parts[5]
            ),
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

pub fn declaration_crate(
    root: &Path,
    name: &str,
) -> Result<Option<(String, PathBuf)>, px_graph_schema::Fault> {
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
            let text = std::fs::read_to_string(&file).map_err(|err| {
                px_graph_schema::Fault::new(
                    px_graph_schema::Kind::Operator,
                    format!("读不了 {}：{err}", file.display()),
                )
            })?;
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
