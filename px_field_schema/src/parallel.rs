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
