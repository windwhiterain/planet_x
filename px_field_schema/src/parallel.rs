//! **行带并行**：把一张 `width × height` 的场按**行**分给若干线程填。
//!
//! ⚠⚠ 为什么按**行带**切、而不是按格交错：每个线程只写自己那一段、互不重叠，
//!   而且同一格里的运算顺序与串行**完全一样** ⇒ 结果**逐位相同**
//!   （缓存键与"可复现"都靠这一条；跨格累加在这里一次都没有）。
//!
//! ⚠ 为什么住在 `px_field_schema`：场算子（`px_field_op`）与体积算法（`px_volume_alg`）
//!   都要用它，而这两个 crate **共同的依赖只有这里**（一个是 dylib 薄壳、一个是 rlib 算法）。
//!   复制一份到两边就等于开两个会漂开的真相。
//!
//! ⚠ 线程数取 `available_parallelism`，并且**不超过行数**（行比线程少时别空转）。
//! ⚠ 单线程与并行走的是**同一个** `band` 闭包 —— 不给"小图有单独实现"留分叉的机会。

/// 用 `band(首行, 行数, 这一段输出)` 并行填满 `width × height` 的场，返回它的数据。
///
/// `band` 会被多个线程同时调用（`Fn` + `Sync`）：它只读外面的参数、只写传给它的那一段。
pub fn rows<F>(width: usize, height: usize, band: F) -> Vec<f32>
where
    F: Fn(usize, usize, &mut [f32]) + Sync,
{
    let mut data = vec![0.0_f32; width.saturating_mul(height)];
    if width == 0 || height == 0 {
        return data;
    }
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(height)
        .max(1);
    if threads <= 1 {
        band(0, height, &mut data);
        return data;
    }
    // 行带：尽量均分，余数给前面几条带（与线程数无关的确定性切法）。
    let base = height / threads;
    let extra = height % threads;
    let mut starts = Vec::with_capacity(threads + 1);
    let mut row = 0_usize;
    for index in 0..threads {
        starts.push(row);
        row += base + usize::from(index < extra);
    }
    starts.push(height);
    let band = &band;
    std::thread::scope(|scope| {
        let mut rest: &mut [f32] = &mut data;
        let mut handles = Vec::with_capacity(threads);
        for pair in starts.windows(2) {
            let (first, last) = (pair[0], pair[1]);
            let rows = last - first;
            let (slice, tail) = rest.split_at_mut(rows * width);
            rest = tail;
            handles.push(scope.spawn(move || band(first, rows, slice)));
        }
        for handle in handles {
            handle.join().expect("行带线程不该 panic");
        }
    });
    data
}
