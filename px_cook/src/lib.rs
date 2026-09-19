//! **px_cook**：`cook` —— 图脚本里唯一那个缓存辅助函数。
//!
//! 图脚本是**普通 Rust**：
//!
//! ```ignore
//! struct ClustersInput {}                        // 图参数：包上游节点
//! struct MixedInput { a: Cooked<Field>, b: Cooked<Field>, mask: Cooked<Field> }
//!
//! let clusters = cook::<field::Fbm>(&cache, ClustersInput {}, canvas)?;
//! let mixed    = cook::<field::Mix>(&cache, MixedInput { a: clusters, b: carved, mask: weight }, canvas)?;
//! let coarse   = cook::<volume::CloudCoarse>(&cache, CoarseInput { coverage: mixed }, canvas)?;
//! ```
//!
//! 三样东西因此是**编译期**的事：输入个数与域（图参数 struct 的字段）、输出域（`PxOp::Payload`）、
//! 参数类型（`PxOp::Params`）。没有枚举分派、没有注册表。
//!
//! ⚠ 本 crate **一个算子实现都不依赖**：实现住 `px_*_op` 的 dylib 里。
//! 这条线一破，「改算子实现不重编图程序」那条性质就没了。

pub mod field_fn;

use std::time::Instant;

use serde::Serialize;
use serde::de::DeserializeOwned;

use px_field_schema::field::Field;
use px_graph_schema::Key;
use px_mesh_schema::MeshData;
use px_volume_schema::VolumeData;

pub use px_graph::{Cache, Report};
/// 宏生成出来的代码按 `$crate::px_graph_schema::…` 走 —— 于是用宏的人不必自己依赖它。
pub use px_graph_schema;
pub use px_graph_schema::{Grid, PxKeyed};

/// 算子在缓存里的**身份**。
#[derive(Debug, Clone, Copy)]
pub struct Identity {
    pub id: &'static str,
    pub version: u32,
    pub source_hash: u64,
}

/// **图参数**：包着上游节点，键由上游的键聚合而成。
///
/// 它就是图脚本里那个 struct —— 字段是具名的，漏一个、接错域都是编译错。
pub trait PxInputs: input_bytes::FromPayloads {
    fn collect(&self, hasher: &mut blake3::Hasher);
}

/// 一个已经拿到手的节点：**类型化的值 + 它的身份**。
#[derive(Clone)]
pub struct Cooked<P> {
    pub key: Key,
    value: P,
    pub hit: bool,
    pub millis: u64,
    pub bytes: usize,
}

impl<P> Cooked<P> {
    /// 类型化的值。
    pub fn value(&self) -> &P {
        &self.value
    }

    /// 只给本 crate 用：`cook` 要把刚算出来的值包进来。
    fn make(key: Key, value: P, hit: bool, millis: u64, bytes: usize) -> Self {
        Self { key, value, hit, millis, bytes }
    }
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

impl<P: PayloadKind> Cooked<P> {
    /// 换成老路径的 `Artifact`（克隆值、**字节从缓存里取回来**，不重新序列化）。
    ///
    /// ⚠ 它存在的唯一理由是两条路要混用：`node(op_id, name, &[&上游])` 收的是 `&Artifact`。
    /// **键同一个**（`Cooked.key` 就是写进 CAS 的那把）⇒ 不重算、不写第二份产物。
    pub fn to_artifact(&self) -> Result<px_graph::Artifact, String>
    where
        P: Clone,
    {
        let bytes = self.cached_bytes()?;
        Ok(px_graph::Artifact {
            key: self.key,
            payload: self.value.clone().into_payload(),
            bytes,
        })
    }

    /// 缓存里那份字节（不重新序列化）。
    pub fn cached_bytes(&self) -> Result<Vec<u8>, String> {
        px_graph::read_cached(self.key)
    }
}

impl Cooked<Field> {
    /// 取那张场（算子侧读上游用）。
    pub fn sample(&self) -> &Field {
        &self.value
    }
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

/// **一个算子**：身份 + 参数类型 + 输出域 + 怎么算。
///
/// ⚠ 它是**接线**，不是实现 —— 实现住 `px_*_op` 的 dylib 里（图脚本静态链的只是这一份 rlib）。
/// `#[derive(PxOp)]` 会把它生成出来；`clouds.rs` 里也能按需手写（见那个演示分支）。
pub trait PxOp {
    const ID: &'static str;
    const VERSION: u32;
    /// 描述符里的输入**名**（老路径的接口形状：`["coverage"]` 那种字节边界上的名字）。
    /// ⚠ 它只给 dylib 那一侧的描述符用；图侧的真实检查在 `Inputs` 上。
    const INPUTS: &'static [&'static str];
    /// 产物档：决定键里要不要掺评审相机。
    const KIND: px_graph_schema::OpKind;
    /// 算子源码（含共享依赖）的 FNV-1a —— 改实现必然重算那一格。
    ///
    /// ⚠ 它是 `const`（描述符是 `static`），且图侧（键里）与 dylib 侧（描述符）**同一个数**。
    const SOURCE_HASH: u64;

    /// 超参数的类型：从 `art/<图>/<节点名>.toml` 解出来的那一份。
    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    /// 输出域（`Field` / `VolumeData` / `MeshData`）—— 域、相机口径、编解码全从它推。
    type Payload: PayloadKind + payload::Build;
    /// **图参数**的类型：这个算子被接的那个输入 struct。
    /// ⚠ `cook` 要求它等于调用点给的那个 `I` ⇒ 接错 struct、接错算子都是编译错。
    type Inputs: PxInputs;

    /// 造一个算子实例（算子是无状态的，`new()` 就是 `Self`）。
    fn new() -> Self;
    fn params() -> params::Kind<Self::Params>;
    fn payload() -> payload::Kind<Self::Payload>;
    /// 怎么算：超参数 + **图参数**（这个图接的上游 struct）+ 画布。
    ///
    /// ⚠ 它在 `I` 上泛型：同一个算子被两张图用不同的接法接，实例各生成一份。
    fn render(&self, params: &Self::Params, inputs: &Self::Inputs, grid: Grid)
        -> Result<Self::Payload, String>;
}

/// 每个域的"怎么解上游字节 / 怎么编码自己的产物"。
pub mod payload {
    use px_protocol::art::Domain;

    use px_field_schema::field::Field;
    use px_mesh_schema::MeshData;
    use px_volume_schema::VolumeData;

    /// 有个类型参数，这样 `PxOp::payload()` 能指名自己的域。
    pub struct Kind<P>(pub std::marker::PhantomData<P>);

    pub trait Build: Sized {
        const WITH_CAMERAS: bool;
        fn encode(payload: &Self) -> Result<Vec<u8>, String>;
        /// 上游字节 → 类型化的值。场载荷不含投影，所以要 `projection` 补回去。
        fn decode(bytes: &[u8], projection: Domain, node: &str) -> Result<Self, String>;
    }

    impl Build for Field {
        const WITH_CAMERAS: bool = true;
        fn encode(payload: &Self) -> Result<Vec<u8>, String> {
            px_field_schema::payload::encode(payload).placeholder()
        }
        fn decode(bytes: &[u8], projection: Domain, node: &str) -> Result<Self, String> {
            let _ = node;
            px_field_schema::payload::decode(bytes, projection)
        }
    }

    impl Build for VolumeData {
        const WITH_CAMERAS: bool = false;
        fn encode(payload: &Self) -> Result<Vec<u8>, String> {
            px_volume_schema::payload::encode(payload).placeholder()
        }
        fn decode(bytes: &[u8], _projection: Domain, node: &str) -> Result<Self, String> {
            let _ = node;
            px_volume_schema::payload::decode(bytes)
        }
    }

    impl Build for MeshData {
        const WITH_CAMERAS: bool = true;
        fn encode(payload: &Self) -> Result<Vec<u8>, String> {
            px_mesh_schema::payload::encode(payload).placeholder()
        }
        fn decode(bytes: &[u8], _projection: Domain, node: &str) -> Result<Self, String> {
            let _ = node;
            px_mesh_schema::payload::decode(bytes)
        }
    }

    impl<P: Build> Kind<P> {
        pub fn encode(&self, payload: &P) -> Result<Vec<u8>, String> {
            P::encode(payload)
        }
        pub fn decode(&self, bytes: &[u8], projection: Domain, node: &str) -> Result<P, String> {
            P::decode(bytes, projection, node)
        }
        pub fn with_cameras(&self) -> bool {
            P::WITH_CAMERAS
        }
    }
}

/// 超参数那一侧：TOML 原文 → 规范 JSON（进键的那一份）。
pub mod params {
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    pub struct Kind<P>(pub std::marker::PhantomData<P>);

    impl<P: Serialize + DeserializeOwned + Default> Kind<P> {
        /// ⚠ 与 dylib 那边 `OpDescriptor` 用的**同一份规范化**，否则两侧键对不上。
        pub fn canonical(&self, toml_text: Option<&str>) -> Result<(P, String), String> {
            // ⚠ `None`（参数文件不存在）走 `Default`，**不是**解一个空串 ——
            //   与老路径的 `params::parse` 逐字一致，否则两侧的键对不上。
            let parsed: P = match toml_text {
                Some(text) => toml::from_str(text).map_err(|err| format!("参数解不开：{err}"))?,
                None => P::default(),
            };
            let json = px_graph_schema::canonical_params(&parsed);
            Ok((parsed, json))
        }
    }
}

/// **缓存辅助函数**：算键 → 查 → 命中就解字节；不命中就 `render` → 编码 → 落盘。
///
/// 参数来自 `art/<图>/<节点名>.toml`（按节点名取，与老路径同一个约定）；
/// 上游来自 `inputs`（图参数 struct，键由它聚合）；域由 `PxOp::Payload` 决定。
pub fn cook<O>(
    cache: &dyn Cache,
    node: &str,
    inputs: O::Inputs,
    grid: Grid,
) -> Result<Cooked<O::Payload>, String>
where
    O: PxOp,
{
    let op = O::new();
    let (params, params_json) = O::params().canonical(cache.params_text(node).as_deref())?;

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_cook/v1");
    hasher.update(O::ID.as_bytes());
    hasher.update(&O::VERSION.to_le_bytes());
    hasher.update(&O::SOURCE_HASH.to_le_bytes());
    hasher.update(&cache.graph_version().to_le_bytes());
    let (width, height) = cache.canvas();
    hasher.update(&width.to_le_bytes());
    hasher.update(&height.to_le_bytes());
    hasher.update(cache.projection().name().as_bytes());
    hasher.update(params_json.as_bytes());
    inputs.collect(&mut hasher);
    let base = *hasher.finalize().as_bytes();
    // ⚠ 相机那一档：产物里带着相机表 ⇒ 相机变了产物内容就变 ⇒ 必须进键。
    let with_cameras = O::payload().with_cameras();
    let key = if with_cameras {
        px_graph_schema::key_with_cameras(base, cache.cameras())
    } else {
        base
    };

    let projection = cache.projection();
    if let Some(bytes) = cache.fetch(key) {
        let value = O::payload().decode(&bytes, projection, node)?;
        let bytes_len = bytes.len();
        cache.store(
            Report {
                node,
                op: O::ID,
                op_version: O::VERSION,
                key,
                hit: true,
                millis: 0,
                bytes: bytes_len,
                with_cameras,
            },
            &bytes,
        )?;
        return Ok(Cooked::make(key, value, true, 0, bytes_len));
    }

    let started = Instant::now();
    let value = op.render(&params, &inputs, grid)?;
    let millis = started.elapsed().as_millis() as u64;
    let bytes = O::payload().encode(&value)?;
    let bytes_len = bytes.len();
    cache.store(
        Report {
            node,
            op: O::ID,
            op_version: O::VERSION,
            key,
            hit: false,
            millis,
            bytes: bytes_len,
            with_cameras,
        },
        &bytes,
    )?;
    Ok(Cooked::make(key, value, false, millis, bytes_len))
}

/// **收敛样板**：一个算子只需要写「身份」与「怎么算」，其余（身份常量、参数类型、
/// 输出域、编解码、相机口径）全从这两行推。
///
/// ⚠ 这是 `#[derive(PxOp)]` 的语义，先用宏落地 —— 两者生成的东西必须一致。
///
/// ```ignore
/// px_op! { Fbm = params::FBM, params::fbm::Params, Field, |p, grid| crate::ops::fbm::eval(p, &[], grid) }
/// ```
#[macro_export]
macro_rules! px_op {
    ($name:ident = $id:expr, $version:literal, $params:ty, $inputs:ty, $payload:ty,
     $kind:expr, $arity:expr, $sources:expr,
     |$p:ident, $i:ident, $g:ident| $body:expr) => {
        impl $crate::PxOp for $name {
            const ID: &'static str = $id;
            const VERSION: u32 = $version;
            const INPUTS: &'static [&'static str] = $arity;
            const KIND: $crate::px_graph_schema::OpKind = $kind;

            type Params = $params;
            type Inputs = $inputs;
            type Payload = $payload;

            /// 常数上下文可用（描述符是 `static`）；图侧与 dylib 侧同一个数。
            const SOURCE_HASH: u64 = $crate::source_hash_of(&$sources);

            fn params() -> $crate::params::Kind<$params> {
                $crate::params::Kind(std::marker::PhantomData)
            }

            fn payload() -> $crate::payload::Kind<$payload> {
                $crate::payload::Kind(std::marker::PhantomData)
            }

            fn new() -> Self {
                $name
            }

            fn render(
                &self,
                $p: &$params,
                $i: &$inputs,
                $g: $crate::Grid,
            ) -> Result<$payload, String> {
                Ok($body)
            }
        }
    };
}

/// 算子源码清单的哈希（`SOURCE_HASH` 的算法）——**const fn**，因为描述符是 `static`。
///
/// ⚠ 图侧（键里）与 dylib 侧（描述符）都用它，同一个数。
pub const fn source_hash_of(parts: &[&str]) -> u64 {
    let mut hash = px_graph_schema::FNV_OFFSET;
    let mut index = 0;
    while index < parts.len() {
        hash = px_graph_schema::identity::fnv1a_stage(hash, parts[index]);
        index += 1;
    }
    hash
}

/// **图参数的形状**：`N` 个上游。图脚本就用这几个（`Unary1<Field>` / `Unary3<Field>` …）
/// 当输入 struct —— 字段具名，漏一个、接错域都是编译错。
#[derive(Clone, Copy)]
pub struct Unary1<T> {
    pub a: T,
}

#[derive(Clone, Copy)]
pub struct Unary2<T> {
    pub a: T,
    pub b: T,
}

#[derive(Clone, Copy)]
pub struct Unary3<T> {
    pub a: T,
    pub b: T,
    pub c: T,
}

/// 每个上游把自己的键贡献进来（**图参数的键聚合**）。
impl PxInputs for () {
    fn collect(&self, _hasher: &mut blake3::Hasher) {}
}

impl<T: PayloadKind + payload::Build> PxInputs for Unary1<Cooked<T>> {
    fn collect(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&self.a.key);
    }
}

impl<T: PayloadKind + payload::Build> PxInputs for Unary2<Cooked<T>> {
    fn collect(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&self.a.key);
        hasher.update(&self.b.key);
    }
}

impl<T: PayloadKind + payload::Build> PxInputs for Unary3<Cooked<T>> {
    fn collect(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&self.a.key);
        hasher.update(&self.b.key);
        hasher.update(&self.c.key);
    }
}

// ── dylib 那一侧的管道（全部由宏生成，算子作者看不到）─────────────────────────

/// `canonical_params` 的通用实现：TOML 原文 → 键用的规范 JSON。
pub fn canonical_params<P>(toml_text: Option<&str>) -> Result<String, String>
where
    P: Serialize + DeserializeOwned + Default,
{
    let parsed: P = match toml_text {
        Some(text) => toml::from_str(text).map_err(|err| err.to_string())?,
        None => P::default(),
    };
    Ok(px_graph_schema::canonical_params(&parsed))
}

/// 把**规范参数 JSON + 上游载荷字节**渲染成产物的字节。
///
/// ⚠ 这就是 dylib `call` 的全部内容：它不认识具体算子，只认 `PxOp` 的关联类型 ——
///   上游怎么解、算子的 `render` 怎么调、产物怎么编码，全从 `O::Inputs` / `O::Payload` 推。
pub fn call_dylib<O>(params_json: &str, grid: Grid, inputs: &[&[u8]]) -> Result<Vec<u8>, String>
where
    O: PxOp,
{
    let params: O::Params = serde_json::from_str(params_json)
        .map_err(|err| format!("参数 JSON 解不开：{err}"))?;
    let typed = O::Inputs::from_payloads(inputs, grid)?;
    let value = O::payload().encode(&O::new().render(&params, &typed, grid)?)?;
    Ok(value)
}

/// `PxInputs::from_payloads` 要的那个 trait —— 与 `PxInputs` 同一份实现。
pub use input_bytes::FromPayloads;

mod input_bytes {
    use crate::{Cooked, Grid, PayloadKind, PxInputs, payload};

    /// 本 crate 内部用：把上游的**字节**解成图参数 struct。
    pub trait FromPayloads: Sized {
        fn from_payloads(inputs: &[&[u8]], grid: Grid) -> Result<Self, String>;
    }

    impl FromPayloads for () {
        fn from_payloads(inputs: &[&[u8]], _grid: Grid) -> Result<Self, String> {
            if inputs.is_empty() {
                Ok(())
            } else {
                Err(format!("这个算子不吃上游，却收到 {} 个", inputs.len()))
            }
        }
    }

    impl<T: PayloadKind + payload::Build> FromPayloads for crate::Unary1<Cooked<T>> {
        fn from_payloads(inputs: &[&[u8]], grid: Grid) -> Result<Self, String> {
            let [a] = inputs else {
                return Err(format!("这个算子吃 1 个上游，却收到 {}", inputs.len()));
            };
            Ok(Self {
                a: Cooked::from_payload(a, grid)?,
            })
        }
    }

    impl<T: PayloadKind + payload::Build> FromPayloads for crate::Unary2<Cooked<T>> {
        fn from_payloads(inputs: &[&[u8]], grid: Grid) -> Result<Self, String> {
            let [a, b] = inputs else {
                return Err(format!("这个算子吃 2 个上游，却收到 {}", inputs.len()));
            };
            Ok(Self {
                a: Cooked::from_payload(a, grid)?,
                b: Cooked::from_payload(b, grid)?,
            })
        }
    }

    impl<T: PayloadKind + payload::Build> FromPayloads for crate::Unary3<Cooked<T>> {
        fn from_payloads(inputs: &[&[u8]], grid: Grid) -> Result<Self, String> {
            let [a, b, c] = inputs else {
                return Err(format!("这个算子吃 3 个上游，却收到 {}", inputs.len()));
            };
            Ok(Self {
                a: Cooked::from_payload(a, grid)?,
                b: Cooked::from_payload(b, grid)?,
                c: Cooked::from_payload(c, grid)?,
            })
        }
    }
}

impl<P: PayloadKind + payload::Build> Cooked<P> {
    /// 从上游字节解出一个"已经拿到手"的节点（dylib 那一侧用；键不在这儿，用空键占位）。
    fn from_payload(bytes: &[u8], grid: Grid) -> Result<Self, String> {
        Ok(Self::make(
            [0; 32],
            P::decode(bytes, grid.projection, "")?,
            true,
            0,
            0,
        ))
    }
}

// ── 导出宏：dylib 那一侧的四样样板（描述符 / 规范化 / 分派 / 入口符号）──────────

/// 生成 `canonical_params`：按 `op_id` 找到算子的参数类型，把 TOML 规范化成 JSON。
#[macro_export]
macro_rules! px_canonical_params {
    ($($op:ty),+ $(,)?) => {
        extern "Rust" fn canonical_params(
            op_id: &str,
            toml_text: Option<&str>,
        ) -> Result<String, String> {
            $(
                if op_id == <$op as $crate::PxOp>::ID {
                    return $crate::canonical_params::<<$op as $crate::PxOp>::Params>(toml_text);
                }
            )+
            Err(format!("这个算子库不认识算子 {op_id}"))
        }
    };
}

/// 生成 `call`：按 `op_id` 找到算子，把**字节**渲染成**字节**。
#[macro_export]
macro_rules! px_dylib_call {
    ($($op:ty),+ $(,)?) => {
        extern "Rust" fn call(
            op_id: &str,
            params_json: &str,
            grid: $crate::Grid,
            inputs: &[&[u8]],
        ) -> Result<Vec<u8>, String> {
            $(
                if op_id == <$op as $crate::PxOp>::ID {
                    return $crate::call_dylib::<$op>(params_json, grid, inputs);
                }
            )+
            Err(format!("这个算子库不认识算子 {op_id}"))
        }
    };
}

/// 生成入口符号 `<库名>_table` —— 驱动按**文件名词干**找它（`px_graph_schema::op::table_symbol`）。
///
/// ⚠ `$lib` 必须与 crate 名（也就是产出的 dll 名）一致，否则运行期 `GetProcAddress failed`，
///   编译期毫无提示。
#[macro_export]
macro_rules! px_op_table {
    ($lib:literal, $($op:ty),+ $(,)?) => {
        #[unsafe(export_name = concat!($lib, "_table"))]
        pub extern "Rust" fn table() -> &'static $crate::px_graph_schema::OpTable {
            static OPS: &[$crate::px_graph_schema::OpDescriptor] = &[
                $($crate::px_graph_schema::OpDescriptor {
                    id: <$op as $crate::PxOp>::ID,
                    version: <$op as $crate::PxOp>::VERSION,
                    source_hash: <$op as $crate::PxOp>::SOURCE_HASH,
                    inputs: <$op as $crate::PxOp>::INPUTS,
                    kind: <$op as $crate::PxOp>::KIND,
                }),+
            ];
            static TABLE: $crate::px_graph_schema::OpTable =
                $crate::px_graph_schema::OpTable {
                    ops: OPS,
                    canonical_params: canonical_params
                        as $crate::px_graph_schema::ParamsCanonical,
                    call: call as $crate::px_graph_schema::OpCall,
                };
            &TABLE
        }
    };
}
