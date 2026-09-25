use px_volume_schema::ops::Density;

const DENSITY_GATE: f32 = 0.02;

px_graph_schema::px_body! {
    Density,
    |p, i| {
        let mut volume = px_volume_alg::bake_density(p, i.density.value())?;
        if DENSITY_GATE > 0.0 {
            let span = 1.0 - DENSITY_GATE;
            for value in &mut volume.data {
                *value = if *value < DENSITY_GATE {
                    0.0
                } else {
                    (*value - DENSITY_GATE) / span
                };
            }
        }
        volume
    }
}
