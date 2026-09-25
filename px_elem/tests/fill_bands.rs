//! See docs/elementwise.md
//!
//! `px_elem::fill` is the only loop in the element family and it bands rows across threads. The
//! banding helper has its own bit-exact gate (`px_field_schema/tests/row_bands.rs`) and the operator
//! path has one (`px_graphs/tests/elem.rs`), but neither pins this junction: that `fill` hands each
//! thread the same per-cell inputs the serial loop did, for the same cells.
//!
//! Two shapes to get wrong: the row offset a band starts at, and the direction/uv a cell sees. The
//! fake element function below makes both observable — its value depends on `y`, on `x`, and on the
//! direction it is handed.

use px_elem::{ElementFn, Shape, fill};
use px_field_schema::field::{Field, Projection};
use serde::{Deserialize, Serialize};

/// `Projection` is not a hashable field for `PxParams`, so the params carry its code and the shape
/// converts back. Only the `u32` matters for this gate: it keeps the projection out of the value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, px_derive::PxParams)]
struct ProbeParams {
    width: u32,
    height: u32,
    projection_code: u32,
    scale: f32,
}

impl ProbeParams {
    fn projection(&self) -> Projection {
        Projection::from_code(self.projection_code as u8).unwrap_or(Projection::Equirect)
    }
}

#[derive(Default)]
struct Probe;

impl ElementFn for Probe {
    type Params = ProbeParams;
    type Inputs = ();
    const NAME: &'static str = "probe";
    const SOURCE: &'static str = "tests/row_bands.rs";
    const ROOTS: &'static [&'static str] = &[];
    const BODY: &'static str = "";
    const SYMBOL: &'static str = "px_inst__Probe";

    fn shape(params: &Self::Params, _inputs: &Self::Inputs) -> Shape {
        Shape {
            width: params.width,
            height: params.height,
            projection: params.projection(),
        }
    }
}

fn cell(params: &ProbeParams, x: u32, y: u32, uv: [f32; 2], direction: [f32; 3]) -> f32 {
    let mut acc = 0.0_f32;
    for step in 0..4 {
        acc += ((y as f32 + 0.5) * 0.25 + x as f32 * (params.width as f32).recip() + step as f32)
            .sin();
    }
    acc + (y * params.width + x) as f32 * 1e-6
        + direction[0] * params.scale
        + direction[1] * 0.5
        + direction[2] * 0.25
        + uv[0] * 1e-3
        + uv[1] * 2e-3
}

/// The serial loop `fill` used to run, kept verbatim as the oracle: same probe rule, same uv
/// arithmetic, same write order.
fn serial(params: &ProbeParams) -> Field {
    let shape = <Probe as ElementFn>::shape(params, &());
    let mut out = Field::filled_with(shape.width, shape.height, 0.0, shape.projection);
    let probe = if out.width > 0 && out.height > 0 {
        out.direction_probe()
    } else {
        None
    };
    let direction = probe.unwrap_or([0.0; 3]);
    for y in 0..out.height {
        for x in 0..out.width {
            let uv = out.uv(x, y);
            out.set(x, y, cell(params, x, y, [uv.0, uv.1], direction));
        }
    }
    out
}

#[test]
fn the_banded_fill_is_bit_identical_to_the_serial_loop() {
    let cases = [
        (97_u32, 53_u32, Projection::Equirect),
        (1, 1, Projection::Equirect),
        (3, 1, Projection::Equirect),
        (4, 129, Projection::Equirect),
        (64, 65, Projection::CubeMap),
        (16, 0, Projection::Equirect),
        (0, 16, Projection::Equirect),
    ];
    for (width, height, projection) in cases {
        let params = ProbeParams {
            width,
            height,
            projection_code: projection.code() as u32,
            scale: 0.75,
        };
        let banded = fill::<Probe>(&params, &(), |x, y, uv, direction| {
            cell(&params, x, y, uv, direction)
        });
        let expected = serial(&params);
        assert_eq!(
            (banded.width, banded.height, banded.projection),
            (expected.width, expected.height, expected.projection),
            "{width}×{height}：形状或投影变了"
        );
        assert_eq!(
            banded.data.len(),
            expected.data.len(),
            "{width}×{height}：长度"
        );
        for index in 0..expected.data.len() {
            assert_eq!(
                banded.data[index].to_bits(),
                expected.data[index].to_bits(),
                "{width}×{height} 的第 {index} 格不是逐位相同"
            );
        }
    }
}

/// A volume field has no single direction: `fill` must probe once instead of asking per cell, hand
/// the closure the sentinel, and never call the panicking `direction`. The oracle uses the same
/// sentinel, so bit equality is also what proves the sentinel was the value handed over.
#[test]
fn a_volume_field_gets_the_direction_sentinel_and_does_not_panic() {
    let params = ProbeParams {
        width: 8,
        height: 4,
        projection_code: Projection::Volume.code() as u32,
        scale: 1.0,
    };
    let banded = fill::<Probe>(&params, &(), |x, y, uv, direction| {
        cell(&params, x, y, uv, direction)
    });
    let expected = serial(&params);
    assert_eq!(banded.data.len(), 32);
    for index in 0..expected.data.len() {
        assert_eq!(
            banded.data[index].to_bits(),
            expected.data[index].to_bits(),
            "体积场第 {index} 格不是逐位相同"
        );
    }
}
