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

/// An element function: one cell-wise arithmetic over a field.
///
/// `Params` and `Inputs` are `Sync` because the only loop in this family bands rows across threads
/// ([`fill`]); the bodies read their inputs and never mutate them.
pub trait ElementFn: 'static {
    type Params: Serialize + DeserializeOwned + Default + PxKeyed + Sync;
    type Inputs: PxInputs + Sync;
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

/// The one loop of this operator family. Rows are banded across threads
/// (`px_field_schema::parallel::rows`), and each thread writes a disjoint slice, so the result is
/// bit-for-bit the serial one (the gate for that is `row_bands.rs`).
///
/// Two things must hold before this runs, and neither can be checked here: every element body
/// ignores the direction argument (so one probe for the whole field is the same as asking per cell),
/// and the body is a pure function of its inputs (a payload may not read a source its key cannot
/// see).
pub fn fill<F: ElementFn>(
    params: &F::Params,
    inputs: &F::Inputs,
    cell: impl Fn(u32, u32, [f32; 2], [f32; 3]) -> f32 + Sync,
) -> Field {
    let shape = F::shape(params, inputs);
    // The direction is a constant map of the cell, but it is undefined for `Domain::Volume` (a
    // volume grid has no single direction), and asking per cell panics there — a panic crossing the
    // operator dylib boundary is uncatchable and aborts the process. So probe once: if the
    // projection has no direction, the closure gets a sentinel and decides for itself.
    let probe = if shape.width > 0 && shape.height > 0 {
        Field::filled_with(shape.width, shape.height, 0.0, shape.projection).direction_probe()
    } else {
        None
    };
    let direction = probe.unwrap_or([0.0; 3]);
    let cell = &cell;
    let data = px_field_schema::parallel::rows(
        shape.width as usize,
        shape.height as usize,
        |first: usize, count: usize, out: &mut [f32]| {
            for row in 0..count {
                let y = (first + row) as u32;
                let base = row * shape.width as usize;
                for x in 0..shape.width {
                    let uv = (
                        (x as f32 + 0.5) / shape.width.max(1) as f32,
                        (y as f32 + 0.5) / shape.height.max(1) as f32,
                    );
                    out[base + x as usize] = cell(x, y, [uv.0, uv.1], direction);
                }
            }
        },
    );
    Field::with_projection(shape.width, shape.height, data, shape.projection)
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
