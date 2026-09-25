use std::path::Path;

use px_protocol::material::{MaterialLayout, ParamKind, ParamSlot};
use px_protocol::scene::{Member, Value};

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
            "shader 成员 {member} 的产物没有 schema descriptor：那是契约收口之前烘的。\n  \
             先重烘：cargo run -p px_graphs --bin shaders"
        )
    })?;
    let layout = MaterialLayout::from_json(&text)
        .map_err(|err| format!("shader 成员 {member} 的 descriptor 解不开：{err}"))?;
    Ok((source, layout))
}

pub fn schema_of(member: &Member, root: &Path) -> Result<MaterialLayout, String> {
    Ok(shader_parts_of(member, root)?.1)
}

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
