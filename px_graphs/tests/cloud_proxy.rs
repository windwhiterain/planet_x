use std::collections::HashMap;

use px_field_schema::field::{Field, Projection};
use px_graph_schema::{Cooked, PxOp};
use px_graphs::cloud_proxy;
use px_mesh_schema::MeshData;
use px_mesh_schema::ops as mesh_ops;
use px_mesh_schema::params as mesh_params;
use px_verify::cloud_field::CloudFieldParams;
use px_volume_schema::VolumeData;
use px_volume_schema::ops as volume_ops;
use px_volume_schema::params::{self as volume_params, Params};

const FACE: u32 = 64;

fn coverage() -> Field {
    let mut field = Field::with_projection(
        FACE,
        FACE * 6,
        vec![0.0_f32; (FACE * FACE * 6) as usize],
        Projection::CubeMap,
    );
    for y in 0..field.height {
        for x in 0..field.width {
            let [cx, cy, cz] = field.direction(x, y);
            let value = 0.5
                + 0.18 * (3.0 * cx).sin() * (2.0 * cy).cos()
                + 0.12 * (5.0 * cz).sin() * (4.0 * cx).cos()
                + 0.08 * (9.0 * cy).sin() * (7.0 * cz).cos();
            field.set(x, y, value.clamp(0.0, 1.0));
        }
    }
    field
}

fn params() -> Params {
    Params {
        res: 33,
        layers: 33,
        ..Params::default()
    }
}

fn surface_params() -> mesh_params::proxy::Params {
    mesh_params::proxy::Params {
        level: 0.0,
        depth: 4,
        weld: 1e-4,
        ..mesh_params::proxy::Params::default()
    }
}

fn bake(params: &Params, coverage: &Field) -> VolumeData {
    let input = volume_ops::CloudCoarseInput {
        coverage: Cooked::new([0; 32], coverage.clone(), false, 0, 0),
    };
    volume_ops::CloudCoarse
        .render(params, &input)
        .expect("烘体积失败")
}

fn surface(params: &mesh_params::proxy::Params, volume: &VolumeData) -> MeshData {
    let input = mesh_ops::ProxyInput {
        volume: Cooked::new([0; 32], volume.clone(), false, 0, 0),
    };
    mesh_ops::Proxy
        .render(params, &input)
        .expect("出等值面失败")
}

fn audit(mesh: &px_mesh_schema::MeshData) -> (usize, usize) {
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for triangle in mesh.indices.chunks_exact(3) {
        for pair in 0..3 {
            let (a, b) = (triangle[pair], triangle[(pair + 1) % 3]);
            let key = if a < b { (a, b) } else { (b, a) };
            *edges.entry(key).or_insert(0) += 1;
        }
    }
    (
        edges.values().filter(|count| **count == 1).count(),
        edges.values().filter(|count| **count > 2).count(),
    )
}

#[test]
fn the_proxy_is_closed() {
    let coverage = coverage();
    let params = params();
    let volume = bake(&params, &coverage);
    let mesh = surface(&surface_params(), &volume);

    let (open, nonmanifold) = audit(&mesh);
    println!(
        "代理：{} 顶点 / {} 三角形｜开口边 {open}、非流形边 {nonmanifold}",
        mesh.vertices(),
        mesh.triangles(),
    );
    assert!(mesh.vertices() > 100, "代理太寒酸了，这个测试没在测东西");
    assert_eq!(
        open, 0,
        "代理有 {open} 条开口边 ⇒ 相邻两面的接缝没焊严（焊不上多半是接缝上两面的方向不一致）"
    );
}

#[test]
fn the_mesh_op_is_reproducible() {
    let coverage = coverage();
    let params = params();
    let volume = bake(&params, &coverage);
    let first = surface(&surface_params(), &volume);
    let second = surface(&surface_params(), &volume);
    assert_eq!(first.positions, second.positions, "顶点位置不可复现");
    assert_eq!(first.normals, second.normals, "法线不可复现");
    assert_eq!(first.uvs, second.uvs);
    assert_eq!(first.indices, second.indices);
}

#[test]
fn the_proxy_encloses_the_coarse_field() {
    let coverage = coverage();
    let params = params();
    let volume = bake(&params, &coverage);
    let mesh = surface(&surface_params(), &volume);

    let cloud: CloudFieldParams = px_verify::proxy::from_volume(&params);
    let report = cloud_proxy::containment(&mesh, &cloud, &coverage, &params, 96, 2048);
    cloud_proxy::print_containment(&report, &params);
    assert!(
        report.rays_with_surface > 20,
        "粗场有交点的方向太少，这个测试没在测东西"
    );
    assert_eq!(
        report.missing, 0,
        "有 {} 条方向粗场有交点、代理一个交点都没有",
        report.missing
    );
    assert!(
        report.worst_slack > -report.worst_cell,
        "最差余量 {:+.6} 超过了一个单元对角线 {:.6}",
        report.worst_slack,
        report.worst_cell,
    );
}

#[test]
fn the_final_field_makes_a_tighter_proxy() {
    let coverage = coverage();
    let coarse_params = params();
    let mut final_params = params();
    final_params.field = volume_params::FieldKind::Final;

    let coarse = bake(&coarse_params, &coverage);
    let final_volume = bake(&final_params, &coverage);
    assert_eq!(coarse.data.len(), final_volume.data.len());

    let mut tighter = 0_usize;
    let mut higher = 0_usize;
    for (one, two) in coarse.data.iter().zip(final_volume.data.iter()) {
        if two < one {
            tighter += 1;
        } else if two > one {
            higher += 1;
        }
    }
    let total = coarse.data.len();
    println!(
        "真场 vs 粗场：{} 个节点里 {} 更低、{} 更高、{} 相同（{:.1}% 更低）",
        total,
        tighter,
        higher,
        total - tighter - higher,
        100.0 * tighter as f64 / total as f64,
    );
    assert_eq!(
        higher, 0,
        "真场有 {higher} 个节点比粗场高 ⇒ 粗场不再包住真场（`billows` 的值越过了它的上界）"
    );
    assert!(
        tighter > total / 4,
        "真场只有 {:.1}% 的节点比粗场低 —— `billows` 没接上，换的其实是同一个场",
        100.0 * tighter as f64 / total as f64,
    );

    let mesh = surface(&surface_params(), &final_volume);
    let (open, nonmanifold) = audit(&mesh);
    println!(
        "真场代理：{} 顶点 / {} 三角形｜开口边 {open}、非流形边 {nonmanifold}",
        mesh.vertices(),
        mesh.triangles(),
    );
    assert_eq!(open, 0, "真场代理有 {open} 条开口边");
}

#[test]
fn the_gradient_bound_is_above_the_measured_gradient() {
    let coverage = coverage();
    let params = params();
    let cloud = px_verify::proxy::from_volume(&params);
    let bound = cloud_proxy::measure_gradient_bound(&cloud, &coverage, &params, 6, 24, 16);
    println!(
        "|∇粗场| ≤ {:.3}（三轴 {:.1} / {:.1} / {:.1}，在面 {} 参数 {:?}）；参数里的 scale = {:.3}",
        bound.bound,
        bound.axes[0],
        bound.axes[1],
        bound.axes[2],
        bound.face,
        bound.at,
        params.scale,
    );
    assert!(
        bound.bound > 1.0,
        "量出来的梯度只有 {:.3}，这个测试没在测东西",
        bound.bound
    );
    assert!(
        bound.bound <= params.scale,
        "量到的 |∇粗场| {:.3} 超过参数里的 scale {:.3}",
        bound.bound,
        params.scale,
    );
}
