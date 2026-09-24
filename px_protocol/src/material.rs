//! 材质契约：**唯一的一份表**（§74.3 第 1 步的「契约收口」）。
//!
//! 收口之前这张表被抄在八个地方（§67.4）：`px_render::reflect` 的真源、`slots.rs` 占位 WGSL
//! 的手抄、`shaders.rs` 把组号替成字面量 `"2"`、`scene.rs` 的半份注释与 `check()`、
//! `Value` 与 `ParamKind` 两半词表、烘图侧的两张白名单、探针侧的 `CloudParams` 镜像、
//! 以及把具体偏移钉死的测试。**每多抄一份就多一个「两边会漂开」的地方**，
//! 而漂开的症状是画面错，不是报错。
//!
//! ⚠ 这里的类型只许用 `serde` / `serde_json`（`px_protocol` 的运行时依赖被
//! `tests/crate_graph.rs` 钉死只有这两个）⇒ **naga 反射不住这里**，
//! 它住在叶子 crate `px_shader::reflect`，产出的就是下面这个 [`MaterialLayout`]。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::scene::Value;

/// 材质绑定组在**运行期**是第几组。⚠ 这个数不是我们挑的：Bevy 把它写死了
/// （`bevy_pbr-0.19.1/src/material.rs:74` 的 `MATERIAL_BIND_GROUP_INDEX = 3`），
/// 材质管线特化时又把 `ShaderDefVal::UInt("MATERIAL_BIND_GROUP", 3)` 塞进 shader defs
/// （同文件 `:466-473`），naga_oil 据此把 WGSL 里的 `#{MATERIAL_BIND_GROUP}` 替成 3。
///
/// **离线组装必须替同一个数**：替成别的值，门测的就不是运行期那份文本（§75 记的「2/3 那颗雷」：
/// 在此之前组装器替的是 2、而运行期是 3，两个世界各自自洽所以一直没坏）。
pub const MATERIAL_BIND_GROUP: u32 = 3;

/// 参数块占绑定组的第 0 格，必须是 `var<uniform>` 的一个 `struct`。
pub const PARAMS_BINDING: u32 = 0;

/// 贴图格：`(绑定下标, 维度)`。采样器永远在 `+1`。
///
/// **布局必须与实例无关**（Bevy 的 `MaterialPlugin` 每种材质类型只建一份绑定布局），
/// 所以这是一张**固定超集**：空着的格一律绑兜底贴图/采样器，shader 里声明了就一定绑得上。
/// ⇒ 「多加一张贴图」的天花板就是这个数组的长度（§74.4 的硬顶之一）。
///
/// **2026-09-16 加宽（§74.4 裁决 (a)）**：4 格 → **12 格（8×2D + 4×cube）**，参数上限
/// 1024 → 4096 字节。原来那四格 `1 / 3 / 5 / 7` **一个都没动** —— 加宽只许往后**追加**：
/// 挪老格就等于把每一份既有 shader 的贴图悄悄换到别的格上（`material.rs` 的单测钉着这一条）。
/// 判据：产物键逐字节不变（表不进键）＋出图哈希不变（空格的代价见 `docs/render/schema-limits.md`）。
/// 量到的代价：`app p50 37.07 ms / gpu p50 10.43 ms`（960×640、`orbit`、Vulkan、4 格）
/// → 12 格见 `target/step1/` 的 A/B 两份报告。
pub const TEXTURE_SLOTS: [(u32, TextureDimension); 12] = [
    (1, TextureDimension::D2),
    (3, TextureDimension::D2),
    (5, TextureDimension::Cube),
    (7, TextureDimension::Cube),
    (9, TextureDimension::D2),
    (11, TextureDimension::D2),
    (13, TextureDimension::D2),
    (15, TextureDimension::D2),
    (17, TextureDimension::D2),
    (19, TextureDimension::D2),
    (21, TextureDimension::Cube),
    (23, TextureDimension::Cube),
];

/// 参数块的字节上限。布局上不写死大小（`min_binding_size = None`，每个材质各自一块 buffer），
/// 这条只是「别把 uniform 撑爆」的护栏 —— 它也是硬顶之一（§74.4）。
/// 2026-09-16：1024 → 4096（与贴图格一起加宽；实测形态见 §75 的 E6：1088 字节当场被拒）。
pub const MAX_PARAMS_BYTES: u32 = 4096;

/// uniform 的绑定大小按 16 字节对齐（`mat4` / `vec4` 的天然对齐）。
pub const PARAMS_ALIGN: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureDimension {
    // `texture_depth_2d_array`（每面一层）—— 每级一张影子 atlas（金字塔降采样）。
    // ⚠ 与 `px_pass::Dimension::D2Array` 是一对：那边是执行器的槽位形状，这边是
    //    反射出来的形状。两边不一致的症状是「材质槽位对不上」。
    D2Array,
    D2,
    Cube,
}

impl TextureDimension {
    pub fn name(self) -> &'static str {
        match self {
            Self::D2 => "texture_2d",
            Self::Cube => "texture_cube",
            Self::D2Array => "texture_depth_2d_array",
        }
    }

    /// 贴图产物那边对应的层数：2D = 1 层、cube = 6 层。
    pub fn layers(self) -> u32 {
        match self {
            Self::D2 => 1,
            Self::Cube => crate::art::CUBE_FACES,
            // 每级一张影子 atlas：每面一层 ⇒ 六层（层号 = 灯×6 + 面）。
            Self::D2Array => crate::art::CUBE_FACES,
        }
    }
}

/// 参数的类型档。只收**产物能表达**的那几种（[`Value`] 只有 数 / 文本 / 三数 / 四数）：
/// 多一种类型就是多一条「产物里写不出来」的路。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    F32,
    I32,
    U32,
    Vec3,
    Vec4,
}

impl ParamKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
        }
    }

    pub fn width(self) -> u32 {
        match self {
            Self::F32 | Self::I32 | Self::U32 => 4,
            Self::Vec3 => 12,
            Self::Vec4 => 16,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamSlot {
    pub name: String,
    pub offset: u32,
    pub kind: ParamKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureSlot {
    pub binding: u32,
    pub dimension: TextureDimension,
    /// **深度**贴图（`texture_depth_2d_array` / `texture_depth_2d`）⇒ 绑定组那一格要
    /// `TextureSampleType::Depth` + `Comparison` 采样器（`px_pass::Slot::depth`）。
    ///
    /// ⚠ 它从反射来（naga 的 `ImageClass::Depth`），不是从名字猜 —— 猜错的症状是
    ///    「布局与绑定的类型对不上」在**建管线/建组时**才炸，离病因很远。
    #[serde(default)]
    pub depth: bool,
}

/// 一份 shader 反射出来的材质契约：参数块的布局 + 贴图格的维度。
///
/// 它同时是 **schema descriptor**：烘图时算一份写进 shader 产物（第二个 blob），
/// 装载时用同一套规则再算一份跟它对账 —— 对不上说明产物是拿另一版反射规则烘的，
/// 那就当场拒（与 include 闭包那道闸门同款，§52.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialLayout {
    pub params: Vec<ParamSlot>,
    /// 打包出来的 uniform 缓冲大小（16 对齐，至少 16）。
    pub params_bytes: u32,
    pub textures: Vec<TextureSlot>,
}

impl MaterialLayout {
    pub fn param(&self, name: &str) -> Option<&ParamSlot> {
        self.params.iter().find(|slot| slot.name == name)
    }

    pub fn texture(&self, binding: u32) -> Option<&TextureSlot> {
        self.textures.iter().find(|slot| slot.binding == binding)
    }

    /// 参数名清单（报错用）。
    pub fn param_names(&self) -> String {
        if self.params.is_empty() {
            return "（它一个参数都没声明）".to_string();
        }
        self.params
            .iter()
            .map(|slot| format!("{}: {}", slot.name, slot.kind.name()))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// 规范 JSON：**字段顺序由结构体决定**，没有 map 迭代顺序可以漂
    /// ⇒ 同一份布局两次序列化逐字节相同（判据 D 的前提）。
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|err| err.to_string())
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|err| format!("schema descriptor 解不开：{err}"))
    }

    /// 把产物里按名字给的参数**按 shader 自己声明的布局**打成字节。
    ///
    /// 三档都当场报错，不静默：缺参（shader 要、产物没给）、多参（产物给了、shader 没声明）、
    /// 类型不符（数是小数却要 `u32`、三数给了四数……）。
    pub fn pack(&self, params: &BTreeMap<String, Value>) -> Result<Vec<u8>, String> {
        for key in params.keys() {
            if self.param(key).is_none() {
                return Err(format!(
                    "产物给了参数 '{key}'，但这份 shader 没声明它；它声明的参数：{}",
                    self.param_names()
                ));
            }
        }

        let mut bytes = vec![0_u8; self.params_bytes as usize];
        for slot in &self.params {
            let value = params.get(&slot.name).ok_or_else(|| {
                format!(
                    "shader 声明了参数 '{}'（{}），产物没给；它声明的参数：{}",
                    slot.name,
                    slot.kind.name(),
                    self.param_names()
                )
            })?;
            let start = slot.offset as usize;
            let end = start + slot.kind.width() as usize;
            let Some(target) = bytes.get_mut(start..end) else {
                return Err(format!(
                    "参数 '{}' 落在 {}..{}，超出了参数块（{} 字节）",
                    slot.name, start, end, self.params_bytes
                ));
            };
            write_value(slot, value, target)?;
        }
        Ok(bytes)
    }

    /// 参数块里有没有超出上限（反射时用同一份判断）。
    pub fn too_big(&self) -> Option<String> {
        (self.params_bytes > MAX_PARAMS_BYTES).then(|| {
            format!(
                "参数块是 {} 字节，超过上限 {MAX_PARAMS_BYTES}",
                self.params_bytes
            )
        })
    }
}

/// 贴图格清单（报错用）：`1 / 3 / 5 / 7`。
pub fn texture_bindings() -> String {
    TEXTURE_SLOTS
        .iter()
        .map(|(binding, _)| binding.to_string())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 一格贴图在表里的位置（`None` = 这一格不归材质）。
pub fn texture_slot_of(binding: u32) -> Option<TextureDimension> {
    TEXTURE_SLOTS
        .iter()
        .find(|(slot, _)| *slot == binding)
        .map(|(_, dimension)| *dimension)
}

fn number_of(slot: &ParamSlot, value: &Value) -> Result<f64, String> {
    match value {
        Value::Num(number) => Ok(*number),
        other => Err(format!(
            "参数 '{}' 是 {}，shader 要的是 {}",
            slot.name,
            describe(other),
            slot.kind.name()
        )),
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Num(_) => "一个数",
        Value::Text(_) => "一段文本",
        Value::Triple(_) => "三个数",
        Value::Quad(_) => "四个数",
    }
}

fn write_value(slot: &ParamSlot, value: &Value, target: &mut [u8]) -> Result<(), String> {
    match slot.kind {
        ParamKind::F32 => {
            let number = number_of(slot, value)? as f32;
            target.copy_from_slice(&number.to_le_bytes());
        }
        ParamKind::U32 => {
            let number = number_of(slot, value)?;
            if number.fract() != 0.0 || !(0.0..=u32::MAX as f64).contains(&number) {
                return Err(format!(
                    "参数 '{}' 是 {number}，shader 要的是 u32（非负整数）",
                    slot.name
                ));
            }
            target.copy_from_slice(&(number as u32).to_le_bytes());
        }
        ParamKind::I32 => {
            let number = number_of(slot, value)?;
            if number.fract() != 0.0 || !(i32::MIN as f64..=i32::MAX as f64).contains(&number) {
                return Err(format!(
                    "参数 '{}' 是 {number}，shader 要的是 i32（整数）",
                    slot.name
                ));
            }
            target.copy_from_slice(&(number as i32).to_le_bytes());
        }
        ParamKind::Vec3 => match value {
            Value::Triple(items) => {
                for (index, item) in items.iter().enumerate() {
                    target[index * 4..index * 4 + 4].copy_from_slice(&item.to_le_bytes());
                }
            }
            other => {
                return Err(format!(
                    "参数 '{}' 是 {}，shader 要的是三个数",
                    slot.name,
                    describe(other)
                ));
            }
        },
        ParamKind::Vec4 => match value {
            Value::Quad(items) => {
                for (index, item) in items.iter().enumerate() {
                    target[index * 4..index * 4 + 4].copy_from_slice(&item.to_le_bytes());
                }
            }
            other => {
                return Err(format!(
                    "参数 '{}' 是 {}，shader 要的是四个数",
                    slot.name,
                    describe(other)
                ));
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> MaterialLayout {
        MaterialLayout {
            params: vec![
                ParamSlot {
                    name: "orientation".to_string(),
                    offset: 0,
                    kind: ParamKind::Vec4,
                },
                ParamSlot {
                    name: "steps".to_string(),
                    offset: 16,
                    kind: ParamKind::U32,
                },
                ParamSlot {
                    name: "gain".to_string(),
                    offset: 20,
                    kind: ParamKind::F32,
                },
            ],
            params_bytes: 32,
            textures: vec![TextureSlot {
                binding: 5,
                dimension: TextureDimension::Cube,
                depth: false,
            }],
        }
    }

    fn params(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn packing_uses_the_offsets_the_layout_declared() {
        let bytes = layout()
            .pack(&params(&[
                ("orientation", Value::Quad([1.0, 2.0, 3.0, 4.0])),
                ("steps", Value::Num(7.0)),
                ("gain", Value::Num(0.5)),
            ]))
            .expect("打包失败");
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[0..4], &1.0_f32.to_le_bytes());
        assert_eq!(&bytes[12..16], &4.0_f32.to_le_bytes());
        assert_eq!(&bytes[16..20], &7_u32.to_le_bytes(), "u32 按整数位打包");
        assert_eq!(&bytes[20..24], &0.5_f32.to_le_bytes());
        assert_eq!(&bytes[24..32], &[0_u8; 8], "没声明的尾部是零");
    }

    #[test]
    fn missing_extra_and_ill_typed_params_are_errors() {
        let full = [
            ("orientation", Value::Quad([0.0; 4])),
            ("steps", Value::Num(1.0)),
            ("gain", Value::Num(1.0)),
        ];
        let mut missing = params(&full);
        missing.remove("gain");
        let err = layout().pack(&missing).expect_err("缺参必须报错");
        assert!(err.contains("gain"), "报错要点名：{err}");

        let mut extra = params(&full);
        extra.insert("nonsense".to_string(), Value::Num(1.0));
        let err = layout().pack(&extra).expect_err("多参必须报错");
        assert!(err.contains("nonsense"), "报错要点名：{err}");

        let mut fractional = params(&full);
        fractional.insert("steps".to_string(), Value::Num(1.5));
        let err = layout().pack(&fractional).expect_err("小数给 u32 必须报错");
        assert!(err.contains("steps"), "报错要点名：{err}");

        let mut text = params(&full);
        text.insert("gain".to_string(), Value::Text("软".to_string()));
        let err = layout().pack(&text).expect_err("类型不符必须报错");
        assert!(err.contains("gain"), "报错要点名：{err}");
    }

    #[test]
    fn the_descriptor_json_is_stable_and_round_trips() {
        let once = layout().to_json().expect("序列化");
        let twice = MaterialLayout::from_json(&once)
            .expect("反序列化")
            .to_json()
            .expect("再序列化");
        assert_eq!(once, twice, "同一份布局两次序列化必须逐字节相同");
        assert_eq!(once, layout().to_json().expect("序列化"));
        assert!(
            once.contains("\"binding\":5"),
            "贴图格要进 descriptor：{once}"
        );
        assert!(!once.contains('\n'), "规范 JSON 不带换行");
        assert!(MaterialLayout::from_json("{").is_err(), "坏 JSON 要报错");
    }

    #[test]
    fn the_table_says_where_textures_may_go() {
        let listed = texture_bindings();
        assert!(
            listed.starts_with("1 / 3 / 5 / 7"),
            "老四格一个都不许挪（挪了就是把既有 shader 的贴图换到别的格上）：{listed}"
        );
        assert_eq!(
            listed.split(" / ").count(),
            TEXTURE_SLOTS.len(),
            "清单要覆盖全表：{listed}"
        );
        assert!(
            TEXTURE_SLOTS.iter().all(|(binding, _)| binding % 2 == 1),
            "贴图占奇数格、采样器占 +1"
        );
        assert_eq!(
            TEXTURE_SLOTS
                .iter()
                .filter(|(_, dimension)| *dimension == TextureDimension::D2)
                .count(),
            8,
            "加宽之后是 8 张 2D"
        );
        assert_eq!(
            TEXTURE_SLOTS
                .iter()
                .filter(|(_, dimension)| *dimension == TextureDimension::Cube)
                .count(),
            4,
            "加宽之后是 4 张 cube"
        );
        assert_eq!(texture_slot_of(5), Some(TextureDimension::Cube));
        assert_eq!(texture_slot_of(9), Some(TextureDimension::D2));
        assert_eq!(
            texture_slot_of(25),
            None,
            "第 25 格是加宽之后的第一格**外面**（老的那条「只能是 1/3/5/7」现在不成立，见 §75 的 E7）"
        );
        assert_eq!(TextureDimension::Cube.layers(), 6);
        assert_eq!(
            layout().too_big(),
            None,
            "32 字节远没到 {MAX_PARAMS_BYTES} 的上限"
        );
    }
}
