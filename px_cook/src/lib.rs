//! **px_cook**：类型化的缓存辅助函数 —— 「普通 Rust 逻辑 + 一个 cache 函数」里的那个函数。
//!
//! 图脚本不再写 `node("field.fbm", "continents", &[])`（字符串 id + 字节边界），
//! 而是写**普通 Rust**：
//!
//! ```ignore
//! let continents = cook_field(&driver, FBM, "continents", (), grid)?;
//! let terrain = cook_field(&driver, MIX, "terrain", (&continents, &mountains, &weight), grid)?;
//! let coarse = cook_volume(&driver, CLOUD_COARSE, "coarse", &mixed, grid)?;
//! ```
//!
//! 于是三样东西变成**编译期**的事：参数类型、输入的个数与域、输出的类型。
//!
//! ⚠ 本 crate **一个算子实现都不依赖**（实现住 `px_*_op` 的 dylib 里）。
//! 这条线一破，「改算子实现不重编图程序」那条性质就没了 —— 判据见 `tests/`。

pub mod field_fn;

use std::ops::Deref;
use std::time::Instant;

use serde::Serialize;
use serde::de::DeserializeOwned;

use px_field_schema::field::Field;
use px_graph_schema::Key;
use px_mesh_schema::MeshData;
use px_volume_schema::VolumeData;

pub use px_graph::{Cache, Report};
pub use px_graph_schema::Grid;

/// 算子在缓存里的**身份**。`source_hash` 覆盖它的共享依赖（§28.2）。
#[derive(Debug, Clone, Copy)]
pub struct Identity {
    pub id: &'static str,
    pub version: u32,
    pub source_hash: u64,
}

/// 一个算子的**类型化契约**。
///
/// `Inputs<'a>` 是借用形态的上游载荷（`()` / `&'a Cooked<Field>` /
/// `(&'a Cooked<Field>, &'a Cooked<Field>, …)`）—— 于是"接错几个输入、接错哪个域"
/// 在**编译期**就判得出来，不必等装载。
pub trait Op {
    const IDENTITY: Identity;

    type Params: Serialize + DeserializeOwned + Default;
    type Inputs<'a>;
    type Payload;

    fn cook(params: &Self::Params, inputs: &Self::Inputs<'_>, grid: Grid) -> Self::Payload;
}

/// 缓存里那份字节怎么变成类型化的载荷。
///
/// `projection` 由 `Cache` 给：场载荷**不含投影**（`Field::to_blob` 只存形状），
/// 所以解码时必须把画布的投影补回去。
pub trait Blob: Sized {
    fn encode(&self) -> Result<Vec<u8>, String>;
    fn decode(bytes: &[u8], projection: px_protocol::art::Domain) -> Result<Self, String>;
}

impl Blob for Field {
    fn encode(&self) -> Result<Vec<u8>, String> {
        px_field_schema::payload::encode(self).placeholder()
    }
    fn decode(bytes: &[u8], projection: px_protocol::art::Domain) -> Result<Self, String> {
        px_field_schema::payload::decode(bytes, projection)
    }
}

impl Blob for VolumeData {
    fn encode(&self) -> Result<Vec<u8>, String> {
        px_volume_schema::payload::encode(self).placeholder()
    }
    fn decode(bytes: &[u8], _projection: px_protocol::art::Domain) -> Result<Self, String> {
        px_volume_schema::payload::decode(bytes)
    }
}

impl Blob for MeshData {
    fn encode(&self) -> Result<Vec<u8>, String> {
        px_mesh_schema::payload::encode(self).placeholder()
    }
    fn decode(bytes: &[u8], _projection: px_protocol::art::Domain) -> Result<Self, String> {
        px_mesh_schema::payload::decode(bytes)
    }
}

/// 一个已经拿到手的节点：**类型化的值 + 它的身份**。
pub struct Cooked<P> {
    pub key: Key,
    pub value: P,
    pub hit: bool,
    pub millis: u64,
    pub bytes: usize,
}

/// 各领域载荷 → 老路径那个枚举（`node()` 收 `&Artifact`，混用两条路时要过一下）。
pub trait PayloadKind {
    fn into_payload(self) -> px_graph::Payload;
}

impl PayloadKind for Field {
    fn into_payload(self) -> px_graph::Payload {
        px_graph::Payload::Field(self)
    }
}

impl PayloadKind for VolumeData {
    fn into_payload(self) -> px_graph::Payload {
        px_graph::Payload::Volume(self)
    }
}

impl PayloadKind for MeshData {
    fn into_payload(self) -> px_graph::Payload {
        px_graph::Payload::Mesh(self)
    }
}

impl<P> Deref for Cooked<P> {
    type Target = P;
    fn deref(&self) -> &P {
        &self.value
    }
}

impl<P: PayloadKind> Cooked<P> {
    /// 换成老路径的 `Artifact`：**字节从缓存里取回来**（不重新序列化）。
    ///
    /// ⚠ 它存在的唯一理由是两条路要混用：`node(op_id, name, &[&上游])` 收的是 `&Artifact`，
    /// 而类型化那一支给的是 `Cooked<T>`。**键同一个**（`Cooked.key` 就是写进 CAS 的那把），
    /// 所以这一步不会重算、也不会写出第二份产物。
    pub fn into_artifact(self) -> Result<px_graph::Artifact, String> {
        let bytes = self.cached_bytes()?;
        let payload = self.value.into_payload();
        Ok(px_graph::Artifact { key: self.key, payload, bytes })
    }

    /// 缓存里那份字节（不重新序列化）。
    pub fn cached_bytes(&self) -> Result<Vec<u8>, String> {
        px_graph::read_cached(self.key)
    }
}

impl Cooked<Field> {
    pub fn field(&self) -> &Field {
        &self.value
    }
    pub fn stats(&self) -> px_field_schema::field::Stats {
        self.value.stats()
    }
}

impl Cooked<VolumeData> {
    pub fn volume(&self) -> &VolumeData {
        &self.value
    }
}

impl Cooked<MeshData> {
    pub fn mesh(&self) -> &MeshData {
        &self.value
    }
}

/// 键 = 内容（§17.1）。⚠ 与 `node_key` 的差别只有一项：**源码哈希也进键**。
///
/// 类型化这条路里，算子的语义可能随源码变而 `version` 没升 —— 那样缓存会静默给旧产物。
/// 进了键就是「必然重算」；`version` 仍旧只用来打一句"源码变了"的告警。
/// 老路径（`node_key`）一位不动 ⇒ 老图的老键全部照旧命中。
pub fn cook_key(
    identity: &Identity,
    graph_version: u32,
    canvas: (u32, u32),
    projection: px_protocol::art::Domain,
    params_json: &str,
    input_keys: &[Key],
) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_cook/v1");
    hasher.update(identity.id.as_bytes());
    hasher.update(&identity.version.to_le_bytes());
    hasher.update(&identity.source_hash.to_le_bytes());
    hasher.update(&graph_version.to_le_bytes());
    hasher.update(&canvas.0.to_le_bytes());
    hasher.update(&canvas.1.to_le_bytes());
    hasher.update(projection.name().as_bytes());
    hasher.update(params_json.as_bytes());
    for key in input_keys {
        hasher.update(key);
    }
    *hasher.finalize().as_bytes()
}

/// 上游节点的键集合。⚠ 输入**个数**要留在类型里：数组长度就是它，
/// 于是"接错几个输入"编译期就报 —— 这正是要拿回来的一半类型检查。
pub trait InputKeys {
    fn keys(&self) -> Vec<Key>;
}

impl InputKeys for () {
    fn keys(&self) -> Vec<Key> {
        Vec::new()
    }
}

impl<T> InputKeys for &Cooked<T> {
    fn keys(&self) -> Vec<Key> {
        vec![self.key]
    }
}

impl<T, const N: usize> InputKeys for &[&Cooked<T>; N] {
    fn keys(&self) -> Vec<Key> {
        self.iter().map(|node| node.key).collect()
    }
}

/// 缓存辅助函数的**唯一一份**：算键 → 查 → 命中就解码；不命中就 cook → 编码 → 落盘。
///
/// 参数文件按**节点名**取（`art/<图>/<节点>.toml`）—— 这是老路径的同一个约定，
/// 而节点名是**图**的知识，不是算子的：同一个 `field.fbm` 在 `clouds` 里叫 `clusters`、
/// 在 `planet` 里叫 `continents`。⚠ 参数文件名**不进键**（键里是规范后的参数内容），
/// 所以给同一个算子换个参数文件不会白重算 —— 与老路径逐字同一条口径。
fn cook<O>(
    cache: &dyn Cache,
    node: &str,
    inputs: O::Inputs<'_>,
    grid: Grid,
    with_cameras: bool,
) -> Result<Cooked<O::Payload>, String>
where
    O: Op,
    O::Payload: Blob,
    for<'a> O::Inputs<'a>: InputKeys,
{
    let params: O::Params = read_params::<O>(cache, node)?;
    let params_json = px_graph_schema::canonical_params(&params);
    let base = cook_key(
        &O::IDENTITY,
        cache.graph_version(),
        cache.canvas(),
        cache.projection(),
        &params_json,
        &inputs.keys(),
    );
    // ⚠ 相机那一档：产物里**带着相机表** ⇒ 相机变了产物内容就变 ⇒ 必须进键，
    // 否则会出现「同一个键、不同内容」（§17.1）。体积不进相机（相机是「怎么看」，体积没人看）。
    let key = if with_cameras {
        px_graph_schema::key_with_cameras(base, cache.cameras())
    } else {
        base
    };

    let projection = cache.projection();
    if let Some(bytes) = cache.fetch(key) {
        let value = O::Payload::decode(&bytes, projection)?;
        let bytes_len = bytes.len();
        let report = Report {
            node,
            op: O::IDENTITY.id,
            op_version: O::IDENTITY.version,
            key,
            hit: true,
            millis: 0,
            bytes: bytes_len,
            with_cameras,
        };
        cache.store(report, &bytes)?;
        return Ok(Cooked {
            key,
            value,
            hit: true,
            millis: 0,
            bytes: bytes_len,
        });
    }

    let started = Instant::now();
    let value = O::cook(&params, &inputs, grid);
    let millis = started.elapsed().as_millis() as u64;
    let bytes = value.encode()?;
    let report = Report {
        node,
        op: O::IDENTITY.id,
        op_version: O::IDENTITY.version,
        key,
        hit: false,
        millis,
        bytes: bytes.len(),
        with_cameras,
    };
    let bytes_len = bytes.len();
    cache.store(report, &bytes)?;
    Ok(Cooked {
        key,
        value,
        hit: false,
        millis,
        bytes: bytes_len,
    })
}

/// 参数原文 → 类型化的参数（文件不存在就用默认值）。
fn read_params<O: Op>(cache: &dyn Cache, name: &str) -> Result<O::Params, String> {
    if name.is_empty() {
        return Ok(O::Params::default());
    }
    match cache.params_text(name) {
        Some(text) => toml::from_str(&text).map_err(|err| format!("{name}.toml 解不开：{err}")),
        None => Ok(O::Params::default()),
    }
}

/// 场算子的入口。
pub fn cook_field<O>(
    cache: &dyn Cache,
    node: &str,
    inputs: O::Inputs<'_>,
    grid: Grid,
) -> Result<Cooked<Field>, String>
where
    O: Op<Payload = Field>,
    for<'a> O::Inputs<'a>: InputKeys,
{
    cook::<O>(cache, node, inputs, grid, true)
}

/// 体积算子的入口（⚠ 不掺评审相机：相机是「怎么看」，体积没人看）。
pub fn cook_volume<O>(
    cache: &dyn Cache,
    node: &str,
    inputs: O::Inputs<'_>,
    grid: Grid,
) -> Result<Cooked<VolumeData>, String>
where
    O: Op<Payload = VolumeData>,
    for<'a> O::Inputs<'a>: InputKeys,
{
    cook::<O>(cache, node, inputs, grid, false)
}

/// 网格算子的入口。
pub fn cook_mesh<O>(
    cache: &dyn Cache,
    node: &str,
    inputs: O::Inputs<'_>,
    grid: Grid,
) -> Result<Cooked<MeshData>, String>
where
    O: Op<Payload = MeshData>,
    for<'a> O::Inputs<'a>: InputKeys,
{
    cook::<O>(cache, node, inputs, grid, true)
}

/// 老路径的载荷类型（图脚本要按域解出来时会用到）。
pub use px_graph::Payload as AnyPayload;
