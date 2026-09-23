//! 配方词汇 → shader 词汇：**这一份是两条烘图路（`scene` 与 `passes`）共用的那一半**。
//!
//! 这里的两个函数原来住在 `bin/scene.rs` 里：材质那条路走通之后，pass 那条路要的是**同一件事**
//! ——「配方里写了一个名字，这份 shader 声明了它吗？声明的是哪一档？」。
//! 抄第二份就是第二个会漂开的真相（§66 的病根），所以搬到这里。

use std::path::Path;

use px_protocol::material::{MaterialLayout, ParamKind, ParamSlot};
use px_protocol::scene::{Member, Value};

/// 一份 shader 成员产物里的**两半**：组装前的 WGSL 文本 + 它的契约。
///
/// 为什么要一次读两半：`--bin passes` 要拿 WGSL 去验「配方写的入口名在不在这份 shader 里」
/// （`bad_entry` 那条判据），而契约也要按它校验参数。分两次读就是两次磁盘往返 + 两处
/// "读哪个文件"的知识。
pub fn shader_parts_of(member: &Member, root: &Path) -> Result<(String, MaterialLayout), String> {
    let path = px_protocol::scene::cas_path(root, &member.key)?;
    let (source, schema) = px_protocol::art::read_shader_parts(&path).map_err(|err| {
        format!(
            "读 shader 成员 {member} 的产物失败（{}）：{err}",
            path.display()
        )
    })?;
    let text = schema.ok_or_else(|| {
        format!(
            "shader 成员 {member} 的产物没有 schema descriptor：那是契约收口（§80）之前烘的。\n  \
             先重烘：cargo run -p px_graphs --bin shaders"
        )
    })?;
    let layout = MaterialLayout::from_json(&text)
        .map_err(|err| format!("shader 成员 {member} 的 descriptor 解不开：{err}"))?;
    Ok((source, layout))
}

/// 一份 shader 成员的**契约**：从它的产物里读 schema descriptor（第二个 blob，`08-renderer.md` §80.2）。
///
/// 为什么不现反射：产物里那份就是**装载时会被拿来对账的那一份** —— 烘图侧要校验的是
/// 「这份产物说它要什么」，不是「现在这份 WGSL 会反射出什么」。两者不一致时装载会拒，
/// 那时候再报错就晚了（而且报的是渲染器的错，不是作者写错了配方）。
pub fn schema_of(member: &Member, root: &Path) -> Result<MaterialLayout, String> {
    Ok(shader_parts_of(member, root)?.1)
}

/// TOML 里的一个值 → 产物里的值，**按 shader 声明的那一档**。
pub fn coerce_value(slot: &ParamSlot, value: &toml::Value) -> Result<Value, String> {
    let label = slot.kind.name();
    match slot.kind {
        ParamKind::F32 | ParamKind::I32 | ParamKind::U32 => match value {
            toml::Value::Integer(number) => Ok(Value::Num(*number as f64)),
            toml::Value::Float(number) => Ok(Value::Num(*number)),
            other => Err(format!("要一个 {label}，实际是 {other:?}")),
        },
        ParamKind::Vec3 | ParamKind::Vec4 => {
            let toml::Value::Array(items) = value else {
                return Err(format!("要 {label}（一个数组），实际是 {value:?}"));
            };
            let wanted = if slot.kind == ParamKind::Vec3 { 3 } else { 4 };
            if items.len() != wanted {
                return Err(format!(
                    "要 {label}（{wanted} 个数），实际给了 {} 个",
                    items.len()
                ));
            }
            let mut numbers = [0.0_f32; 4];
            for (index, item) in items.iter().enumerate() {
                numbers[index] = match item {
                    toml::Value::Integer(number) => *number as f32,
                    toml::Value::Float(number) => *number as f32,
                    other => return Err(format!("第 {} 个数不是数：{other:?}", index + 1)),
                };
            }
            if wanted == 3 {
                Ok(Value::Triple([numbers[0], numbers[1], numbers[2]]))
            } else {
                Ok(Value::Quad(numbers))
            }
        }
    }
}

/// 按名字透传 + 烘图时按契约校验一遍。
///
/// 三档，逐条都当场说清楚（§79 的 W1：原来「配方里能写什么」是一张写死的白名单，
/// 加一个 shader 参数就得改那张表 —— 也就是改 Rust）：
/// · 名字在 `structural` 里 ⇒ 编译器自己要用它，**不进** shader 的参数表；
/// · 名字在这份 shader 的参数表里 ⇒ 按它声明的类型透传（**这就是「加一个参数不用改 Rust」**）；
/// · 两边都不是 ⇒ 报错，并把两张表都列出来。
///
/// 最后一步 `pack` 是烘图时的**完整性**校验：shader 声明了而没人给值 ⇒ 这里就红，
/// 不会烘出一份装载时必被拒的产物。
pub fn merge_named(
    what: &str,
    given: &std::collections::BTreeMap<String, toml::Value>,
    structural: &[&str],
    layout: &MaterialLayout,
    computed: std::collections::BTreeMap<String, Value>,
) -> Result<std::collections::BTreeMap<String, Value>, String> {
    let mut params = computed;
    for (key, value) in given {
        if structural.contains(&key.as_str()) {
            continue;
        }
        let Some(slot) = layout.param(key) else {
            return Err(format!(
                "{what} 不认识参数 '{key}'：\n  \
                 编译器自己消化的结构键：{}\n  \
                 这份 shader 声明的参数：{}\n  \
                 ⇒ 要么名字拼错了，要么得先在 shader 的结构体里声明它（声明之后按名字透传，不用改 Rust）",
                if structural.is_empty() {
                    "（没有）".to_string()
                } else {
                    structural.join(" / ")
                },
                layout.param_names(),
            ));
        };
        let value =
            coerce_value(slot, value).map_err(|err| format!("{what} 的参数 '{key}'：{err}"))?;
        params.insert(key.clone(), value);
    }
    layout
        .pack(&params)
        .map_err(|err| format!("{what} 的参数对不上这份 shader 的契约：{err}"))?;
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> MaterialLayout {
        MaterialLayout {
            params: vec![
                ParamSlot {
                    name: "strength".to_string(),
                    offset: 0,
                    kind: ParamKind::F32,
                },
                ParamSlot {
                    name: "tint".to_string(),
                    offset: 16,
                    kind: ParamKind::Vec3,
                },
            ],
            params_bytes: 32,
            textures: Vec::new(),
        }
    }

    fn toml_of(text: &str) -> std::collections::BTreeMap<String, toml::Value> {
        toml::from_str::<toml::Value>(text)
            .expect("测试夹具")
            .as_table()
            .expect("一张表")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    #[test]
    fn a_name_the_shader_declares_is_passed_through_by_its_own_kind() {
        let params = merge_named(
            "pass 'grade'",
            &toml_of("strength = 1\ntint = [1.0, 2.0, 3.0]\n"),
            &[],
            &layout(),
            Default::default(),
        )
        .expect("名字都在契约里 ⇒ 透传");
        assert_eq!(params["strength"], Value::Num(1.0));
        assert_eq!(params["tint"], Value::Triple([1.0, 2.0, 3.0]));
    }

    #[test]
    fn an_unknown_name_lists_both_tables() {
        let err = merge_named(
            "pass 'grade'",
            &toml_of("strengh = 1\n"),
            &[],
            &layout(),
            Default::default(),
        )
        .expect_err("拼错一个名字必须当场红");
        assert!(err.contains("strengh"), "{err}");
        assert!(err.contains("strength"), "{err}");
    }

    #[test]
    fn a_declared_parameter_nobody_gave_is_caught_at_bake_time() {
        let err = merge_named(
            "pass 'grade'",
            &Default::default(),
            &[],
            &layout(),
            Default::default(),
        )
        .expect_err("声明了却没人给 ⇒ 烘图时就该红");
        assert!(err.contains("strength"), "{err}");
    }

    #[test]
    fn the_wrong_shape_is_refused_by_the_declared_kind() {
        let err = merge_named(
            "pass 'grade'",
            &toml_of("strength = [1.0, 2.0]\n"),
            &[],
            &layout(),
            Default::default(),
        )
        .expect_err("数给了数组 ⇒ 拒");
        assert!(err.contains("strength"), "{err}");
    }

    #[test]
    fn a_structural_key_is_left_to_the_compiler() {
        let mut given = toml_of("strength = 0.5\ntint = [1.0, 1.0, 1.0]\n");
        given.insert("radius".to_string(), toml::Value::Float(1.0));
        let params = merge_named(
            "pass 'grade'",
            &given,
            &["radius"],
            &layout(),
            Default::default(),
        )
        .expect("结构键跳过，不进 shader 参数表");
        assert!(!params.contains_key("radius"), "{params:?}");
        assert_eq!(params["strength"], Value::Num(0.5));
    }
}
