use px_nurbs_schema::curve::Curve;
use px_nurbs_schema::mesh::{grid_mesh, polyline};
use px_nurbs_schema::surface::Surface;
use px_protocol::art::{MeshData, PolylineData};

pub fn curve(
    curve: &Curve,
    tolerance: f64,
    depth: u32,
    segments: u32,
) -> Result<PolylineData, String> {
    let (low, high) = curve.domain();
    let steps = segments.max(1) as usize;
    let mut values: Vec<f64> = (0..=steps)
        .map(|step| low + (high - low) * step as f64 / steps as f64)
        .collect();
    let mut points: Vec<[f64; 3]> = values
        .iter()
        .map(|value| curve.point(*value))
        .collect::<Result<_, _>>()?;

    for _ in 0..depth {
        let mut next_values = Vec::with_capacity(values.len() * 2 - 1);
        let mut next_points: Vec<[f64; 3]> = Vec::with_capacity(points.len() * 2 - 1);
        let mut split = false;
        for index in 0..values.len() - 1 {
            let a = points[index];
            let b = points[index + 1];
            let middle = (values[index] + values[index + 1]) * 0.5;
            let point = curve.point(middle)?;
            let chord = [
                (a[0] + b[0]) * 0.5,
                (a[1] + b[1]) * 0.5,
                (a[2] + b[2]) * 0.5,
            ];
            next_values.push(values[index]);
            next_points.push(a);
            if distance(point, chord) > tolerance {
                split = true;
                next_values.push(middle);
                next_points.push(point);
            }
        }
        next_values.push(values[values.len() - 1]);
        next_points.push(points[points.len() - 1]);
        values = next_values;
        points = next_points;
        if !split {
            break;
        }
    }

    let closed = distance(points[0], points[points.len() - 1]) <= 1e-12;
    Ok(polyline(&points, closed))
}

pub fn surface(
    surface: &Surface,
    tolerance: f64,
    depth: u32,
    segments: u32,
) -> Result<MeshData, String> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    if !(u1 > u0) || !(v1 > v0) {
        return Err("细分一份参数域为空的曲面".to_string());
    }
    let steps = u64::from(segments.clamp(1, 64));
    let depth = depth.min(16);
    let mut level = 0_u32;
    loop {
        let unit = steps << level;
        let mut worst = 0.0_f64;
        for i in 0..unit {
            for j in 0..unit {
                let error = cell_error(surface, u0, u1, v0, v1, unit, i, j)?;
                if error > worst {
                    worst = error;
                }
            }
        }
        if worst <= tolerance || level >= depth {
            return grid(surface, u0, u1, v0, v1, unit);
        }
        level += 1;
    }
}

fn grid(
    surface: &Surface,
    u0: f64,
    u1: f64,
    v0: f64,
    v1: f64,
    unit: u64,
) -> Result<MeshData, String> {
    let along = (unit + 1) as usize;
    let vertices = along * along;
    let mut positions = vec![0.0_f32; vertices * 3];
    let mut normals = vec![0.0_f32; vertices * 3];
    let mut uvs = vec![0.0_f32; vertices * 2];
    for i in 0..along {
        for j in 0..along {
            let u = u0 + (u1 - u0) * i as f64 / unit as f64;
            let v = v0 + (v1 - v0) * j as f64 / unit as f64;
            let patch = surface.patch(u, v)?;
            let index = i * along + j;
            for lane in 0..3 {
                positions[index * 3 + lane] = patch.point[lane] as f32;
            }
            let normal = patch.normal.unwrap_or([0.0, 1.0, 0.0]);
            for lane in 0..3 {
                normals[index * 3 + lane] = normal[lane] as f32;
            }
            uvs[index * 2] = u as f32;
            uvs[index * 2 + 1] = v as f32;
        }
    }
    Ok(grid_mesh(&positions, &normals, &uvs, along, along))
}

fn cell_error(
    surface: &Surface,
    u0: f64,
    u1: f64,
    v0: f64,
    v1: f64,
    unit: u64,
    i: u64,
    j: u64,
) -> Result<f64, String> {
    let parameter = |index: u64| -> (f64, f64) {
        (
            u0 + (u1 - u0) * index as f64 / unit as f64,
            v0 + (v1 - v0) * index as f64 / unit as f64,
        )
    };
    let (u, _) = parameter(i);
    let (next_u, _) = parameter(i + 1);
    let (_, bottom) = parameter(j);
    let (_, top) = parameter(j + 1);
    let corners = [
        surface.point(u, bottom)?,
        surface.point(next_u, bottom)?,
        surface.point(u, top)?,
        surface.point(next_u, top)?,
    ];
    let middle = surface.point((u + next_u) * 0.5, (bottom + top) * 0.5)?;
    Ok(distance(middle, average4(corners)))
}

fn distance(one: [f64; 3], two: [f64; 3]) -> f64 {
    let delta = [one[0] - two[0], one[1] - two[1], one[2] - two[2]];
    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
}

fn average4(points: [[f64; 3]; 4]) -> [f64; 3] {
    let mut out = [0.0_f64; 3];
    for point in points {
        for lane in 0..3 {
            out[lane] += point[lane] * 0.25;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tessellated_sphere_stays_on_the_radius() {
        let ball = px_nurbs_schema::surface::sphere(2.0, [0.0; 3], 2).expect("造球");
        let mesh = surface(&ball, 1e-3, 4, 2).expect("细分");
        assert!(mesh.vertices() > 0 && mesh.triangles() > 0);
        for vertex in 0..mesh.vertices() {
            let point = &mesh.positions[vertex * 3..vertex * 3 + 3];
            let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
            assert!(
                (radius - 2.0).abs() < 1e-5,
                "第 {vertex} 个顶点离球心 {radius}（应当是 2）"
            );
        }
    }

    #[test]
    fn the_tessellated_sphere_is_watertight() {
        let ball = px_nurbs_schema::surface::sphere(1.0, [0.0; 3], 2).expect("造球");
        let mesh = surface(&ball, 1e-2, 3, 2).expect("细分");
        let mut edges: std::collections::HashMap<(u32, u32), usize> =
            std::collections::HashMap::new();
        for triangle in mesh.indices.chunks_exact(3) {
            for pair in 0..3 {
                let one = triangle[pair];
                let two = triangle[(pair + 1) % 3];
                let key = if one < two { (one, two) } else { (two, one) };
                *edges.entry(key).or_default() += 1;
            }
        }
        let open: Vec<_> = edges.iter().filter(|(_, count)| **count != 2).collect();
        assert!(
            open.is_empty(),
            "有 {} 条开口边（共 {} 条边）：{:?}",
            open.len(),
            edges.len(),
            &open[..open.len().min(5)]
        );
    }

    #[test]
    fn the_tessellated_circle_closes_on_itself() {
        let circle = px_nurbs_schema::curve::circle(1.0, "xy", [0.0; 3]).expect("造圆");
        let line = curve(&circle, 1e-3, 6, 4).expect("细分");
        assert!(line.vertices() >= 8);
        assert_eq!(line.segments(), line.vertices(), "闭合折线的段数 = 顶点数");
        for vertex in 0..line.vertices() {
            let point = &line.positions[vertex * 3..vertex * 3 + 3];
            let radius = (point[0] * point[0] + point[1] * point[1]).sqrt();
            assert!(
                (radius - 1.0).abs() < 1e-6,
                "第 {vertex} 个顶点离圆心 {radius}（应当是 1）"
            );
            assert!(point[2].abs() < 1e-6, "第 {vertex} 个顶点跑出了 xy 平面");
        }
        for segment in line.indices.chunks_exact(2) {
            assert_ne!(segment[0], segment[1], "有一段是零长线段");
            assert!((segment[0] as usize) < line.vertices());
            assert!((segment[1] as usize) < line.vertices());
        }
        let seam = &line.indices[line.indices.len() - 2..];
        assert_eq!(
            seam,
            &[line.vertices() as u32 - 1, 0],
            "最后一段没有接回第 0 个顶点"
        );
    }

    #[test]
    fn the_polyline_payload_round_trips_bit_for_bit() {
        use px_graph_schema::Build;
        let circle = px_nurbs_schema::curve::circle(1.0, "xy", [0.0; 3]).expect("造圆");
        let line = curve(&circle, 1e-3, 6, 4).expect("细分");
        let bundle = <PolylineData as Build>::encode(&line).expect("编码");
        assert_eq!(bundle.params["vertices"] as usize, line.vertices());
        assert_eq!(bundle.params["segments"] as usize, line.segments());
        let bytes = bundle.to_bytes("nurbs.curve.tessellate").expect("写流");
        let frame = px_graph_schema::PayloadBundle::from_bytes(&bytes).expect("读回清单帧");
        let back = <PolylineData as Build>::decode(&frame, "nurbs.curve.tessellate").expect("解码");
        assert_eq!(back.positions, line.positions);
        assert_eq!(back.indices, line.indices);
    }
}
