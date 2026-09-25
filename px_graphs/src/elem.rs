use px_elem::ElementFn;
use px_field_schema::field::Field;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/elem_gen.rs"));
}

pub use generated::*;

pub use px_elem::{
    ConstantParams, FuseInput, FuseParams, MixInput, MixParams, RemapInput, RemapParams,
};

pub fn key_of<F: ElementFn>() -> Result<String, String> {
    let facts = px_elem::facts_of::<F>();
    px_cook::inst::key_of_facts(
        "",
        facts.interface,
        facts.decl_hash,
        &px_elem::all_roots(spec_of::<F>()),
        spec_of::<F>().source,
        spec_of::<F>().body,
    )
}

fn spec_of<F: ElementFn>() -> &'static px_elem::ElemSpec {
    px_elem::ELEM_SPECS
        .iter()
        .find(|spec| F::NAME == spec.name)
        .unwrap_or_else(|| panic!("element 函数 `{}` 不在规格表里", F::NAME))
}

pub fn source_hash_of<F: ElementFn>() -> Result<&'static str, String> {
    static KEYS: std::sync::Mutex<Vec<(std::any::TypeId, &'static str)>> =
        std::sync::Mutex::new(Vec::new());
    let id = std::any::TypeId::of::<F>();
    let mut keys = KEYS.lock().expect("内容键缓存锁坏了");
    if let Some((_, key)) = keys.iter().find(|(each, _)| *each == id) {
        return Ok(key);
    }
    let key: &'static str = Box::leak(key_of::<F>()?.into_boxed_str());
    keys.push((id, key));
    Ok(key)
}

pub fn render<F: ElementFn>(params: &F::Params, inputs: &F::Inputs) -> Result<Field, String> {
    let path = px_cook::inst::library_path(source_hash_of::<F>()?);
    let body =
        px_graph_schema::ops::load_at::<ElemOp<F>>(path.to_string_lossy().as_ref(), F::SYMBOL)?;
    body(params, inputs)
}

pub struct ElemOp<F: ElementFn>(std::marker::PhantomData<F>);

impl<F: ElementFn> px_graph_schema::PxOp for ElemOp<F> {
    const ID: &'static str = F::NAME;
    const LIB: &'static str = "";
    const SYMBOL: &'static str = F::SYMBOL;
    type Params = F::Params;
    type Inputs = F::Inputs;
    type Payload = Field;
    fn new() -> Self {
        Self(std::marker::PhantomData)
    }
    fn source_hash() -> Result<&'static str, String> {
        source_hash_of::<F>()
    }
    fn decl_hash() -> &'static str {
        px_elem::DECL_HASH
    }
}
