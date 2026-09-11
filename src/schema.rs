//! **JSON Schema 生成器的唯一入口**（读面 `--schema`/`--nouns` 与写面 `--control-schema` 都走这里）。
//!
//! 为什么不让各处直接调 `schemars::schema_for!`：那个宏用的是 `SchemaGenerator::default()`，
//! 而 schemars 1.x 的默认方言是 **JSON Schema 2020-12**（`$defs` + `#/$defs/X` 的 `$ref`），
//! 0.8 时代的默认是 **draft-07**（`definitions` + `#/definitions/X`）。读面的消费方
//! （`play/tests/g4_spec.py`、`src/tests/model/neutral.rs`、`web/static/tip.js`）走的是后者，
//! 而本仓升级 schemars（0.8.22 → 1.2.2）**只是为了修 `///` → `description` 的发射缺陷**
//! （见 [`of`] 的文档），不是为了换 schema 方言。
//!
//! 所以这里把方言**钉死在 draft-07**：升级前后 `--schema` / `--nouns` 的外形（顶层
//! `definitions`、`$ref` 前缀、`"$schema"` 的 URI）逐字不变，消费方一行都不用改。
//! 「钉方言」这件事必须**只有一处**——散在各处就迟早有一处跟着 schemars 的默认走。

/// 用本仓钉死的方言（draft-07）为 `T` 生成一份 JSON Schema。
///
/// 与 [`schemars::schema_for!`] 的唯一区别就是那套 [`SchemaSettings`]：
///
/// * `definitions_path = "/definitions"`（不是 1.x 默认的 `/$defs`）；
/// * `meta_schema = draft-07`；
/// * 三个 draft-07 兼容变换（`unevaluatedProperties` → `additionalProperties`、
///   `$ref` 的兄弟键收进 `allOf`、`prefixItems` → `items` 数组）——其中第二个正是
///   0.8 的行为，`neutral.rs::resolve` 认的就是 `{"allOf":[{"$ref":…}],"description":…}` 这层。
///
/// **本仓为什么升级 schemars**：0.8.22 的 derive 里有一段向后兼容 hack
/// （`schemars_derive-0.8.22/src/attr/doc.rs`）——只要文档的**所有行**都以 `*` 开头，
/// 就当成 `/** … */` 风格把每行首的 `*` 剥掉。于是**单行**注释若以 `**加粗**` 开头
/// （`/// **完整的世界快照**（…`）会被误判，读面收到的是 `*完整的世界快照**（…`
/// ——**坏掉的 markdown**（实测 46/46 条坏样本全是单行注释）。
/// 1.x 重写了 doc→description（`schemars-1.2.2/src/_private/rustdoc.rs`，
/// `get_title_and_description`）= 只做 `trim` + `#` 首行取标题，**没有任何剥 `*` 的逻辑**。
///
/// [`SchemaSettings`]: schemars::generate::SchemaSettings
pub fn of<T: schemars::JsonSchema + ?Sized>() -> serde_json::Value {
    let mut generator =
        schemars::SchemaGenerator::new(schemars::generate::SchemaSettings::draft07());
    let mut out =
        serde_json::to_value(generator.root_schema_for::<T>()).expect("schema is serializable");
    merge_descriptions(&mut out);
    out
}

/// 把 schema 里**每一个** `description` 按「散文」归一（见 [`merge_doc_lines`]）。
///
/// 只动 `description`：`title` 是单行、`column_docs`/`leaves` 那些手写文案本来就已经是定稿，
/// 都不该被这里改写。
fn merge_descriptions(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if key == "description" {
                    if let Some(text) = val.as_str() {
                        let merged = merge_doc_lines(text);
                        if merged != text {
                            *val = serde_json::Value::String(merged);
                        }
                        continue;
                    }
                }
                merge_descriptions(val);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(merge_descriptions),
        _ => {}
    }
}

/// 源码 `///` 的**硬换行是给人看代码用的**（~100 列），不是给弹窗用的：
/// 弹窗只有 420px 宽，逐行保留换行 ⇒ 每一条 ~100 字的源码行都会被再折一次，
/// 段落变成一段锯齿。所以读面上把**段内**换行归一成空格，只保留**段间**空行
/// （两段之间仍是 `\n\n`，`tip.js` 渲染成换行）。
///
/// 这条规则**不是新发明的**：它就是 schemars 0.8 的 `merge_description_lines`
/// （`schemars_derive-0.8.22/src/attr/doc.rs`）——0.8 的读面文案一直长这样，1.x 只是
/// 把「原样的 doc 文本」交出来。升级时在这里补回一次，为的是让读面**只有该变的东西变**：
/// `--nouns` 与升级前逐条对账，差异只剩那 46 条被剥掉一个 `*` 的（见 [`of`]）。
///
/// ⚠ 顺序不能换：0.8 是**先把每一行 trim 掉**、再用 `\n\n` 切段的
/// （`get_doc` 里 `.map(str::trim)`）。先切段再 trim 行的话，
/// `/// 上溯 \n/// \n/// 全局` 这种「行尾带空格 + 空行」就切不出段来。
fn merge_doc_lines(doc: &str) -> String {
    let lines = doc
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    lines
        .trim()
        .split("\n\n")
        .map(|para| para.trim().replace('\n', " "))
        .filter(|para| !para.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::merge_doc_lines;

    #[test]
    fn merge_doc_lines_joins_within_paragraph_keeps_blank_line() {
        assert_eq!(merge_doc_lines("a\nb\n\nc\nd"), "a b\n\nc d");
        // 空行多打几个也不该多出段落（0.8 的 `none_if_empty` 过滤行为）。
        assert_eq!(merge_doc_lines("a\n\n\n\nb"), "a\n\nb");
        // 行尾的空格跟着行一起 trim 掉：`上溯 \n\n 全局` 仍是**两段**
        // （先 trim 行、再切段——顺序反了就切不出这一段）。
        assert_eq!(merge_doc_lines("上溯 \n \n 全局"), "上溯\n\n全局");
        // 单行（含以 `*` 开头的那种）**逐字不动**——这条 hack 修的就是它们。
        assert_eq!(
            merge_doc_lines("**加粗**开头（…）。"),
            "**加粗**开头（…）。"
        );
    }
}
