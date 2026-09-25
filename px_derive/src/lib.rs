//! See docs/operators.md

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

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
            fn key(&self, hasher: &mut ::px_graph_schema::blake3::Hasher) {
                #(#steps)*
            }
        }
    };
    expanded.into()
}

fn compile_error(message: &str) -> TokenStream {
    let message = syn::LitStr::new(message, proc_macro2::Span::call_site());
    quote! { ::core::compile_error!(#message); }.into()
}

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
