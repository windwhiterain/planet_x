//! See docs/elementwise.md

pub use px_field_schema::params::Shape;
use px_graph_schema::{PxInputs, PxKeyed};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub mod constant;
pub mod fuse;
pub mod mix;
pub mod remap;

pub mod specs;

pub use constant::ConstantParams;
pub use fuse::{FuseInput, FuseParams};
pub use mix::{MixInput, MixParams};
pub use px_field_schema::field::Field;
pub use px_graph_schema::interface_hash;
pub use remap::{RemapInput, RemapParams};
pub use specs::ELEM_SPECS;

pub fn like(field: &Field) -> Shape {
    Shape {
        width: field.width,
        height: field.height,
        projection: field.projection,
    }
}

pub fn all_roots(spec: &ElemSpec) -> Vec<&'static str> {
    let mut roots = vec!["px_elem", "px_field_schema"];
    for each in spec.roots {
        if !roots.contains(each) {
            roots.push(each);
        }
    }
    roots
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElemFacts {
    pub interface: u64,
    pub decl_hash: &'static str,
    pub params: &'static str,
    pub inputs: &'static str,
    pub payload: &'static str,
}

pub struct ElemSpec {
    pub ty: &'static str,
    pub name: &'static str,
    pub source: &'static str,
    pub roots: &'static [&'static str],
    pub body: &'static str,
    pub facts: fn() -> ElemFacts,
}

pub const DECL_HASH: &str = env!("PX_SOURCE_HASH");

pub trait ElementFn: 'static {
    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    type Inputs: PxInputs;
    const NAME: &'static str;
    const SOURCE: &'static str;
    const ROOTS: &'static [&'static str];
    const BODY: &'static str;
    const SYMBOL: &'static str;
    fn shape(params: &Self::Params, inputs: &Self::Inputs) -> Shape;
}

pub fn facts_of<F: ElementFn>() -> ElemFacts {
    ElemFacts {
        interface: interface_hash(&[
            ::core::any::type_name::<F::Params>(),
            ::core::any::type_name::<F::Inputs>(),
            ::core::any::type_name::<Field>(),
        ]),
        decl_hash: DECL_HASH,
        params: ::core::any::type_name::<F::Params>(),
        inputs: ::core::any::type_name::<F::Inputs>(),
        payload: ::core::any::type_name::<Field>(),
    }
}

pub fn fill<F: ElementFn>(
    params: &F::Params,
    inputs: &F::Inputs,
    cell: impl Fn(u32, u32, [f32; 2], [f32; 3]) -> f32,
) -> Field {
    let mut out = F::shape(params, inputs).filled(0.0);
    for y in 0..out.height {
        for x in 0..out.width {
            let uv = out.uv(x, y);
            let direction = out.direction(x, y);
            out.set(x, y, cell(x, y, [uv.0, uv.1], direction));
        }
    }
    out
}

#[macro_export]
macro_rules! px_elem_specs {
    ($(
        $(#[$meta:meta])*
        $ty:ident, $params:ty, $inputs:ty, $name:literal,
            source: $source:literal, roots: $roots:expr, shape: $shape:expr;
    )*) => {
        $(
            $(#[$meta])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $ty;

            impl $crate::ElementFn for $ty {
                type Params = $params;
                type Inputs = $inputs;
                const NAME: &'static str = $name;
                const SOURCE: &'static str = $source;
                const ROOTS: &'static [&'static str] = $roots;
                const SYMBOL: &'static str =
                    ::core::concat!("px_inst__", ::core::stringify!($ty));
                const BODY: &'static str = ::core::concat!(
                    "px_elem::fill::<px_elem::specs::",
                    ::core::stringify!($ty),
                    ">(p, i, |x, y, uv, direction| value(p, i, x, y, uv, direction))"
                );
                fn shape(params: &Self::Params, inputs: &Self::Inputs) -> $crate::Shape {
                    ($shape)(params, inputs)
                }
            }
        )*

        pub const ELEM_SPECS: &[$crate::ElemSpec] = &[
            $(
                $crate::ElemSpec {
                    ty: ::core::stringify!($ty),
                    name: $name,
                    source: $source,
                    roots: $roots,
                    body: <$ty as $crate::ElementFn>::BODY,
                    facts: || $crate::facts_of::<$ty>(),
                },
            )*
        ];
    };
}
