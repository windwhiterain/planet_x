//! See docs/field.md

use px_field_schema::field::{Field, Projection, tangent_frame};
use px_field_schema::ops::{FieldPairInput, Warp};
use px_graph_schema::Cooked;

fn axis_ramp(width: u32, height: u32, axis: usize) -> Field {
    let mut field = Field::filled_with(width, height, 0.0, Projection::Equirect);
    for y in 0..height {
        for x in 0..width {
            field.set(x, y, field.direction(x, y)[axis] * 0.5 + 0.5);
        }
    }
    field
}

#[test]
fn the_warp_output_stays_continuous_across_the_polar_band() {
    let (width, height) = (32, 16);
    let warp_field = Field::filled_with(width, height, 0.6, Projection::Equirect);
    let params = px_field_schema::params::warp::Params::default();
    let body = px_graph_schema::ops::body::<Warp>()
        .unwrap_or_else(|err| panic!("field.warp does not load from its library: {err}"));

    let run = |input: &Field| {
        body(
            &params,
            &FieldPairInput {
                field: Cooked::of(input.clone()).expect("ramp input must wrap"),
                offset: Cooked::of(warp_field.clone()).expect("warp field must wrap"),
            },
        )
        .unwrap_or_else(|err| panic!("field.warp via the loaded library must cook: {err}"))
    };
    let out_x = run(&axis_ramp(width, height, 0));
    let out_y = run(&axis_ramp(width, height, 1));
    let out_z = run(&axis_ramp(width, height, 2));

    let mut polar_count = 0_usize;
    let mut magnitude_sum = 0.0_f32;
    let mut worst = 0.0_f32;
    for y in [0, height - 1] {
        let mut ring: Vec<[f32; 2]> = Vec::new();
        for x in 0..width {
            let direction = out_x.direction(x, y);
            if direction[1].abs() <= 0.99 {
                continue;
            }
            polar_count += 1;
            let displaced = [
                out_x.at(x, y) * 2.0 - 1.0 - direction[0],
                out_y.at(x, y) * 2.0 - 1.0 - direction[1],
                out_z.at(x, y) * 2.0 - 1.0 - direction[2],
            ];
            let (east, north) = tangent_frame(direction);
            let along = displaced[0] * east[0] + displaced[1] * east[1] + displaced[2] * east[2];
            let across =
                displaced[0] * north[0] + displaced[1] * north[1] + displaced[2] * north[2];
            magnitude_sum += (along * along + across * across).sqrt();
            ring.push([along, across]);
        }
        for index in 0..ring.len() {
            let here = ring[index];
            let next = ring[(index + 1) % ring.len()];
            let dot = here[0] * next[0] + here[1] * next[1];
            let cross = here[0] * next[1] - here[1] * next[0];
            worst = worst.max(cross.atan2(dot).to_degrees().abs());
        }
    }
    assert!(
        polar_count > 0,
        "polar band |direction[1]| > 0.99 holds no texel on 32x16 equirect \
         — the gate measures nothing (a skip is not a pass)"
    );
    let mean_magnitude = magnitude_sum / polar_count.max(1) as f32;
    assert!(
        mean_magnitude > 1e-3,
        "mean warp displacement {mean_magnitude:.5} is ~zero — the warp field \
         does not displace and the angle below passes vacuously"
    );
    assert!(
        worst < 1.0,
        "field.warp turns {worst:.4} deg between adjacent polar texels \
         (32x16 equirect, |direction[1]| > 0.99, {polar_count} texels, \
         measured 0.0010 deg green vs 180 deg on a flipped sample; \
         the old reversing frame turned 162 deg here)"
    );
}
