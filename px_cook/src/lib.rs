//! **px_cook**：`cook` —— 图脚本里唯一那个缓存辅助函数。
//!
//! 图脚本是**普通 Rust**：
//!
//! ```ignore
//! let clusters = cook::<field::Fbm>(&cache, "clusters", (), canvas)?;
//! let mixed    = cook::<field::Mix>(&cache, "mixed",
//!                    field::MixInput { a: clusters, b: carved, mask: weight }, canvas)?;
//! let coarse   = cook::<volume::CloudCoarse>(&cache, "coarse",
//!                    volume::CloudCoarseInput { coverage: mixed.clone() }, canvas)?;
//! ```
//!
//! ⚠ **输入 struct 由算子自己定义**（住在 `px_*_op/src/typed.rs`），字段名就是它吃的东西的
//! 名字（`MixInput { a, b, mask }`、`WarpInput { field, offset }`）。于是图侧写错一个字段、
//! 少给一个上游，都是**编译错**。`px_cook` 只提供那个"不吃上游"的 `()`。
//!
//! 三样东西因此是**编译期**的事：输入个数与域（输入 struct 的字段）、输出域（`PxOp::Payload`）、
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
/// ⚠ `PxInputs::collect` 的签名里就是 `blake3::Hasher` —— 实现者（算子侧）得拿到它，
/// 所以从这里 re-export，别让人为一个签名去加依赖。
pub use px_graph_schema::identity::blake3;

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
    /// **源码指纹**（`build.rs` 算的十六进制）—— 改实现必然重算那一格。
    ///
    /// ⚠ 由 `build.rs` 生成、`env!("PX_SOURCE_HASH")` 取用：**没有手维护的清单**。
    ///   它算的是"这个 crate 编译进去的全部源码"（自己 `src/` + 所有 path 依赖的 `src/`）。
    const SOURCE_HASH: &'static str;

    /// **接口形状哈希** —— 取代手写的 `version`，见 [`interface_hash`]。
    ///
    /// 默认实现从三个类型名推（`Params` / `Inputs` / `Payload`）⇒ 改了接口自动变。
    /// 想强制失效就在 `px_op!` 的接口那一栏写一个字面量（逃生门）。
    fn interface() -> u64 {
        // ⚠ **不缓存**：泛型函数里的 `static` 在这里**不按单态化分开**（实测：
        //   `cached_interface::<A>` 与 `::<B>` 拿到同一个值），缓存反而制造 bug。
        //   代价是每次 `cook` 多哈希三个类型名 —— 可以忽略。
        // ⚠ 类型名在字段改名/增删时会变 —— 那正是我们要的信号。
        crate::interface_hash(&[
            ::core::any::type_name::<Self::Params>(),
            ::core::any::type_name::<Self::Inputs>(),
            ::core::any::type_name::<Self::Payload>(),
        ])
    }

    /// 超参数的类型：从 `art/<图>/<节点名>.toml` 解出来的那一份。
    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    /// 输出域（`Field` / `VolumeData` / `MeshData`）—— 域、相机口径、编解码全从它推。
    type Payload: payload::Build;
    /// **图参数**的类型：这个算子被接的那个输入 struct。
    /// ⚠ `cook` 要求它等于调用点给的那个 `I` ⇒ 接错 struct、接错算子都是编译错。
    type Inputs: PxInputs;

    /// 造一个算子实例（算子是无状态的，`new()` 就是 `Self`）。
    fn new() -> Self;
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

    impl<P: Build> Kind<P> {
        /// 键里要不要掺画布 —— 见 `Build::RESOLUTION_IS_CANVAS`。
        pub fn resolution_is_canvas(&self) -> bool {
            P::RESOLUTION_IS_CANVAS
        }
    }

    pub trait Build: Sized {
        /// **产物里带不带评审相机表？** 场/网格带，体积不带。
        ///
        /// ⚠ 就是这一条 —— 不再有并行的 `OpKind`。域**就是**这个类型：
        ///   `O::Payload = VolumeData` 已经说明了一切，再声明一次只会多一个能写错的地方。
        const WITH_CAMERAS: bool;
        /// **这个域的产物尺寸是不是就是画布？**
        ///
        /// * `true`（场）：分辨率 = 画布 ⇒ 画布必须进键，否则"改了画布却命中旧分辨率"。
        /// * `false`（体积/网格）：自己的分辨率由参数给 ⇒ 画布与产物无关，
        ///   掺进去只会让"改画布"连带重烘它们。
        const RESOLUTION_IS_CANVAS: bool;
        /// 清单里那三个读数（`min` / `max` / `mean`）。
        ///
        /// ⚠ 从前这件事由驱动**解字节**去猜（那要求驱动同时认识三个域）；
        ///   类型化之后值就在手里，不必解。
        fn stats(payload: &Self) -> (f64, f64, f64);
        fn encode(payload: &Self) -> Result<Vec<u8>, String>;
        /// 上游字节 → 类型化的值。场载荷不含投影，所以要 `projection` 补回去。
        fn decode(bytes: &[u8], projection: Domain, node: &str) -> Result<Self, String>;
    }

    impl Build for Field {
        const WITH_CAMERAS: bool = true;
        const RESOLUTION_IS_CANVAS: bool = true;
        fn stats(payload: &Self) -> (f64, f64, f64) {
            let stats = payload.stats();
            (stats.min as f64, stats.max as f64, stats.mean as f64)
        }
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
        const RESOLUTION_IS_CANVAS: bool = false;
        fn stats(payload: &Self) -> (f64, f64, f64) {
            // ⚠ 逐元素 + 用 f64 累加：与老路径 `volume_stats` 逐字一致。
            let (mut min, mut max, mut sum) = (f32::INFINITY, f32::NEG_INFINITY, 0.0_f64);
            for value in &payload.data {
                min = min.min(*value);
                max = max.max(*value);
                sum += *value as f64;
            }
            let mean = if payload.data.is_empty() {
                0.0
            } else {
                sum / payload.data.len() as f64
            };
            (min as f64, max as f64, mean)
        }
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
        const RESOLUTION_IS_CANVAS: bool = false;
        fn stats(payload: &Self) -> (f64, f64, f64) {
            // 与老路径同一口径：`min` = 顶点数、`max` = 三角形数、`mean` = 0。
            (payload.vertices() as f64, payload.triangles() as f64, 0.0)
        }
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

    }
}

/// 超参数那一侧：TOML 原文 → `(解析后的参数, 进键的规范 JSON)`。
///
/// ⚠ 缺文件（`None`）走 `Default`，**不是**解一个空串。
pub fn canonical_params<P>(toml_text: Option<&str>) -> Result<(P, String), String>
where
    P: serde::Serialize + serde::de::DeserializeOwned + Default,
{
    let parsed: P = match toml_text {
        Some(text) => toml::from_str(text).map_err(|err| format!("参数解不开：{err}"))?,
        None => P::default(),
    };
    let json = px_graph_schema::canonical_params(&parsed);
    Ok((parsed, json))
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
) -> Result<Cooked<O::Payload>, String>
where
    O: PxOp,
{
    // ⚠ 画布从 `cache` 取，**不是参数**：它本来就住在驱动里（`GraphSpec`），
    //   而键里那一份与算子 `render` 手里那一份必须是同一个值。
    //   从前它是参数 ⇒ 两处可以不一致，而那种不一致没有任何检查看得见。
    let grid = cache.grid();
    let op = O::new();
    let toml_text = cache.params_text(node);
    let from_file = toml_text.is_some();
    let (params, params_json) = canonical_params::<O::Params>(toml_text.as_deref())?;
    // ⚠ 缺文件是静默用默认值的 —— 这一条记录让"我少写了什么/写错了哪个字段名"跑完就看得见。
    cache.record_params(node, O::ID, &params_json, from_file);

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_cook/v1");
    hasher.update(O::ID.as_bytes());
    hasher.update(&O::interface().to_le_bytes());
    hasher.update(O::SOURCE_HASH.as_bytes());
    // ⚠ **画布按域决定要不要进键**：场的分辨率就是画布，体积/网格不是。
    //   一刀切（都掺）会让"改画布"连带重烘体积；一刀切（都不掺）会让场出现
    //   "同一个键、不同分辨率"。
    if <O::Payload as payload::Build>::RESOLUTION_IS_CANVAS {
        hasher.update(&grid.width.to_le_bytes());
        hasher.update(&grid.height.to_le_bytes());
        hasher.update(grid.projection.name().as_bytes());
    }
    hasher.update(params_json.as_bytes());
    inputs.collect(&mut hasher);
    let base = *hasher.finalize().as_bytes();
    // ⚠ 相机那一档：产物里带着相机表 ⇒ 相机变了产物内容就变 ⇒ 必须进键。
    // ⚠ 相机口径从**载荷类型**推：域就是这个类型，没有第二处声明。
    let with_cameras = <O::Payload as payload::Build>::WITH_CAMERAS;
    let key = if with_cameras {
        px_graph_schema::key_with_cameras(base, cache.cameras())
    } else {
        base
    };

    if let Some(bytes) = cache.fetch(key) {
        let value = <O::Payload as payload::Build>::decode(&bytes, grid.projection, node)?;
        let bytes_len = bytes.len();
        cache.store(
            Report {
                node,
                op: O::ID,
                interface: O::interface(),
                key,
                hit: true,
                millis: 0,
                bytes: bytes_len,
                with_cameras,
                stats: <O::Payload as payload::Build>::stats(&value),
            },
            &bytes,
        )?;
        return Ok(Cooked::make(key, value, true, 0, bytes_len));
    }

    let started = Instant::now();
    let value = op.render(&params, &inputs, grid)?;
    let millis = started.elapsed().as_millis() as u64;
    let bytes = <O::Payload as payload::Build>::encode(&value)?;
    let bytes_len = bytes.len();
    cache.store(
        Report {
            node,
            op: O::ID,
            interface: O::interface(),
            key,
            hit: false,
            millis,
            bytes: bytes_len,
            with_cameras,
            stats: <O::Payload as payload::Build>::stats(&value),
        },
        &bytes,
    )?;
    Ok(Cooked::make(key, value, false, millis, bytes_len))
}

/// **收敛样板**：一个算子只需要写「身份」与「怎么算」，其余（身份常量、参数类型、
/// 输出域、编解码、相机口径）全从这两行推。
///
/// ```ignore
/// px_op! { Fbm = params::FBM, params::fbm::Params, (), Field,
///          |p, _i, g| crate::ops::fbm::eval(p, &[], g) }
/// ```
///
/// ⚠ **身份全是自动的**：
///   * 源码指纹 = `build.rs` 算的（`env!("PX_SOURCE_HASH")`）—— 没有手维护的清单
///   * 接口哈希 = 三个类型名（`Params` / `Inputs` / `Payload`）—— 没有手写的 version
///
/// 两者都在 `PxOp` 里，所以图侧（键）与 dylib 侧（描述符）**天然同一个值**。
#[macro_export]
macro_rules! px_op {
    ($name:ident = $id:expr, $params:ty, $inputs:ty, $payload:ty,
     |$p:ident, $i:ident, $g:ident| $body:expr
     $(, interface = $interface:literal)?) => {
        impl $crate::PxOp for $name {
            const ID: &'static str = $id;

            type Params = $params;
            type Inputs = $inputs;
            type Payload = $payload;

            /// 这个 crate 编译进去**全部源码**的指纹 —— `build.rs` 算的，
            /// 没有手维护的清单（见 `build/fingerprint.rs`）。
            const SOURCE_HASH: &'static str = env!("PX_SOURCE_HASH");


            // 逃生门：给了字面量就强制失效（改类型之外的理由）。
            $(
                fn interface() -> u64 {
                    $crate::interface_hash(&[
                        ::core::any::type_name::<Self::Params>(),
                        ::core::any::type_name::<Self::Inputs>(),
                        ::core::any::type_name::<Self::Payload>(),
                        $interface,
                    ])
                }
            )?

            fn new() -> Self {
                $name
            }

            fn render(
                &self,
                $p: &$params,
                $i: &$inputs,
                $g: $crate::Grid,
            ) -> Result<$payload, String> {
                // ⚠ 不用任何 downcast：`cook<O>` 收的就是 `O::Inputs`，
                //    "图参数 struct 接错算子"是**编译错**。`$body` 先绑成值，
                //    免得 `?`/`return` 的语义被这里的表达式位置改掉。
                let $i = $i;
                let _ = &$i;
                Ok($body)
            }
        }

    };
}

/// `const` 上下文里的字符串相等（`str ==` 在那里用不了）。
pub const fn str_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}


/// **接口形状的哈希** —— 它取代了手写的 `version`。
///
/// 喂进来的是几个类型名（`Params` / `Inputs` / `Payload`）。于是：
///
/// * 改了参数 struct 的字段、改了输入 struct、改了输出域 ⇒ **自动变**（该重算）
/// * 只改了算法体 ⇒ **不变**（那是 `SOURCE_HASH` 管的事）
/// * **什么都不用记** —— 这正是 `version` 唯一的存在理由，而它是会忘的
///
/// ⚠ 参的是 `core::any::type_name` 的字符串，而它**不保证跨编译器稳定**。
///   我们的键只要求"同一台机器上前后一致"，所以够用；想按别的理由强制失效，
///   就给 `px_op!` 末尾加 `, interface = "n"`（逃生门）。
pub fn interface_hash(parts: &[&str]) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_cook/interface/v1");
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..8]);
    u64::from_le_bytes(bytes)
}

/// **无上游**那一档的形状：它没有名字问题，留在契约里。
impl PxInputs for () {
    fn collect(&self, _hasher: &mut blake3::Hasher) {}
}

// ── dylib 那一侧的管道（全部由宏生成，算子作者看不到）─────────────────────────

/// `PxInputs::from_payloads` 要的那个 trait —— 与 `PxInputs` 同一份实现。
pub use input_bytes::FromPayloads;

mod input_bytes {
    use crate::Grid;

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

}

impl<P: payload::Build> Cooked<P> {
    /// **从上游字节解出一个"已经拿到手"的节点**。
    ///
    /// ⚠ 算子侧写自己的 `<算子>Input` 时用它：`Cooked::from_bytes(bytes, grid)?`。
    /// 键不在这儿（字节里没有键），所以先给一把空键占位 —— 它只用于 dylib 那一侧的
    /// 重算路径，不参与任何键的计算。
    pub fn from_bytes(bytes: &[u8], grid: Grid) -> Result<Self, String> {
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

#[cfg(test)]
mod tests {
    use super::payload::Build;

    /// **哪些域的产物尺寸就是画布** —— 这条决定键里要不要掺画布。
    ///
    /// 病根：一刀切都会错。
    /// * 都掺 ⇒ 改画布连带重烘体积/网格（它们的尺寸由参数给，与画布无关）。
    /// * 都不掺 ⇒ 场出现「同一个键、不同分辨率」。
    #[test]
    fn only_the_field_domain_bakes_the_canvas_into_its_size() {
        assert!(
            <px_field_schema::field::Field as Build>::RESOLUTION_IS_CANVAS,
            "场的分辨率就是画布 ⇒ 画布必须进键"
        );
        assert!(
            !<px_volume_schema::VolumeData as Build>::RESOLUTION_IS_CANVAS,
            "体积的分辨率由参数（res/layers）给 ⇒ 画布与它无关"
        );
        assert!(
            !<px_mesh_schema::MeshData as Build>::RESOLUTION_IS_CANVAS,
            "网格的尺寸由参数给 ⇒ 画布与它无关"
        );
        // 顺带把"相机口径"也读一遍（`cook` 用它，不再有并行的 `OpKind`）。
        assert!(<px_field_schema::field::Field as Build>::WITH_CAMERAS);
        assert!(!<px_volume_schema::VolumeData as Build>::WITH_CAMERAS);
        assert!(<px_mesh_schema::MeshData as Build>::WITH_CAMERAS);
    }
}
