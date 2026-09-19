//! **算子的样板收敛成宏**。
//!
//! 现在只有一支：
//!
//! * [`derive(PxParams)`] —— **超参数** struct：每个字段按自己的类型写进键。
//!
//! 计划中的下一支（见 `docs/generic-op-and-graph-integration.md`）：
//! `#[derive(PxOp)]` —— 从「身份 + 参数类型 + 输入 struct + 输出域」推出
//! `PxOp` 实现与 dylib 那一侧的 `OpDescriptor` / `canonical_params` / `call` / `<库名>_table`。

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
            fn key(&self, hasher: &mut ::blake3::Hasher) {
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

/// 给**图参数** struct 生成 `PxInputs`：`collect`（把上游的键折进来）+ `PxInputs` 的
/// 另一半 `from_payloads`（把上游的字节解回来）。
///
/// ```ignore
/// #[derive(PxInputs)]
/// pub struct MixInput { pub a: Cooked<Field>, pub b: Cooked<Field>, pub mask: Cooked<Field> }
/// //                    ↓ 生成
/// impl px_cook::PxInputs for MixInput {
///     fn collect(&self, hasher: &mut blake3::Hasher) {
///         hasher.update(b"a");    hasher.update(&self.a.key);
///         hasher.update(b"b");    hasher.update(&self.b.key);
///         hasher.update(b"mask"); hasher.update(&self.mask.key);
///     }
/// }
/// impl px_cook::FromPayloads for MixInput {
///     fn from_payloads(inputs: &[&[u8]], grid: Grid) -> Result<Self, String> {
///         let [a, b, mask] = inputs else { return Err(…); };
///         Ok(Self { a: Cooked::from_bytes(a, grid)?, … })
///     }
/// }
/// ```
///
/// ⚠ **字段名进键**（与 `PxParams` 同一条口径）：加一个字段又删一个，不会因为"值恰好一样"而撞。
/// ⚠⚠ **字段顺序 = 上游顺序**。这是"按位置解字节"的代价，与手写版一样 —— 但手写版是
///   `let [a, b, mask] = inputs` 摆在眼前，宏生成的要记住这条。改顺序 = 换接口。
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

    let mut idents = Vec::new();
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
        idents.push(ident);
    }
    let arity = idents.len();

    let expanded = quote! {
        impl ::px_cook::PxInputs for #name {
            fn collect(&self, hasher: &mut ::px_cook::blake3::Hasher) {
                #(#collect)*
            }
        }

        impl ::px_cook::FromPayloads for #name {
            fn from_payloads(
                inputs: &[&[u8]],
                grid: ::px_cook::Grid,
            ) -> Result<Self, String> {
                let [ #(#idents),* ] = inputs else {
                    return Err(format!(
                        "吃 {} 个上游，却收到 {} 个",
                        #arity,
                        inputs.len(),
                    ));
                };
                Ok(Self {
                    #(#idents: ::px_cook::Cooked::from_bytes(#idents, grid)?,)*
                })
            }
        }
    };
    expanded.into()
}
