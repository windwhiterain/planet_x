//! **算子的样板收敛成宏**。
//!
//! 两支：
//!
//! * [`derive(PxParams)`] —— **超参数** struct：每个字段按自己的类型写进键。
//! * [`derive(PxInputs)`] —— **图参数** struct：把每个上游的键折进键。
//!
//! ⚠ 算子**本身**那一份声明不走 derive，走 `px_graph_schema` 的 `px_op!`（它要同时把
//! 身份、符号名与"去哪个实现库取"钉在一起）。

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

/// 给**超参数** struct 生成 `PxKeyed`：每个字段**按自己的类型**写进 hasher。
///
/// ```ignore
/// #[derive(PxParams)]
/// #[serde(default, deny_unknown_fields)]
/// pub struct Coarse { pub res: u32, pub scale: f32 }
/// //                    ↓ 生成
/// impl px_graph_schema::PxKeyed for Coarse {
///     fn key(&self, hasher: &mut blake3::Hasher) {
///         hasher.update(b"res");   hasher.update(&self.res.to_le_bytes());
///         hasher.update(b"scale"); hasher.update(&self.scale.to_le_bytes());
///     }
/// }
/// ```
///
/// ⚠ 字段名也写进去：将来加一个字段又删一个，不会因为"值恰好一样"而撞键。
/// ⚠ 带 `#[nohash]` 的字段**跳过** —— 纯局部开关（不影响产物的那种）不该进键。
#[proc_macro_derive(PxParams, attributes(nohash))]
pub fn derive_px_params(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as DeriveInput);
    let name = &parsed.ident;

    let Data::Struct(data) = &parsed.data else {
        return compile_error("PxParams 只能用在 struct 上");
    };
    let Fields::Named(named) = &data.fields else {
        return compile_error("PxParams 要具名字段（键里要写字段名）");
    };

    let mut steps = Vec::new();
    for field in &named.named {
        let skipped = field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("nohash"));
        if skipped {
            continue;
        }
        let Some(ident) = &field.ident else {
            continue;
        };
        let label = ident.to_string();
        steps.push(quote! {
            hasher.update(#label.as_bytes());
            ::px_graph_schema::HashField::hash_field(&self.#ident, hasher);
        });
    }

    let expanded = quote! {
        impl ::px_graph_schema::PxKeyed for #name {
            // ⚠ 走 `px_graph_schema::blake3`（契约层的 re-export），**不是** `::blake3`：
            //   后者要求"用这个 derive 的 crate 自己直接依赖 blake3" —— 图侧现写算子
            //   （`px_cook::px_local_op!`）不该为了 derive 再添一个依赖。
            //   两个 derive 的口径在这一行上必须一致（另一个在 `PxInputs` 那边）。
            fn key(&self, hasher: &mut ::px_graph_schema::blake3::Hasher) {
                #(#steps)*
            }
        }
    };
    expanded.into()
}

/// 让"用错地方"也有一句人话，而不是 syn 的报错。
fn compile_error(message: &str) -> TokenStream {
    let message = syn::LitStr::new(message, proc_macro2::Span::call_site());
    quote! { ::core::compile_error!(#message); }.into()
}

/// 给**图参数** struct 生成 `PxInputs::collect`：把每个上游的**键**折进来。
///
/// ```ignore
/// #[derive(PxInputs)]
/// pub struct MixInput { pub a: Cooked<Field>, pub b: Cooked<Field>, pub mask: Cooked<Field> }
/// //                    ↓ 生成
/// impl px_graph_schema::PxInputs for MixInput {
///     fn collect(&self, hasher: &mut blake3::Hasher) {
///         hasher.update(b"a");    hasher.update(&self.a.key);
///         hasher.update(b"b");    hasher.update(&self.b.key);
///         hasher.update(b"mask"); hasher.update(&self.mask.key);
///     }
/// }
/// ```
///
/// ⚠ **字段名进键**（与 `PxParams` 同一条口径）：加一个字段又删一个，不会因为"值恰好一样"而撞。
/// ⚠ 只有这一半：图脚本是把**值**交给 `cook` 的（`MixInput { a, b, mask }`），
///   从来没有"按位置解上游字节"那条路 ⇒ 字段顺序不是接口的一部分，字段名才是。
#[proc_macro_derive(PxInputs)]
pub fn derive_px_inputs(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as DeriveInput);
    let name = &parsed.ident;

    let Data::Struct(data) = &parsed.data else {
        return compile_error("PxInputs 只能用在 struct 上");
    };
    let Fields::Named(named) = &data.fields else {
        return compile_error("PxInputs 要具名字段（键里要写字段名）");
    };

    let mut collect = Vec::new();
    for field in &named.named {
        let Some(ident) = &field.ident else {
            continue;
        };
        let label = ident.to_string();
        collect.push(quote! {
            hasher.update(#label.as_bytes());
            hasher.update(&self.#ident.key);
        });
    }

    let expanded = quote! {
        impl ::px_graph_schema::PxInputs for #name {
            fn collect(&self, hasher: &mut ::px_graph_schema::blake3::Hasher) {
                #(#collect)*
            }
        }
    };
    expanded.into()
}
