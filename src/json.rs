//! **结构无关**的 JSON dump：把任意模型值转成 `serde_json::Value`。
//!
//! 为什么需要它：引擎的持久格式是 RON，RON 允许**非字符串的 map key**——最典型的是
//! 可控状态里的 `invest_weights` / `build_weights`，键是 `(城市名, 建筑id)` 元组
//! （[`crate::model::InvestKey`]）。JSON 的对象键**必须**是字符串，所以直接
//! `serde_json::to_value(&state)` 会失败（`key must be a string`）。
//!
//! 本模块提供一个适配器 [`to_value`]：照常序列化整棵树，只在遇到 map key 时用
//! [`KeyAsString`] 把键转成字符串——
//!
//! * 字符串键      → 原样（`"中国"`）
//! * 整数/浮点键   → `"3"` / `"0.5"`
//! * 元组/序列键   → 各元素按本规则转字符串后用 `|` 连接（`"长三角|7"`）
//! * 单元/新类型   → 内层值的字符串形式
//!
//! 于是**模型怎么改都行**：将来任何字段、任何嵌套、任何古怪的键类型，都不需要动
//! 这里一行业务代码，也不会让前端的通用 widget 需要认识新字段。这是「前端 widget
//! 必须 generic」在**传输层**的前提条件：模型必须能被整份 dump 出来，而不是被
//! 手工挑字段地投影（手工投影才会漂移）。

use serde::Serialize;
use serde::ser::{
    self, Impossible, SerializeMap, SerializeSeq, SerializeStruct, SerializeTuple, Serializer,
};
use serde_json::{Map, Value};

/// 把任意 `Serialize` 值转成 `serde_json::Value`，非字符串 map key 自动转字符串
/// （见模块文档）。除「map key 不是标量/元组」这种病态情况外不会失败。
pub fn to_value<T: Serialize + ?Sized>(value: &T) -> Result<Value, serde_json::Error> {
    value.serialize(JsonSafe)
}

/// 与 `serde_json::to_value` 同语义，但 map key 走 [`KeyAsString`]。
pub struct JsonSafe;

type Err = serde_json::Error;
type Res<T> = Result<T, Err>;

impl Serializer for JsonSafe {
    type Ok = Value;
    type Error = Err;
    type SerializeSeq = Seq;
    type SerializeTuple = Seq;
    type SerializeTupleStruct = Seq;
    type SerializeTupleVariant = TupleVariant;
    type SerializeMap = MapSer;
    type SerializeStruct = StructSer;
    type SerializeStructVariant = StructVariant;

    fn serialize_bool(self, v: bool) -> Res<Value> {
        Ok(Value::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Res<Value> {
        Ok(Value::Number(v.into()))
    }
    fn serialize_i16(self, v: i16) -> Res<Value> {
        Ok(Value::Number(v.into()))
    }
    fn serialize_i32(self, v: i32) -> Res<Value> {
        Ok(Value::Number(v.into()))
    }
    fn serialize_i64(self, v: i64) -> Res<Value> {
        Ok(Value::Number(v.into()))
    }
    fn serialize_u8(self, v: u8) -> Res<Value> {
        Ok(Value::Number(v.into()))
    }
    fn serialize_u16(self, v: u16) -> Res<Value> {
        Ok(Value::Number(v.into()))
    }
    fn serialize_u32(self, v: u32) -> Res<Value> {
        Ok(Value::Number(v.into()))
    }
    fn serialize_u64(self, v: u64) -> Res<Value> {
        Ok(Value::Number(v.into()))
    }
    fn serialize_f32(self, v: f32) -> Res<Value> {
        Ok(finite_f64(v as f64))
    }
    fn serialize_f64(self, v: f64) -> Res<Value> {
        Ok(finite_f64(v))
    }
    fn serialize_char(self, v: char) -> Res<Value> {
        Ok(Value::String(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Res<Value> {
        Ok(Value::String(v.to_string()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Res<Value> {
        Ok(Value::Array(
            v.iter().map(|b| Value::Number((*b).into())).collect(),
        ))
    }
    fn serialize_none(self) -> Res<Value> {
        Ok(Value::Null)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Res<Value> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Res<Value> {
        Ok(Value::Null)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Res<Value> {
        Ok(Value::Null)
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _idx: u32,
        variant: &'static str,
    ) -> Res<Value> {
        Ok(Value::String(variant.to_string()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Res<Value> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _idx: u32,
        variant: &'static str,
        value: &T,
    ) -> Res<Value> {
        let mut map = Map::new();
        map.insert(variant.to_string(), value.serialize(JsonSafe)?);
        Ok(Value::Object(map))
    }
    fn serialize_seq(self, len: Option<usize>) -> Res<Self::SerializeSeq> {
        Ok(Seq {
            items: Vec::with_capacity(len.unwrap_or(0)),
        })
    }
    fn serialize_tuple(self, len: usize) -> Res<Self::SerializeTuple> {
        Ok(Seq {
            items: Vec::with_capacity(len),
        })
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Res<Self::SerializeTupleStruct> {
        Ok(Seq {
            items: Vec::with_capacity(len),
        })
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _idx: u32,
        variant: &'static str,
        len: usize,
    ) -> Res<Self::SerializeTupleVariant> {
        Ok(TupleVariant {
            variant: variant.to_string(),
            items: Vec::with_capacity(len),
        })
    }
    fn serialize_map(self, _len: Option<usize>) -> Res<Self::SerializeMap> {
        Ok(MapSer {
            map: Map::new(),
            key: None,
        })
    }
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Res<Self::SerializeStruct> {
        Ok(StructSer { map: Map::new() })
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _idx: u32,
        variant: &'static str,
        _len: usize,
    ) -> Res<Self::SerializeStructVariant> {
        Ok(StructVariant {
            variant: variant.to_string(),
            map: Map::new(),
        })
    }
}

/// 小工具：非有限浮点（NaN/∞）在 JSON 里没有表示，落成 `null`（与 serde_json 一致）。
fn finite_f64(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

// --- 序列/元组 --------------------------------------------------------------

pub struct Seq {
    items: Vec<Value>,
}
impl SerializeSeq for Seq {
    type Ok = Value;
    type Error = Err;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        self.items.push(value.serialize(JsonSafe)?);
        Ok(())
    }
    fn end(self) -> Res<Value> {
        Ok(Value::Array(self.items))
    }
}
impl SerializeTuple for Seq {
    type Ok = Value;
    type Error = Err;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Res<Value> {
        SerializeSeq::end(self)
    }
}
impl ser::SerializeTupleStruct for Seq {
    type Ok = Value;
    type Error = Err;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Res<Value> {
        SerializeSeq::end(self)
    }
}

pub struct TupleVariant {
    variant: String,
    items: Vec<Value>,
}
impl ser::SerializeTupleVariant for TupleVariant {
    type Ok = Value;
    type Error = Err;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        self.items.push(value.serialize(JsonSafe)?);
        Ok(())
    }
    fn end(self) -> Res<Value> {
        let mut map = Map::new();
        map.insert(self.variant, Value::Array(self.items));
        Ok(Value::Object(map))
    }
}

// --- map / struct（这里的键走 KeyAsString） --------------------------------

pub struct MapSer {
    map: Map<String, Value>,
    key: Option<String>,
}
impl SerializeMap for MapSer {
    type Ok = Value;
    type Error = Err;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Res<()> {
        self.key = Some(key.serialize(KeyAsString)?);
        Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        let key = self.key.take().expect("serde contract: key before value");
        self.map.insert(key, value.serialize(JsonSafe)?);
        Ok(())
    }
    fn end(self) -> Res<Value> {
        Ok(Value::Object(self.map))
    }
}

pub struct StructSer {
    map: Map<String, Value>,
}
impl SerializeStruct for StructSer {
    type Ok = Value;
    type Error = Err;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Res<()> {
        self.map.insert(key.to_string(), value.serialize(JsonSafe)?);
        Ok(())
    }
    fn end(self) -> Res<Value> {
        Ok(Value::Object(self.map))
    }
}

pub struct StructVariant {
    variant: String,
    map: Map<String, Value>,
}
impl ser::SerializeStructVariant for StructVariant {
    type Ok = Value;
    type Error = Err;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Res<()> {
        self.map.insert(key.to_string(), value.serialize(JsonSafe)?);
        Ok(())
    }
    fn end(self) -> Res<Value> {
        let mut outer = Map::new();
        outer.insert(self.variant, Value::Object(self.map));
        Ok(Value::Object(outer))
    }
}

// --- map key → 字符串 -------------------------------------------------------

/// map key 的字符串化适配器（见模块文档）。
pub struct KeyAsString;

impl Serializer for KeyAsString {
    type Ok = String;
    type Error = Err;
    type SerializeSeq = KeySeq;
    type SerializeTuple = KeySeq;
    type SerializeTupleStruct = KeySeq;
    type SerializeTupleVariant = KeySeq;
    type SerializeMap = Impossible<String, Err>;
    type SerializeStruct = Impossible<String, Err>;
    type SerializeStructVariant = Impossible<String, Err>;

    fn serialize_bool(self, v: bool) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_i8(self, v: i8) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_i16(self, v: i16) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_i32(self, v: i32) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_i64(self, v: i64) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_u8(self, v: u8) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_u16(self, v: u16) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_u32(self, v: u32) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_u64(self, v: u64) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_f32(self, v: f32) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_f64(self, v: f64) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_char(self, v: char) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_str(self, v: &str) -> Res<String> {
        Ok(v.to_string())
    }
    fn serialize_bytes(self, v: &[u8]) -> Res<String> {
        Ok(String::from_utf8_lossy(v).into_owned())
    }
    fn serialize_none(self) -> Res<String> {
        Ok(String::new())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Res<String> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Res<String> {
        Ok(String::new())
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Res<String> {
        Ok(String::new())
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _idx: u32,
        variant: &'static str,
    ) -> Res<String> {
        Ok(variant.to_string())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Res<String> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _idx: u32,
        variant: &'static str,
        value: &T,
    ) -> Res<String> {
        Ok(format!("{variant}:{}", value.serialize(KeyAsString)?))
    }
    fn serialize_seq(self, len: Option<usize>) -> Res<Self::SerializeSeq> {
        Ok(KeySeq {
            parts: Vec::with_capacity(len.unwrap_or(0)),
            first: None,
        })
    }
    fn serialize_tuple(self, len: usize) -> Res<Self::SerializeTuple> {
        Ok(KeySeq {
            parts: Vec::with_capacity(len),
            first: None,
        })
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Res<Self::SerializeTupleStruct> {
        Ok(KeySeq {
            parts: Vec::with_capacity(len),
            first: None,
        })
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _idx: u32,
        variant: &'static str,
        len: usize,
    ) -> Res<Self::SerializeTupleVariant> {
        Ok(KeySeq {
            parts: Vec::with_capacity(len + 1),
            first: Some(variant.to_string()),
        })
    }
    fn serialize_map(self, _len: Option<usize>) -> Res<Self::SerializeMap> {
        Err(ser::Error::custom("map key must not itself be a map"))
    }
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Res<Self::SerializeStruct> {
        Err(ser::Error::custom("map key must not itself be a struct"))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _idx: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Res<Self::SerializeStructVariant> {
        Err(ser::Error::custom("map key must not itself be a struct"))
    }
}

/// 元组键的各元素：按同样规则转字符串，用 `|` 连接（`("长三角", 7)` → `"长三角|7"`）。
pub struct KeySeq {
    parts: Vec<String>,
    first: Option<String>,
}
impl KeySeq {
    fn finish(mut self) -> String {
        if let Some(f) = self.first {
            self.parts.insert(0, f);
        }
        self.parts.join("|")
    }
}
impl SerializeSeq for KeySeq {
    type Ok = String;
    type Error = Err;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        self.parts.push(value.serialize(KeyAsString)?);
        Ok(())
    }
    fn end(self) -> Res<String> {
        Ok(self.finish())
    }
}
impl SerializeTuple for KeySeq {
    type Ok = String;
    type Error = Err;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Res<String> {
        Ok(self.finish())
    }
}
impl ser::SerializeTupleStruct for KeySeq {
    type Ok = String;
    type Error = Err;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Res<String> {
        Ok(self.finish())
    }
}
impl ser::SerializeTupleVariant for KeySeq {
    type Ok = String;
    type Error = Err;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Res<()> {
        self.parts.push(value.serialize(KeyAsString)?);
        Ok(())
    }
    fn end(self) -> Res<String> {
        Ok(self.finish())
    }
}

#[cfg(test)]
#[path = "tests/json.rs"]
mod tests;

// --- 存档（`--save` / `--start`）：JSON 出口 + **元组键**的反向还原 ----------------
//
// 2026-10 起存档格式是 **JSON**（不再是 RON）：RON 要求结构体字段名是合法标识符，
// 而「字段名就是给人看的中文名词」是本仓的方向（见 `.agents/notes/field-naming.md`），
// 带空格/符号的名字（`和平↔军国`、`MOND 掌握度`）RON 根本写不出来。
//
// [`to_value`] 已经把**非字符串的 map key** 转成了 `"城|7"` 这种字符串（见模块文档），
// 所以反方向只要把这两个形状的键拆回来即可——JSON 里只有这两处：

use serde::Deserialize;
use std::collections::BTreeMap;

/// 把模型序列化成**给人看、可 diff** 的 JSON（键序 = 结构体声明顺序，见 `preserve_order`）。
pub fn to_string_pretty<T: Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&to_value(value)?)
}

/// 从 JSON 文本还原模型。元组键由 [`de_keys_ss`] / [`de_keys_su`] 在各字段上还原。
pub fn from_str<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(text)
}

/// `(势力, 天体)` 形状的 map 键（[`crate::model::State::depots`] 在用）。
pub fn de_keys_ss<'de, D, V>(d: D) -> Result<BTreeMap<(String, String), V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: serde::Deserialize<'de>,
{
    let raw = BTreeMap::<String, V>::deserialize(d)?;
    raw.into_iter()
        .map(|(k, v)| match k.split_once('|') {
            Some((a, b)) => Ok(((a.to_string(), b.to_string()), v)),
            None => Err(serde::de::Error::custom(format!(
                "键 `{k}` 不是 `势力|天体` 形状（存档是被手改过？）"
            ))),
        })
        .collect()
}

/// `(城, 建筑编号)` 形状的 map 键（`invest_weights` / `build_weights` 在用）。
pub fn de_keys_su<'de, D, V>(d: D) -> Result<BTreeMap<(String, u32), V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: serde::Deserialize<'de>,
{
    let raw = BTreeMap::<String, V>::deserialize(d)?;
    raw.into_iter()
        .map(|(k, v)| match k.split_once('|') {
            Some((a, b)) => b
                .parse::<u32>()
                .map(|n| ((a.to_string(), n), v))
                .map_err(|_| serde::de::Error::custom(format!("键 `{k}` 的建筑编号不是整数"))),
            None => Err(serde::de::Error::custom(format!(
                "键 `{k}` 不是 `城|建筑编号` 形状（存档是被手改过？）"
            ))),
        })
        .collect()
}
