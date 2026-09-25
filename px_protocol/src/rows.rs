
use std::thread;

fn workers() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

pub fn fill_rows(out: &mut [f32], width: usize, fill: impl Fn(u32, &mut [f32]) + Sync) {
    assert!(width > 0, "宽度不能是 0");
    assert_eq!(out.len() % width, 0, "输出长度必须是宽度的整数倍");
    let groups = out.len() / width;
    let threads = workers().min(groups).max(1);
    if threads <= 1 || groups == 0 {
        for (index, slice) in out.chunks_mut(width).enumerate() {
            fill(index as u32, slice);
        }
        return;
    }

    let per = groups.div_ceil(threads);
    let fill = &fill;
    thread::scope(|scope| {
        for (index, chunk) in out.chunks_mut(per * width).enumerate() {
            let first = (index * per) as u32;
            scope.spawn(move || {
                for (offset, slice) in chunk.chunks_mut(width).enumerate() {
                    fill(first + offset as u32, slice);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parallel_field_is_bit_identical_to_a_serial_one() {
        let (width, height) = (97_usize, 53_usize);
        let cell = |y: u32, x: u32| {
            let mut acc = 0.0_f32;
            for step in 0..3 {
                acc += (y as f32 * 0.5 + x as f32 * 0.25 + step as f32).sin();
            }
            acc
        };
        let mut serial = vec![0.0_f32; width * height];
        for y in 0..height {
            for x in 0..width {
                serial[y * width + x] = cell(y as u32, x as u32);
            }
        }
        let mut parallel = vec![0.0_f32; width * height];
        fill_rows(&mut parallel, width, |y, line| {
            for (x, out) in line.iter_mut().enumerate() {
                *out = cell(y, x as u32);
            }
        });
        assert_eq!(serial.len(), parallel.len());
        for index in 0..serial.len() {
            assert_eq!(
                serial[index].to_bits(),
                parallel[index].to_bits(),
                "第 {index} 格不是逐位相同"
            );
        }
    }

    #[test]
    fn no_group_is_lost() {
        let (width, height) = (3_usize, 19_usize);
        let mut out = vec![0.0_f32; width * height];
        fill_rows(&mut out, width, |y, line| {
            for value in line.iter_mut() {
                *value = y as f32;
            }
        });
        for y in 0..height {
            for x in 0..width {
                assert_eq!(out[y * width + x], y as f32, "第 {y} 组第 {x} 列没被填");
            }
        }
    }
}
