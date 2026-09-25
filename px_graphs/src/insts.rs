#[path = "inst_recipe.rs"]
pub mod recipe;

pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/insts_gen.rs"));
}

pub use generated::{Band, LatBands, Waves};

pub fn build(g: &mut px_cook::inst::BuildGraph) {
    for item in recipe::INSTANCES {
        let (interface, decl_hash) = generated::facts_of(item.type_name);
        g.facts(
            item.type_name,
            item.op_id,
            interface,
            decl_hash,
            item.roots,
            item.source,
            item.body,
        );
    }
    for spec in px_elem::ELEM_SPECS {
        let facts = (spec.facts)();
        let roots = px_elem::all_roots(spec);
        g.facts(
            spec.ty,
            spec.name,
            facts.interface,
            facts.decl_hash,
            &roots,
            spec.source,
            spec.body,
        );
    }
}

pub mod catalogue {
    include!(concat!(env!("OUT_DIR"), "/insts_gen_catalogue.rs"));
}

pub fn codegen() -> &'static px_cook::inst::InstCatalogue {
    static ALL: std::sync::OnceLock<px_cook::inst::InstCatalogue> = std::sync::OnceLock::new();
    ALL.get_or_init(|| px_cook::inst::InstCatalogue::from_generated(catalogue::INST_CODEGEN))
}
