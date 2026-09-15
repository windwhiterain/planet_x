use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::art::Camera;

/// 场景描述的形状版本。它变了 ⇒ 旧场景产物不该再被当作同一份东西。
pub const SCENE_SCHEMA: u32 = 1;

/// CAS 里一个内容键的落盘规则。烘图侧与渲染侧共用这一份 ——
/// 两边各写一遍「键 → 路径」就是又一个「同一个键、不同内容」的入口。
pub fn cas_path(root: &Path, key: &str) -> Result<PathBuf, String> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("内容键应当是 64 位十六进制，实际是 '{key}'"));
    }
    let lower = key.to_ascii_lowercase();
    Ok(root
        .join("ab")
        .join(&lower[..2])
        .join(format!("{lower}.pxart")))
}

/// 场景对某个成员的引用：**图名 + 节点名 + 当时的键**。
/// 名字给人看与报错，键给渲染器取产物（键 = 内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub graph: String,
    pub node: String,
    pub key: String,
}

impl Member {
    pub fn new(graph: &str, node: &str, key: &str) -> Self {
        Self {
            graph: graph.to_string(),
            node: node.to_string(),
            key: key.to_string(),
        }
    }

    pub fn short_key(&self) -> &str {
        &self.key[..self.key.len().min(12)]
    }

    pub fn resolve(&self, root: &Path) -> Result<PathBuf, String> {
        let path = cas_path(root, &self.key)?;
        if !path.exists() {
            return Err(format!(
                "场景要的成员不在 CAS：{}/{}（键 {}）→ {}",
                self.graph,
                self.node,
                self.short_key(),
                path.display()
            ));
        }
        Ok(path)
    }
}

impl std::fmt::Display for Member {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}/{}@{}", self.graph, self.node, self.short_key())
    }
}

/// part 参数的一个值。JSON 里就是 `0.35` / `"rocky"` / `[0.44,0.64,0.98]` 三种写法。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Num(f64),
    Text(String),
    Triple([f32; 3]),
}

fn keys_of<T>(map: &BTreeMap<String, T>) -> String {
    if map.is_empty() {
        return "（空）".to_string();
    }
    map.keys().cloned().collect::<Vec<_>>().join(" / ")
}

/// 场景里的一个物体。格式**不认识**「行星」「云」「大气」——
/// 它只知道：叫什么、用哪个装配器、用哪个 shader 槽、引用哪些成员、带哪些参数。
/// 不认识 kind 是渲染器的事（按注册表报错），不是格式的事。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// 实体身份。重建场景时按它配对，所以同一份场景里不许重名。
    pub id: String,
    /// 装配器名（渲染器侧注册表：planet / clouds / atmosphere / …）。
    pub kind: String,
    /// shader 槽：决定用哪套材质绑定（WGSL 的 `@group/@binding` 形状）。
    /// WGSL 本体不在这里 —— 它和场、网格一样是 CAS 里的资产，
    /// 由 `members["shader"]` 引用（见 09-instruments / 06-clouds §51）。
    pub shader: String,
    /// 这个 part 引用的产物成员，按角色命名
    /// （`height` / `mesh` / `shader` / `field` / `slope_x` / …）。
    #[serde(default)]
    pub members: BTreeMap<String, Member>,
    /// 这个 part 的参数，按名字取（`radius` / `coverage` / `palette` / `tint` / …）。
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

impl Part {
    pub fn member(&self, role: &str) -> Result<&Member, String> {
        self.members.get(role).ok_or_else(|| {
            format!(
                "part '{}'（kind {}）要成员 '{role}'；这份有：{}",
                self.id,
                self.kind,
                keys_of(&self.members)
            )
        })
    }

    pub fn number(&self, key: &str) -> Result<f64, String> {
        match self.params.get(key) {
            Some(Value::Num(value)) => Ok(*value),
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要一个数，实际是 {other:?}",
                self.id
            )),
            None => Err(format!(
                "part '{}'（kind {}）缺参数 '{key}'；这份有：{}",
                self.id,
                self.kind,
                keys_of(&self.params)
            )),
        }
    }

    pub fn number_or(&self, key: &str, fallback: f64) -> f64 {
        match self.params.get(key) {
            Some(Value::Num(value)) => *value,
            _ => fallback,
        }
    }

    pub fn integer(&self, key: &str) -> Result<u32, String> {
        let value = self.number(key)?;
        if !(value.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&value)) {
            return Err(format!(
                "part '{}' 参数 '{key}' 要一个非负整数，实际是 {value}",
                self.id
            ));
        }
        Ok(value as u32)
    }

    pub fn integer_or(&self, key: &str, fallback: u32) -> u32 {
        self.integer(key).unwrap_or(fallback)
    }

    pub fn text(&self, key: &str) -> Result<&str, String> {
        match self.params.get(key) {
            Some(Value::Text(text)) => Ok(text),
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要一段文本，实际是 {other:?}",
                self.id
            )),
            None => Err(format!(
                "part '{}'（kind {}）缺参数 '{key}'；这份有：{}",
                self.id,
                self.kind,
                keys_of(&self.params)
            )),
        }
    }

    pub fn triple(&self, key: &str) -> Result<[f32; 3], String> {
        match self.params.get(key) {
            Some(Value::Triple(value)) => Ok(*value),
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要三个数，实际是 {other:?}",
                self.id
            )),
            None => Err(format!(
                "part '{}'（kind {}）缺参数 '{key}'；这份有：{}",
                self.id,
                self.kind,
                keys_of(&self.params)
            )),
        }
    }
}

/// 一般场景描述：环境光 + 相机表 + 一串物体。
/// 渲染器只吃这一份，不再从命令行接收内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneSpec {
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub ambient: f32,
    /// 评审相机表（`--sheet` 用）。空 = 走 `--cam`。
    #[serde(default)]
    pub cameras: Vec<Camera>,
    pub parts: Vec<Part>,
}

impl SceneSpec {
    pub fn part(&self, id: &str) -> Option<&Part> {
        self.parts.iter().find(|part| part.id == id)
    }

    pub fn parts_of_kind(&self, kind: &str) -> Vec<&Part> {
        self.parts.iter().filter(|part| part.kind == kind).collect()
    }

    pub fn members(&self) -> Vec<&Member> {
        self.parts
            .iter()
            .flat_map(|part| part.members.values())
            .collect()
    }

    pub fn check(&self) -> Result<(), String> {
        if self.schema != SCENE_SCHEMA {
            return Err(format!(
                "场景描述是 v{}，这份渲染器认 v{}",
                self.schema, SCENE_SCHEMA
            ));
        }
        if self.parts.is_empty() {
            return Err("场景里一个物体都没有".to_string());
        }
        let mut seen: Vec<&str> = Vec::new();
        for part in &self.parts {
            if part.id.is_empty() {
                return Err(format!("有个 part（kind {}）没给 id", part.kind));
            }
            if seen.contains(&part.id.as_str()) {
                return Err(format!("part id 重了：'{}'", part.id));
            }
            seen.push(&part.id);
            if part.shader.is_empty() {
                return Err(format!("part '{}' 没给 shader 槽", part.id));
            }
        }
        Ok(())
    }

    pub fn audit(&self) -> String {
        let mut lines = vec![format!(
            "场景 {}（v{}）｜环境光 {}｜相机 {} 个｜物体 {} 个",
            self.name,
            self.schema,
            self.ambient,
            self.cameras.len(),
            self.parts.len()
        )];
        for part in &self.parts {
            let members = if part.members.is_empty() {
                String::from("无成员")
            } else {
                part.members
                    .iter()
                    .map(|(role, member)| format!("{role}={member}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let params = if part.params.is_empty() {
                String::from("无参数")
            } else {
                part.params.keys().cloned().collect::<Vec<_>>().join(",")
            };
            lines.push(format!(
                "  [{}] kind {}｜shader 槽 {}｜{}｜参数 {}",
                part.id,
                part.kind,
                part.shader,
                members,
                params
            ));
        }
        lines.join("\n")
    }
}

pub fn read_scene(path: &Path) -> Result<SceneSpec, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读 {} 失败：{err}", path.display()))?;
    let frames =
        crate::stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    frames
        .into_iter()
        .find_map(|frame| match frame {
            crate::stream::Frame::Scene(spec) => Some(spec),
            _ => None,
        })
        .ok_or_else(|| format!("{} 里没有场景帧（Scene）", path.display()))
}

/// 写一份场景产物：清单帧（`kind = Scene`，带指纹与相机表）+ 场景帧。
/// 指纹由调用方算（协议 crate 不引哈希库，只承诺形状）。
pub fn write_scene(path: &Path, spec: &SceneSpec, fingerprint: u64) -> Result<u64, String> {
    spec.check()?;
    let bundle = crate::art::ArtBundle {
        assets: vec![crate::art::AssetManifest {
            id: spec.name.clone(),
            kind: crate::art::AssetKind::Scene,
            params: BTreeMap::from([
                ("schema".to_string(), f64::from(spec.schema)),
                ("parts".to_string(), spec.parts.len() as f64),
            ]),
            blobs: Vec::new(),
            fingerprint,
            cameras: spec.cameras.clone(),
        }],
    };
    let frames = vec![
        crate::stream::Frame::Art(bundle),
        crate::stream::Frame::Scene(spec.clone()),
    ];
    let mut bytes = Vec::new();
    crate::stream::write_stream(&mut bytes, &frames).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, &bytes).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
    Ok(bytes.len() as u64)
}
