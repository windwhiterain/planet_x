use px_protocol::art::{MeshData, PolylineData};

pub fn grid_mesh(
    positions: &[f32],
    normals: &[f32],
    uvs: &[f32],
    out_u: usize,
    out_v: usize,
) -> MeshData {
    let vertices = out_u.saturating_mul(out_v);
    if positions.len() < vertices * 3 || normals.len() < vertices * 3 || uvs.len() < vertices * 2 {
        return MeshData::default();
    }
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for vertex in 0..vertices {
        for lane in 0..3 {
            let value = f64::from(positions[vertex * 3 + lane]);
            low[lane] = low[lane].min(value);
            high[lane] = high[lane].max(value);
        }
    }
    let diagonal = {
        let delta = [high[0] - low[0], high[1] - low[1], high[2] - low[2]];
        (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
    };
    let step = (diagonal * 1e-6).max(f64::MIN_POSITIVE);
    let key = |vertex: usize| -> [i64; 3] {
        let mut out = [0_i64; 3];
        for lane in 0..3 {
            out[lane] = (f64::from(positions[vertex * 3 + lane]) / step).round() as i64;
        }
        out
    };

    let mut canonical: Vec<u32> = Vec::with_capacity(vertices);
    let mut lookup: std::collections::HashMap<[i64; 3], u32> = std::collections::HashMap::new();
    let mut kept_positions: Vec<f32> = Vec::new();
    let mut kept_normals: Vec<f32> = Vec::new();
    let mut kept_uvs: Vec<f32> = Vec::new();
    for vertex in 0..vertices {
        let index = match lookup.get(&key(vertex)) {
            Some(found) => *found,
            None => {
                let index = (kept_positions.len() / 3) as u32;
                kept_positions.extend_from_slice(&positions[vertex * 3..vertex * 3 + 3]);
                kept_normals.extend_from_slice(&normals[vertex * 3..vertex * 3 + 3]);
                kept_uvs.extend_from_slice(&uvs[vertex * 2..vertex * 2 + 2]);
                lookup.insert(key(vertex), index);
                index
            }
        };
        canonical.push(index);
    }

    let mut indices = Vec::with_capacity(out_u.saturating_mul(out_v) * 6);
    for i in 0..out_u.saturating_sub(1) {
        for j in 0..out_v.saturating_sub(1) {
            let a = canonical[i * out_v + j];
            let b = canonical[(i + 1) * out_v + j];
            let c = canonical[i * out_v + j + 1];
            let d = canonical[(i + 1) * out_v + j + 1];
            for triangle in [[a, b, c], [c, b, d]] {
                if triangle[0] != triangle[1]
                    && triangle[1] != triangle[2]
                    && triangle[0] != triangle[2]
                {
                    indices.extend_from_slice(&triangle);
                }
            }
        }
    }
    MeshData {
        positions: kept_positions,
        normals: kept_normals,
        uvs: kept_uvs,
        indices,
    }
}

pub fn polyline(points: &[[f64; 3]], closed: bool) -> PolylineData {
    let keep = if closed {
        points.len().saturating_sub(1)
    } else {
        points.len()
    };
    let mut positions = Vec::with_capacity(keep * 3);
    for point in &points[..keep] {
        positions.extend_from_slice(&[point[0] as f32, point[1] as f32, point[2] as f32]);
    }
    let mut indices = Vec::with_capacity(keep * 2);
    let span = if closed { keep } else { keep.saturating_sub(1) };
    for index in 0..span {
        indices.push(index as u32);
        indices.push(((index + 1) % keep.max(1)) as u32);
    }
    PolylineData { positions, indices }
}
