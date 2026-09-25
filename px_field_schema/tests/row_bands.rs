//! See docs/field.md

use px_field_schema::parallel::rows;

/// A deterministic per-cell value with enough work in it that a band split cannot hide an
/// off-by-one: every cell depends on both its own coordinates and on `width`, so a thread
/// that receives the wrong row offset or the wrong slice length produces different bits.
fn cell(width: usize, y: usize, x: usize) -> f32 {
    let mut acc = 0.0_f32;
    for step in 0..4 {
        acc += ((y as f32 + 0.5) * 0.25 + (x as f32) * (width as f32).recip() + step as f32).sin();
    }
    acc + (y * width + x) as f32 * 1e-6
}

fn serial(width: usize, height: usize) -> Vec<f32> {
    let mut out = vec![0.0_f32; width * height];
    for y in 0..height {
        for x in 0..width {
            out[y * width + x] = cell(width, y, x);
        }
    }
    out
}

/// The gate `px_field_schema::parallel::rows` was missing: a banded field must be **bit**
/// identical to the same field computed in one pass, not merely close. `rows` hands each
/// thread a disjoint slice, so any overlap or gap shows up as differing bits here.
/// Heights are chosen across the remainder path (`height % threads`) rather than only at
/// multiples, because that is where a split goes wrong.
#[test]
fn a_banded_field_is_bit_identical_to_a_serial_one() {
    let cases = [
        (97_usize, 53_usize),
        (1, 1),
        (3, 1),
        (4, 129),
        (7, 8),
        (64, 65),
    ];
    for (width, height) in cases {
        // Mirrors what production does (`fbm3` / `ridged3` iterate `0..count`), so a wrong
        // band length is caught here rather than papered over by re-deriving it from the
        // slice. The assertion below then proves `count` agreed with the slice it was given.
        let parallel = rows(width, height, |first, count, out| {
            assert_eq!(
                out.len(),
                count * width,
                "第 {first} 行起给了 {count} 行的计数，切片却是 {} 格",
                out.len()
            );
            for row in 0..count {
                let y = first + row;
                let base = row * width;
                for x in 0..width {
                    out[base + x] = cell(width, y, x);
                }
            }
        });
        let expected = serial(width, height);
        assert_eq!(parallel.len(), width * height, "{width}×{height}: 长度不对");
        for index in 0..expected.len() {
            assert_eq!(
                parallel[index].to_bits(),
                expected[index].to_bits(),
                "{width}×{height} 的第 {index} 格不是逐位相同"
            );
        }
    }
}

/// `rows` allocates the buffer itself, so an empty dimension must produce an empty field
/// rather than indexing a zero-length band.
#[test]
fn an_empty_dimension_yields_an_empty_field() {
    for (width, height) in [(0_usize, 0_usize), (0, 16), (16, 0)] {
        let field = rows(width, height, |_, _, _| unreachable!("空尺寸不该调用闭包"));
        assert!(field.is_empty(), "{width}×{height} 应当是空场");
    }
}
