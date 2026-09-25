/// One band worker's panic, spoken instead of unwound. A panic that unwinds out of an operator body
/// reaches the dylib boundary, where it aborts the process rather than surfacing as a failure; a band
/// worker's panic has no other route back to its caller, so it is turned into an error here. The
/// banding stays bit-for-bit the serial result (`row_bands.rs`), and the cell closure's signature is
/// unchanged.
fn band_panic(payload: &(dyn std::any::Any + Send)) -> String {
    let what = if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "载荷不是文本".to_string()
    };
    format!("行带线程 panic：{what}")
}

pub fn rows<F>(width: usize, height: usize, band: F) -> Result<Vec<f32>, String>
where
    F: Fn(usize, usize, &mut [f32]) + Sync,
{
    // `thread::scope` panics at teardown when any of its threads panicked, and it does so *after* the
    // handle was joined — so handling the join result alone is not enough, and the scope re-panic is
    // caught here. The worker's own message is what the join sees first, so it is kept in preference
    // to the scope's placeholder.
    let mut worker: Option<String> = None;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        band_rows(width, height, &band, &mut worker)
    }));
    if let Some(message) = worker {
        return Err(message);
    }
    match outcome {
        Ok(data) => Ok(data),
        Err(payload) => Err(band_panic(&*payload)),
    }
}

fn band_rows<F>(width: usize, height: usize, band: &F, worker: &mut Option<String>) -> Vec<f32>
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
            if let Err(payload) = handle.join() {
                if worker.is_none() {
                    *worker = Some(band_panic(&*payload));
                }
            }
        }
    });
    data
}
