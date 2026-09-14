use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::wire::{Blob, BlobHeader, WireError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Field2D,
    OctahedralField,
    CubeField,
    CubeMap,
    Mesh,
    Instances,
}

fn sign(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

pub fn octahedral_direction(u: f32, v: f32) -> [f32; 3] {
    let x = u.clamp(0.0, 1.0) * 2.0 - 1.0;
    let y = v.clamp(0.0, 1.0) * 2.0 - 1.0;
    let mut direction = [x, y, 1.0 - x.abs() - y.abs()];
    if direction[2] < 0.0 {
        let (old_x, old_y) = (direction[0], direction[1]);
        direction[0] = (1.0 - old_y.abs()) * sign(old_x);
        direction[1] = (1.0 - old_x.abs()) * sign(old_y);
    }
    let length =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
            .sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [
        direction[0] / length,
        direction[1] / length,
        direction[2] / length,
    ]
}

pub fn octahedral_direction_y_up(u: f32, v: f32) -> [f32; 3] {
    let direction = octahedral_direction(u, v);
    [direction[0], direction[2], -direction[1]]
}

pub fn octahedral_uv(direction: [f32; 3]) -> [f32; 2] {
    let length = direction[0].abs() + direction[1].abs() + direction[2].abs();
    if length <= f32::EPSILON {
        return [0.5, 0.5];
    }
    let mut x = direction[0] / length;
    let mut y = direction[1] / length;
    if direction[2] < 0.0 {
        let old_x = x;
        x = (1.0 - y.abs()) * sign(old_x);
        y = (1.0 - old_x.abs()) * sign(y);
    }
    [x * 0.5 + 0.5, y * 0.5 + 0.5]
}

pub fn octahedral_uv_y_up(direction: [f32; 3]) -> [f32; 2] {
    octahedral_uv([direction[0], -direction[2], direction[1]])
}

pub const MESH_ATTRIBUTES: [&str; 4] = ["positions", "normals", "uvs", "indices"];
pub const MESH_POSITION: usize = 0;
pub const MESH_NORMAL: usize = 1;
pub const MESH_UV: usize = 2;
pub const MESH_INDEX: usize = 3;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeshData {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn vertices(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn blobs(&self) -> Vec<Blob> {
        vec![
            Blob::from_f32(vec![self.vertices() as u32, 3], &self.positions),
            Blob::from_f32(vec![self.vertices() as u32, 3], &self.normals),
            Blob::from_f32(vec![self.vertices() as u32, 2], &self.uvs),
            Blob::from_u32(vec![self.indices.len() as u32], &self.indices),
        ]
    }

    pub fn from_blobs(blobs: &[&Blob]) -> Result<Self, WireError> {
        if blobs.len() < MESH_ATTRIBUTES.len() {
            return Err(WireError::TruncatedFrame);
        }
        let mesh = Self {
            positions: blobs[MESH_POSITION].f32s()?,
            normals: blobs[MESH_NORMAL].f32s()?,
            uvs: blobs[MESH_UV].f32s()?,
            indices: blobs[MESH_INDEX].u32s()?,
        };
        let vertices = mesh.vertices();
        if mesh.normals.len() != vertices * 3 || mesh.uvs.len() != vertices * 2 {
            return Err(WireError::TruncatedFrame);
        }
        Ok(mesh)
    }
}

/// 一台评审相机。
///
/// `direction` 是**局部坐标系**里的方向（行星还没被 `SYSTEM_TILT` 转过去的那个系），
/// `distance` 以行星半径 1 为单位。渲染器负责套上自己的倾斜
/// （`px_render::planet::camera_for`），所以这里**不含**任何渲染器常数。
///
/// 它住在 `.pxart` 里 ⇒「这个产物该怎么看」跟产物一起走，
/// 不再散在 `tools/probe.ps1` 的 yaw/pitch 与那个 `$tilt = 0.34` 里。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub direction: [f32; 3],
    pub distance: f32,
    #[serde(default)]
    pub tag: String,
}

impl Camera {
    /// 方向归一化、距离保底。0 长度方向会让渲染器把相机放在原点。
    pub fn new(direction: [f32; 3], distance: f32, tag: impl Into<String>) -> Self {
        Self::raw(direction, distance, tag).normalized()
    }

    pub fn raw(direction: [f32; 3], distance: f32, tag: impl Into<String>) -> Self {
        Self { direction, distance, tag: tag.into() }
    }

    pub fn normalized(mut self) -> Self {
        let length = (self.direction[0] * self.direction[0]
            + self.direction[1] * self.direction[1]
            + self.direction[2] * self.direction[2])
            .sqrt();
        self.direction = if length > f32::EPSILON {
            [self.direction[0] / length, self.direction[1] / length, self.direction[2] / length]
        } else {
            [0.0, 0.0, 1.0]
        };
        self.distance = self.distance.max(1e-3);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetManifest {
    pub id: String,
    pub kind: AssetKind,
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<BlobHeader>,
    /// 载荷内容的 FNV-1a 指纹（0 = 没记过，旧产物）。
    /// diff 靠它分辨「参数一样、值不一样」—— CAS 路径能分辨，同名覆盖分辨不了。
    #[serde(default)]
    pub fingerprint: u64,
    /// 评审相机表。空 = 这个产物没带看法，渲染器走 `--cam`。
    #[serde(default)]
    pub cameras: Vec<Camera>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ArtBundle {
    pub assets: Vec<AssetManifest>,
}

pub const CUBE_FACES: u32 = 6;
pub const CUBE_COLUMNS: u32 = 3;
pub const CUBE_GUTTER: u32 = 2;

pub fn cube_cell_size(width: u32) -> u32 {
    width / CUBE_COLUMNS
}

pub fn cube_face_size(width: u32) -> u32 {
    cube_cell_size(width).saturating_sub(CUBE_GUTTER * 2)
}

pub fn cube_direction(face: u32, s: f32, t: f32) -> [f32; 3] {
    let a = s * 2.0 - 1.0;
    let b = t * 2.0 - 1.0;
    let direction = match face % CUBE_FACES {
        0 => [1.0, -b, -a],
        1 => [-1.0, -b, a],
        2 => [a, 1.0, b],
        3 => [a, -1.0, -b],
        4 => [a, -b, 1.0],
        _ => [-a, -b, -1.0],
    };
    let length =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
            .sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [
        direction[0] / length,
        direction[1] / length,
        direction[2] / length,
    ]
}

pub fn cube_face_of(direction: [f32; 3]) -> (u32, f32, f32) {
    let (x, y, z) = (direction[0], direction[1], direction[2]);
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let face = if ax >= ay && ax >= az {
        if x > 0.0 { 0 } else { 1 }
    } else if ay >= az {
        if y > 0.0 { 2 } else { 3 }
    } else if z > 0.0 {
        4
    } else {
        5
    };
    let major = match face {
        0 | 1 => ax,
        2 | 3 => ay,
        _ => az,
    }
    .max(f32::EPSILON);
    let (a, b) = match face {
        0 => (-z, -y),
        1 => (z, -y),
        2 => (x, z),
        3 => (x, -z),
        4 => (x, -y),
        _ => (-x, -y),
    };
    let s = (a / major) * 0.5 + 0.5;
    let t = (b / major) * 0.5 + 0.5;
    (face, s.clamp(0.0, 1.0), t.clamp(0.0, 1.0))
}

pub fn cube_atlas_uv(face: u32, s: f32, t: f32, face_size: u32, gutter: u32) -> [f32; 2] {
    let cell = face_size + gutter * 2;
    let column = face % CUBE_COLUMNS;
    let row = face / CUBE_COLUMNS;
    let x = (column * cell) as f32 + gutter as f32 + s * face_size as f32;
    let y = (row * cell) as f32 + gutter as f32 + t * face_size as f32;
    let width = (cell * CUBE_COLUMNS) as f32;
    let height = (cell * 2) as f32;
    [x / width, y / height]
}


pub fn cube_map_extent(face_size: u32) -> (u32, u32) {
    let face = face_size.max(1);
    (face, face * CUBE_FACES)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Domain {
    Equirect,
    Octahedral,
    Cube,
    CubeMap,
}

impl Domain {
    pub fn name(self) -> &'static str {
        match self {
            Self::Equirect => "equirect",
            Self::Octahedral => "octahedral",
            Self::Cube => "cube",
            Self::CubeMap => "cubemap",
        }
    }
}

pub fn direction_at(
    domain: Domain,
    width: u32,
    height: u32,
    x: u32,
    y: u32,
) -> [f32; 3] {
    let u = (x as f32 + 0.5) / width.max(1) as f32;
    let v = (y as f32 + 0.5) / height.max(1) as f32;
    match domain {
        Domain::Equirect => {
            let theta = v.clamp(0.0, 1.0) * std::f32::consts::PI;
            let phi = u * std::f32::consts::TAU;
            let ring = theta.sin();
            [ring * phi.cos(), theta.cos(), ring * phi.sin()]
        }
        Domain::Octahedral => octahedral_direction_y_up(u, v),
        Domain::Cube => {
            let cell = cube_cell_size(width).max(1);
            let face_size = cube_face_size(width).max(1);
            let gutter = CUBE_GUTTER;
            let face = (y / cell) * CUBE_COLUMNS + (x / cell);
            let s = (x % cell) as f32 + 0.5 - gutter as f32;
            let t = (y % cell) as f32 + 0.5 - gutter as f32;
            cube_direction(face, s / face_size as f32, t / face_size as f32)
        }
        Domain::CubeMap => {
            let face_size = width.max(1);
            let face = (y / face_size).min(CUBE_FACES - 1);
            let s = (x as f32 + 0.5) / face_size as f32;
            let t = ((y % face_size) as f32 + 0.5) / face_size as f32;
            cube_direction(face, s, t)
        }
    }
}

pub fn uv_of(domain: Domain, direction: [f32; 3], width: u32, _height: u32) -> [f32; 2] {
    match domain {
        Domain::Equirect => {
            let v = direction[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
            let u = (direction[2].atan2(direction[0]) / std::f32::consts::TAU).rem_euclid(1.0);
            [u, v]
        }
        Domain::Octahedral => octahedral_uv_y_up(direction),
        Domain::Cube => {
            let (face, s, t) = cube_face_of(direction);
            cube_atlas_uv(
                face,
                s,
                t,
                cube_face_size(width),
                CUBE_GUTTER,
            )
        }
        Domain::CubeMap => {
            let (face, s, t) = cube_face_of(direction);
            [s, (face as f32 + t) / CUBE_FACES as f32]
        }
    }
}

// ---------------------------------------------------------------------------
// 产物 diff
//
// 渲染器拿到新的一批 `.pxart` 时，不该无脑把整场景重建一遍。它先问
// 「跟上次那份比，到底哪个节点的输出变了」—— 这就是这里回答的问题。
//
// 三档判据，从硬到软：指纹（内容）> 参数 > 载荷头（形状/类型）。
// 有了指纹，跨路径（新烘的产物换了 CAS 路径）也能分辨「值真的变了」。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum AssetChange {
    Added,
    Removed,
    Kind {
        before: AssetKind,
        after: AssetKind,
    },
    /// 载荷指纹不同 ⇒ 同样的参数烘出了不一样的值（最硬的一档）。
    Content,
    /// 哪些参数变了（含新增/删除）。
    Params {
        keys: Vec<String>,
    },
    /// 载荷的形状/类型变了（blob 头逐项对比）。
    Shape {
        before: Vec<BlobHeader>,
        after: Vec<BlobHeader>,
    },
}

impl AssetChange {
    pub fn describe(&self) -> String {
        match self {
            Self::Added => "新增".to_string(),
            Self::Removed => "移除".to_string(),
            Self::Kind { before, after } => format!("类型 {before:?}→{after:?}"),
            Self::Content => "内容变了".to_string(),
            Self::Params { keys } => format!("参数变了（{}）", keys.join(",")),
            Self::Shape { before, after } => {
                format!("载荷 {}→{}", shape_text(before), shape_text(after))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetDiff {
    pub id: String,
    pub change: AssetChange,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    pub changed: Vec<AssetDiff>,
    pub unchanged: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl Diff {
    pub fn is_identical(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }

    /// 变了的节点名（渲染器用它决定「哪些子资源要重做」）。
    pub fn touched(&self) -> Vec<&str> {
        self.changed
            .iter()
            .map(|entry| entry.id.as_str())
            .chain(self.added.iter().map(String::as_str))
            .chain(self.removed.iter().map(String::as_str))
            .collect()
    }

    /// 一行能给人/agent 看的摘要。
    pub fn summary(&self) -> String {
        if self.is_identical() {
            return format!("零变化（{} 个产物逐项相同）", self.unchanged.len());
        }
        let mut parts: Vec<String> = self
            .changed
            .iter()
            .map(|entry| format!("{}：{}", entry.id, entry.change.describe()))
            .collect();
        parts.extend(self.added.iter().map(|id| format!("{id}：新增")));
        parts.extend(self.removed.iter().map(|id| format!("{id}：移除")));
        format!(
            "{} 个产物有变化（{} 个未动）：{}",
            self.changed.len() + self.added.len() + self.removed.len(),
            self.unchanged.len(),
            parts.join(" / ")
        )
    }
}

fn shape_text(headers: &[BlobHeader]) -> String {
    if headers.is_empty() {
        return "空".to_string();
    }
    headers
        .iter()
        .map(|header| format!("{:?}{:?}", header.dtype, header.shape))
        .collect::<Vec<_>>()
        .join("×")
}

/// 按 `id`（节点名）配对两份清单，逐项给出最硬的那条差异。
///
/// ⚠️ 配对靠节点名。图里的节点名一改，这份 diff 只会说「一个新增一个移除」——
/// 这是诚实的：名字变了就不再是同一样东西。
pub fn diff(before: &ArtBundle, after: &ArtBundle) -> Diff {
    let mut out = Diff::default();
    for asset in &before.assets {
        let Some(other) = after.assets.iter().find(|entry| entry.id == asset.id) else {
            out.removed.push(asset.id.clone());
            continue;
        };
        match classify(asset, other) {
            Some(change) => out.changed.push(AssetDiff {
                id: asset.id.clone(),
                change,
            }),
            None => out.unchanged.push(asset.id.clone()),
        }
    }
    for asset in &after.assets {
        if !before.assets.iter().any(|entry| entry.id == asset.id) {
            out.added.push(asset.id.clone());
        }
    }
    out
}

fn classify(before: &AssetManifest, after: &AssetManifest) -> Option<AssetChange> {
    if before.kind != after.kind {
        return Some(AssetChange::Kind {
            before: before.kind,
            after: after.kind,
        });
    }
    if before.fingerprint != 0 && after.fingerprint != 0 && before.fingerprint != after.fingerprint {
        return Some(AssetChange::Content);
    }
    let keys: Vec<String> = before
        .params
        .keys()
        .chain(after.params.keys())
        .filter(|key| before.params.get(*key) != after.params.get(*key))
        .cloned()
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();
    if !keys.is_empty() {
        return Some(AssetChange::Params { keys });
    }
    if before.blobs != after.blobs {
        return Some(AssetChange::Shape {
            before: before.blobs.clone(),
            after: after.blobs.clone(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::DType;

    fn manifest(id: &str, fingerprint: u64, relief: f64, shape: [u32; 2]) -> AssetManifest {
        let mut params = BTreeMap::new();
        params.insert("relief".to_string(), relief);
        AssetManifest {
            id: id.to_string(),
            kind: AssetKind::Field2D,
            params,
            blobs: vec![BlobHeader {
                dtype: DType::F32,
                shape: shape.to_vec(),
            }],
            fingerprint,
            cameras: Vec::new(),
        }
    }

    fn bundle(assets: Vec<AssetManifest>) -> ArtBundle {
        ArtBundle { assets }
    }

    #[test]
    fn identical_bundles_are_identical() {
        let one = bundle(vec![manifest("height", 7, 0.3, [4, 4])]);
        let report = diff(&one, &one.clone());
        assert!(report.is_identical());
        assert_eq!(report.unchanged, vec!["height".to_string()]);
        assert!(report.summary().contains("零变化"));
    }

    #[test]
    fn a_changed_fingerprint_beats_equal_params() {
        // 参数逐项相同、只有值变了 —— 正是「换了 CAS 路径 / 同名覆盖」那一档
        let before = bundle(vec![manifest("height", 7, 0.3, [4, 4])]);
        let after = bundle(vec![manifest("height", 8, 0.3, [4, 4])]);
        let report = diff(&before, &after);
        assert_eq!(report.changed.len(), 1);
        assert_eq!(report.changed[0].change, AssetChange::Content);
    }

    #[test]
    fn a_param_change_is_reported_key_by_key() {
        let before = bundle(vec![manifest("height", 0, 0.3, [4, 4])]);
        let after = bundle(vec![manifest("height", 0, 0.4, [4, 4])]);
        let report = diff(&before, &after);
        assert_eq!(
            report.changed[0].change,
            AssetChange::Params {
                keys: vec!["relief".to_string()]
            }
        );
        assert!(report.summary().contains("relief"));
    }

    #[test]
    fn added_and_removed_are_not_confused_with_changes() {
        let before = bundle(vec![manifest("height", 1, 0.3, [4, 4])]);
        let after = bundle(vec![manifest("clouds", 1, 0.3, [4, 4])]);
        let report = diff(&before, &after);
        assert_eq!(report.added, vec!["clouds".to_string()]);
        assert_eq!(report.removed, vec!["height".to_string()]);
        assert!(!report.is_identical());
        assert_eq!(report.touched().len(), 2);
    }

    #[test]
    fn a_camera_change_is_not_a_payload_change() {
        let mut before = manifest("height", 7, 0.3, [4, 4]);
        let mut after = before.clone();
        before.cameras = vec![Camera::new([0.0, 0.0, 1.0], 3.15, "front")];
        after.cameras = vec![Camera::new([1.0, 1.0, 1.0], 1.4, "corner")];
        assert!(
            diff(&bundle(vec![before]), &bundle(vec![after])).is_identical(),
            "相机表属于「怎么看」不属于「是什么」：值没变就不该算变化"
        );
    }

    #[test]
    fn the_manifest_frame_is_readable_from_a_prefix() {
        let bundle = bundle(vec![manifest("height", 7, 0.3, [4, 4])]);
        let mut bytes = Vec::new();
        crate::stream::write_stream(
            &mut bytes,
            &[
                crate::stream::Frame::Art(bundle.clone()),
                crate::stream::Frame::Blob(Blob::from_f32(vec![2, 2], &[0.0, 1.0, 2.0, 3.0])),
            ],
        )
        .unwrap();

        // 前缀恰好停在清单帧末尾（载荷一个字节都没读）也必须解得出来：
        // 服务端就是靠这个先把「变没变」问清楚的。帧头 = magic 4 + 版本 4 + 长度 4。
        let length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let exact = 12 + length;
        assert!(exact < bytes.len(), "载荷应当还在后面");
        assert_eq!(bundle_from_prefix(&bytes[..exact]), Some(bundle.clone()));
        assert_eq!(bundle_from_prefix(&bytes), Some(bundle));
    }

    #[test]
    fn a_stream_that_does_not_start_with_a_manifest_falls_back() {
        let mut bytes = Vec::new();
        crate::stream::write_stream(
            &mut bytes,
            &[crate::stream::Frame::Blob(Blob::from_f32(vec![1], &[1.0]))],
        )
        .unwrap();
        assert_eq!(bundle_from_prefix(&bytes), None, "第一个帧不是清单 ⇒ 交给整读兜底");
        assert_eq!(bundle_from_prefix(&bytes[..6]), None, "前缀太短也不许 panic");
    }

    #[test]
    fn a_camera_normalizes_its_direction_and_keeps_a_distance_floor() {
        let camera = Camera::new([0.0, 0.0, 2.0], 0.0, "front");
        assert!((camera.direction[2] - 1.0).abs() < 1e-6);
        assert!(camera.distance >= 1e-3);
        let degenerate = Camera::new([0.0, 0.0, 0.0], 3.0, "degenerate");
        assert_eq!(degenerate.direction, [0.0, 0.0, 1.0]);
    }
}

/// 从 `.pxart` / `.pxstream` 里取出第一份产物清单。
pub fn read_bundle(path: &std::path::Path) -> Result<ArtBundle, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let frames = crate::stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    bundle_of(&frames)
        .cloned()
        .ok_or_else(|| format!("{} 里没有 Art 帧", path.display()))
}

/// 读清单要预读多少字节。清单是 JSON（载荷指纹 + 相机表 + blob 头），几 KB 足够；
/// 给到 64 KB 是为了容下相机表以后长胖。
pub const MANIFEST_PREFIX: usize = 64 * 1024;

/// 只读**清单帧**，不碰载荷。
///
/// 服务端每次请求都要先问「这份产物跟上次那份是不是同一份」（缓存键 = 路径 + 载荷指纹），
/// 而清单帧**写在流的最前面**（`px_ops::write_artifact` 如此）⇒ 读一个前缀就够，
/// 不必把 8 MB 的场整个读进来。前缀里没解出清单帧（文件不是那么写的）⇒ 回落到整读。
pub fn read_manifest(path: &std::path::Path) -> Result<ArtBundle, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let mut prefix = vec![0_u8; MANIFEST_PREFIX];
    let mut filled = 0;
    while filled < prefix.len() {
        match std::io::Read::read(&mut file, &mut prefix[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(err) => return Err(format!("读不到 {}：{err}", path.display())),
        }
    }
    prefix.truncate(filled);
    match bundle_from_prefix(&prefix) {
        Some(bundle) => Ok(bundle),
        None => read_bundle(path),
    }
}

/// 前缀里的**第一个**帧就是清单帧时返回它，否则 `None`（说明这份流不是那么排的，交给整读）。
fn bundle_from_prefix(prefix: &[u8]) -> Option<ArtBundle> {
    let rest = prefix.strip_prefix(&crate::stream::MAGIC[..])?;
    let version = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?);
    if version != crate::stream::STREAM_VERSION {
        return None;
    }
    let length = u32::from_le_bytes(rest.get(4..8)?.try_into().ok()?) as usize;
    let payload = rest.get(8..8 + length)?;
    match crate::stream::Frame::decode(payload) {
        Ok(crate::stream::Frame::Art(bundle)) => Some(bundle),
        _ => None,
    }
}

/// 一组帧里的第一份清单。
pub fn bundle_of(frames: &[crate::stream::Frame]) -> Option<&ArtBundle> {
    frames.iter().find_map(|frame| match frame {
        crate::stream::Frame::Art(bundle) => Some(bundle),
        _ => None,
    })
}

/// 一份清单里声明的评审相机（第一份带相机表的产物说了算）。
pub fn cameras_of(bundle: &ArtBundle) -> &[Camera] {
    bundle
        .assets
        .iter()
        .find(|asset| !asset.cameras.is_empty())
        .map(|asset| asset.cameras.as_slice())
        .unwrap_or(&[])
}
