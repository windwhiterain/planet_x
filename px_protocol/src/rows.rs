//! **按行并行**：把一整张场的逐格计算分给几个线程。
//!
//! ⚠⚠ **只用"按输出分区"，绝不用并行归约。**
//!   本仓的地基是「**同参数 ⇒ 同产物字节**」：键是产物字节的 `blake3`，
//!   而**浮点加法不满足结合律** ⇒ 并行归约（`sum()` / `reduce()`）的求和顺序随线程调度变，
//!   同一份参数会算出**不同的字节**、于是**不同的键** ⇒ 缓存永不命中。
//!   按输出分区没有这个问题：**每一格只由它自己那一个线程算**，
//!   与"一个线程从头算到尾"逐位相同（`a_parallel_field_is_bit_identical_to_a_serial_one` 钉着这条）。
//!
//! ⚠ 不引第三方库（rayon）：这个工作区**离线构建**，本地没有 registry 缓存
//!   （实测 `~/.cargo/registry/cache` 里没有 rayon），而 `std::thread::scope` 够用。
//!
//! ⚠ 为什么按**行**切：一行是 `width` 个 `f32`，而 `data` 是行主序的连续内存
//!   ⇒ `par_chunks_mut(width)` 切的块互不相交，不必任何锁。

use std::thread;

/// 本机的并行度（至少 1）。⚠ `available_parallelism` 在受限环境会失败 ⇒ 退回 1（串行）。
fn workers() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// 逐格填 `out`：第 `group` 组的 `width` 个值由 `fill` 写进它自己那一段。
///
/// `out.len()` 必须是 `width` 的整数倍。
///
/// ⚠ 闭包拿到的是**它自己那一段的可变切片**（不是"格号 + 全局缓冲"）：
///   于是"写到别人的格上"在**类型上**就不可能 —— 前一版让闭包自己往 `&mut data` 里写，
///   既要同时持有可变与不可变借用（借用检查器当场拒），也确实存在写错格的危险。
///
/// ⚠ 组数少于线程数、或并行度是 1 时**直接串行**：线程启动开销比算一格还大，
///   而小图（`face = 16`）在这种档上很常见。
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

    // 每个线程拿一段**连续的组**（切点向上取整，保证覆盖全部组）。
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

    /// **按行并行与串行逐位相同**。
    ///
    /// ⚠ 这条是本仓的命门：键 = 产物字节，而浮点加法不满足结合律。
    ///   这里刻意用一个**含加法的**函数（不是"每格自成一世界"那种平凡情形），
    ///   因为只测平凡情形就测不出"归约顺序变了"这类错。
    #[test]
    fn a_parallel_field_is_bit_identical_to_a_serial_one() {
        let (width, height) = (97_usize, 53_usize);
        // ⚠ 一处定义、两种调度：串行那一份**逐行调同一个函数**（而不是另写一个表达式
        //   —— 第一版正是另写了一个，于是"两边算的根本不是同一个函数"，
        //   判据失败却与并行无关。判据必须只让"调度"这一个变量变）。
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

    /// 组数不是线程数的整数倍时**一组不漏**（切点取整最容易丢掉尾巴）。
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
