//! **契约**：一个算子长什么样、怎么调 —— 但**一行实现都没有**。
//!
//! 实现住在 `px_*_op` 的 dylib 里（运行时装载，见 [`crate::ops`]）。所以这一层必须比谁都低：
//! 数据（`px_*_schema`）、算子声明、图脚本都要它，而它谁都不依赖（除了线格式）。
//!
//! ⚠ `render` 与 `source_hash` 都是**提供的**方法：它们只做两件事 —— 去实现库里取那个符号、
//!   把参数交过去。于是"加一个算子"在声明侧只剩「身份 + 三个类型」。

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::identity::PxKeyed;
use crate::keys::Key;
use crate::ops;
use crate::payload::Build;
use crate::protocol::Grid;

/// **图参数**：包着上游节点，键由上游的键聚合而成。
///
/// 它就是图脚本里那个 struct（`#[derive(PxInputs)]` 生成 `collect`），字段名就是它吃的东西
/// 的名字 ⇒ 图侧写错一个字段、少给一个上游都是**编译错**。
pub trait PxInputs {
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
    /// 只给 `cook` 用：把刚算出来的（或者刚解出来的）值包成一个"已经拿到手的节点"。
    pub fn new(key: Key, value: P, hit: bool, millis: u64, bytes: usize) -> Self {
        Self {
            key,
            value,
            hit,
            millis,
            bytes,
        }
    }

    /// 类型化的值 —— 算子读上游、图脚本读结果，都走这一条。
    pub fn value(&self) -> &P {
        &self.value
    }
}

/// **一个算子**：身份 + 参数类型 + 输出域 + 怎么算。
pub trait PxOp: Sized {
    /// 人读的那个 id（`field.fbm`）—— **它也进键**（见 [`crate::OpId`]）。
    const ID: &'static str;
    /// 实现住在哪个库（`px_field_op`）。装载、报错、陈旧检查都用它。
    const LIB: &'static str;
    /// 实现库里那个符号的名字：由 `px_op!` 从 `LIB` + 类型名拼出来（**编译期常量**，
    /// 不是运行期查表 —— 这一层没有注册表、没有字符串 id 分派）。
    const SYMBOL: &'static str;

    /// 超参数：从 `art/<图>/<节点名>.toml` 解出来的那一份。
    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    /// **图参数**：这个算子被接的那个输入 struct。
    type Inputs: PxInputs;
    /// 输出域（`Field` / `VolumeData` / `MeshData`）—— 域、相机口径、编解码全从它推。
    type Payload: Build;

    /// 造一个算子实例（算子无状态，`new()` 就是 `Self`）。
    fn new() -> Self;

    /// **接口形状哈希** —— 取代手写的 `version`，见 [`crate::contract::interface_hash`]。
    fn interface() -> u64 {
        // ⚠ **不缓存**：泛型函数里的 `static` 在这里**不按单态化分开**（实测：
        //   `cached_interface::<A>` 与 `::<B>` 拿到同一个值），缓存反而制造 bug。
        //   代价是每次 `cook` 多哈希三个类型名 —— 可以忽略。
        // ⚠ 类型名在字段改名/增删时会变 —— 那正是我们要的信号。
        interface_hash(&[
            ::core::any::type_name::<Self::Params>(),
            ::core::any::type_name::<Self::Inputs>(),
            ::core::any::type_name::<Self::Payload>(),
        ])
    }

    /// **实现那一份源码**的指纹 —— 从**实现库的运行期身份**取，不是编译期常量。
    ///
    /// ⚠ 这是"图程序不重编也能看出实现换了"的全部机关：它进键 ⇒ 改了实现必然换键
    ///   （不会陈旧命中），而图 exe 的字节一位没动。
    fn source_hash() -> Result<&'static str, String> {
        ops::source_hash(Self::LIB)
    }

    /// **声明所在 crate** 的源码指纹（`px_op!` 自动填它所在 crate 那一份）。
    ///
    /// ⚠ 泛型实例（`px_cook::px_inst!`）的键必须覆盖它：接口哈希只哈希三个**类型名**，
    ///   不含字段布局 —— 往 `CloudCoarseInput` 加一个字段时，图程序会重编而实例库的键不变，
    ///   复用一份按**旧布局**编出来的 DLL 就是越界读写。见 `19-generic-inst.md` §177。
    ///
    /// 图侧现写的算子（`px_local_op!`）与不吃泛型参数的算子不用管它（默认空串）。
    fn decl_hash() -> &'static str {
        ""
    }

    /// 怎么算：把参数 / 图参数 / 画布交给实现库里那个符号。
    fn render(
        &self,
        params: &Self::Params,
        inputs: &Self::Inputs,
        grid: Grid,
    ) -> Result<Self::Payload, String> {
        let body = ops::body::<Self>()?;
        body(params, inputs, grid)
    }
}

/// **无上游**那一档的形状：它没有名字问题，留在契约里。
///
/// 于是图脚本写 `cook::<field::Fbm>(&graph, "clusters", ())` —— 那个 `()` 就是它。
impl PxInputs for () {
    fn collect(&self, _hasher: &mut blake3::Hasher) {}
}

/// **接口形状的哈希** —— 它取代了手写的 `version`。
///
/// 喂进来的是几个类型名（`Params` / `Inputs` / `Payload`）。于是：
///
/// * 改了参数 struct 的字段、改了输入 struct、改了输出域 ⇒ **自动变**（该重算）
/// * 只改了算法体 ⇒ **不变**（那是源码指纹管的事）
/// * **什么都不用记** —— 这正是 `version` 唯一的存在理由，而它是会忘的
///
/// ⚠ 参的是 `core::any::type_name` 的字符串，而它**不保证跨编译器稳定**。
///   我们的键只要求"同一台机器上前后一致"，所以够用；想按别的理由强制失效，
///   就改声明里那个 id 字面量（它是身份的一部分）。
///
/// ⚠ `b"px_cook/interface/v1"` 是**域分隔符**（键的域与接口哈希的域要分得开），
///   不是一个可以顺手改的字符串：改它 = 全仓换键。
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

/// **算子声明**：一个算子只需要写「身份 + 三个类型」，其余全从这三样推
/// （接口形状哈希、源码指纹、以及"去实现库里调哪个符号"）。
///
/// ```ignore
/// px_op! { Fbm, "field.fbm", "px_field_op", params::fbm::Params, (), Field }
/// ```
///
/// ⚠ 三个类型是**接口的形状**：图侧接错一个上游、少给一个字段，都是编译错。
#[macro_export]
macro_rules! px_op {
    ($(#[$meta:meta])* $name:ident, $id:literal, $lib:literal, $params:ty, $inputs:ty, $payload:ty) => {
        $(#[$meta])*
        pub struct $name;

        impl $crate::PxOp for $name {
            const ID: &'static str = $id;
            const LIB: &'static str = $lib;
            const SYMBOL: &'static str =
                ::core::concat!($lib, "__", ::core::stringify!($name));

            type Params = $params;
            type Inputs = $inputs;
            type Payload = $payload;

            fn new() -> Self {
                $name
            }

            /// 声明就住这个 crate ⇒ `env!("PX_SOURCE_HASH")` 就是它的指纹（见 `PxOp::decl_hash`）。
            fn decl_hash() -> &'static str {
                env!("PX_SOURCE_HASH")
            }
        }
    };
}

/// **算子实现**：与 `px_op!` 同名的那一条，导出实现库里的一个符号。
///
/// ```ignore
/// px_body! { Fbm, |p, _i, g| crate::ops::fbm::eval(p, &[], g) }
/// ```
///
/// ⚠ 符号名 = `<本库的包名>__<算子类型名>` —— 与声明侧那句
///   `concat!(LIB, "__", stringify!(算子))` 是同一个字符串。拼不上的那天是
///   **装载失败**（带命令的报错），不是默默调错函数。
#[macro_export]
macro_rules! px_body {
    ($name:ident, |$p:ident, $i:ident, $g:ident| $body:expr) => {
        #[unsafe(export_name = ::core::concat!(
                    env!("CARGO_PKG_NAME"),
                    "__",
                    ::core::stringify!($name)
                ))]
        pub extern "Rust" fn __px_body(
            $p: &<$name as $crate::PxOp>::Params,
            $i: &<$name as $crate::PxOp>::Inputs,
            $g: $crate::Grid,
        ) -> ::core::result::Result<<$name as $crate::PxOp>::Payload, ::std::string::String> {
            // ⚠ 外面套一层 `Ok`：`$body` 是一个**值**（内部函数回 `Result` 时，
            //   就在块里用 `?` —— 那个 `?` 从本函数往外传）。
            ::core::result::Result::Ok($body)
        }
    };
}

/// **实现库的身份**：每个实现库在自己的 `lib.rs` 里写一次。
///
/// 它导出两样东西：
///
/// * `…__source_hash` —— 这个库编译进去的**全部源码**指纹（进键的那一半
///   "实现是哪一份"）；
/// * `…__contract_hash` —— 它编的时候**契约**是哪一份。装载时与图程序手里那一份比：
///   对不上说明类型布局可能已经不一样了 ⇒ 当场拒（见 `ops::library`）。
#[macro_export]
macro_rules! px_impl_lib {
    () => {
        #[unsafe(export_name = ::core::concat!(env!("CARGO_PKG_NAME"), "__source_hash"))]
        pub extern "Rust" fn __px_source_hash() -> &'static str {
            env!("PX_SOURCE_HASH")
        }

        #[unsafe(export_name = ::core::concat!(env!("CARGO_PKG_NAME"), "__contract_hash"))]
        pub extern "Rust" fn __px_contract_hash() -> &'static str {
            $crate::SOURCE_HASH
        }

        /// ⚠ **工具链身份**：`extern "Rust"` 的 ABI 由编译器定，契约一致还不够。
        /// 实例库（`px_jit build` 生成的）不是 cargo 沿主 workspace 编的 ⇒ 这一道尤其要紧。
        #[unsafe(export_name = ::core::concat!(env!("CARGO_PKG_NAME"), "__toolchain_hash"))]
        pub extern "Rust" fn __px_toolchain_hash() -> &'static str {
            env!("PX_TOOLCHAIN_HASH")
        }
    };
}
