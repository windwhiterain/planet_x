use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::identity::PxKeyed;
use crate::keys::Key;
use crate::ops;
use crate::payload::Build;

pub trait PxInputs {
    fn collect(&self, hasher: &mut blake3::Hasher);
}

#[derive(Clone)]
pub struct Cooked<P> {
    pub key: Key,
    value: P,
    pub hit: bool,
    pub millis: u64,
    pub bytes: usize,
}

impl<P> Cooked<P> {
    pub fn new(key: Key, value: P, hit: bool, millis: u64, bytes: usize) -> Self {
        Self {
            key,
            value,
            hit,
            millis,
            bytes,
        }
    }

    pub fn value(&self) -> &P {
        &self.value
    }
}

impl<P: Build> Cooked<P> {
    pub fn of(value: P) -> Result<Self, String> {
        let bundle = P::encode(&value)?;
        let bytes = bundle.to_bytes("")?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"px_cook/cooked/v1");
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        let mut key = [0_u8; 32];
        key.copy_from_slice(hasher.finalize().as_bytes());
        Ok(Self::new(key, value, false, 0, 0))
    }
}

pub trait PxOp: Sized {
    const ID: &'static str;
    const LIB: &'static str;
    const SYMBOL: &'static str;

    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    type Inputs: PxInputs;
    type Payload: Build;

    fn new() -> Self;

    fn interface() -> u64 {
        interface_hash(&[
            ::core::any::type_name::<Self::Params>(),
            ::core::any::type_name::<Self::Inputs>(),
            ::core::any::type_name::<Self::Payload>(),
        ])
    }

    fn source_hash() -> Result<&'static str, String> {
        ops::source_hash(Self::LIB)
    }

    fn decl_hash() -> &'static str {
        ""
    }

    fn render(
        &self,
        params: &Self::Params,
        inputs: &Self::Inputs,
    ) -> Result<Self::Payload, String> {
        let body = ops::body::<Self>()?;
        body(params, inputs)
    }
}

impl PxInputs for () {
    fn collect(&self, _hasher: &mut blake3::Hasher) {}
}

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

            fn decl_hash() -> &'static str {
                env!("PX_SOURCE_HASH")
            }
        }
    };
}

#[macro_export]
macro_rules! px_body {
    ($name:ident, |$p:ident, $i:ident| $body:expr) => {
        #[unsafe(export_name = ::core::concat!(
                                                                    env!("CARGO_PKG_NAME"),
                                                                    "__",
                                                                    ::core::stringify!($name)
                                                                ))]
        pub extern "Rust" fn __px_body(
            $p: &<$name as $crate::PxOp>::Params,
            $i: &<$name as $crate::PxOp>::Inputs,
        ) -> ::core::result::Result<<$name as $crate::PxOp>::Payload, ::std::string::String> {
            ::core::result::Result::Ok($body)
        }
    };
}

#[macro_export]
macro_rules! px_body_raw {
    ($name:ident, $params:ty, $inputs:ty, $payload:ty, |$p:ident, $i:ident| $body:expr) => {
        #[unsafe(export_name = ::core::concat!(
                                                                    env!("CARGO_PKG_NAME"),
                                                                    "__",
                                                                    ::core::stringify!($name)
                                                                ))]
        pub extern "Rust" fn __px_body(
            $p: &$params,
            $i: &$inputs,
        ) -> ::core::result::Result<$payload, ::std::string::String> {
            ::core::result::Result::Ok($body)
        }
    };
}

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

        #[unsafe(export_name = ::core::concat!(env!("CARGO_PKG_NAME"), "__toolchain_hash"))]
        pub extern "Rust" fn __px_toolchain_hash() -> &'static str {
            env!("PX_TOOLCHAIN_HASH")
        }
    };
}
