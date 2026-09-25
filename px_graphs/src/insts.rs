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

/// The instance tier of the pre-flight gate, for graph exes that run bare (no
/// `px run` stage 1 in front of them). Whole-graph strictness on purpose: the
/// seven libraries are compiled as one stage-1 batch, so "some instance I did not
/// touch is missing" is still a stopped-state this family should refuse loudly
/// instead of discovering two frames into a bake. Callers are the graph-exe side
/// only; implementation libraries never reach `px_cook` (docs/invariants.md).
pub fn gate(graph: &str) -> Result<(), String> {
    let mut built = px_cook::inst::BuildGraph::new();
    build(&mut built);
    px_cook::inst::gate_ready(Some((graph, &built)), &[])
}
