//! 硬表面外接代理的端到端判据：闭合、包住、可复现。
//!
//! 覆盖图是**造**出来的一张小 cube map（不是 `mixed` 那句真场），这样测试不依赖 CAS 里
//! 有没有烘过 `clouds` 图；真数据上的同一条断言在 `px_graphs --bin clouds` 的 `check` 里跑。
//!
//! ⚠ 这里**静态**调算子的函数体（`px_volume_op` / `px_mesh_op` 是 dev-dependency）：判的是数值。
//! 「算子真的从 dylib 里被装载、被调用」由 `px_graphs/src/bin/*` 那一趟端到端走 —— 它的
//! 产物键就是判据（键相同 ⇒ 走的是同一条路）。

use std::collections::HashMap;

use px_field_schema::field::{Field, Projection};
use px_field_schema::noise::FbmSettings;
use px_field_op::noise::fbm_3;
use px_mesh_schema::params as mesh_params;
use px_graphs::cloud_proxy;
use px_volume_schema::VolumeGrid;
use px_volume_schema::params::{self as volume_params, Params};
use px_verify::cloud_field::CloudFieldParams;

const FACE: u32 = 64;

fn coverage() -> Field {
    let settings = FbmSettings {
        frequency: 2.0,
        octaves: 3,
        lacunarity: 2.0,
        gain: 0.5,
        seed: 11,
    };
    let mut field = Field::with_projection(
        FACE,
        FACE * 6,
        vec![0.0_f32; (FACE * FACE * 6) as usize],
        Projection::CubeMap,
    );
    for y in 0..field.height {
        for x in 0..field.width {
            let direction = field.direction(x, y);
            field.set(x, y, fbm_3(direction, &settings));
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
    // `offset`（P20 待删的死码）没写在这里 ⇒ 走它的默认 0.0：老路径逐位不变。
    mesh_params::proxy::Params {
        level: 0.0,
        depth: 4,
        weld: 1e-4,
        ..mesh_params::proxy::Params::default()
    }
}

fn bake(params: &Params, coverage: &Field) -> px_volume_schema::VolumeData {
    px_volume_op::bake(params, coverage)
}

fn surface(params: &mesh_params::proxy::Params, volume: &px_volume_schema::VolumeData) -> px_mesh_schema::MeshData {
    let grid = VolumeGrid::new(volume);
    px_mesh_op::proxy::surface(params, &grid).expect("出等值面失败")
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
    // 缓存是「键 = 内容」：同一份参数跑两次必须逐位一样。当初换掉自适应哈希八叉树就是因为
    // 它拿 HashMap 的遍历序决定顶点位置（同一进程里跑两次都有 8% 的顶点不同）。
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
    assert!(report.rays_with_surface > 20, "粗场有交点的方向太少，这个测试没在测东西");
    assert_eq!(report.missing, 0, "有 {} 条方向粗场有交点、代理一个交点都没有", report.missing);
    assert!(
        report.worst_slack > -report.worst_cell,
        "最差余量 {:+.6} 超过了一个单元对角线 {:.6}",
        report.worst_slack,
        report.worst_cell,
    );
}

#[test]
fn the_final_field_makes_a_tighter_proxy() {
    // 换 `field` 的全部意义：真场 ≤ 粗场 ⇒ 真场的等值面在粗场里面。
    // 落点变近靠的就是这一条，所以「每个节点的值都不超过粗场」是可直接断言的充分条件。
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
    // 载重的那一条是**上界**：真场 = `shape_of(cover, altitude, √b)` 而粗场取噪声上界 1.0，
    // 所以一个节点都不许比粗场高。细节噪声加了 `sqrt` 之后 `√b` 变大 ⇒ `lobed` 更容易饱和到
    // 1（= 与粗场逐位相同的那批），"更低"的占比从原来的 >50% 掉到 ~47% —— 所以下面那条
    // 只当"不是同一个场"的非空判据，别拿它当上界用。
    assert_eq!(
        higher, 0,
        "真场有 {higher} 个节点比粗场高 ⇒ 粗场不再包住真场（`billows` 的值越过了它的上界）"
    );
    assert!(
        tighter > total / 4,
        "真场只有 {:.1}% 的节点比粗场低 —— `billows` 没接上，换的其实是同一个场",
        100.0 * tighter as f64 / total as f64,
    );

    // 网格也必须照样闭合：缺几何是硬失败。
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
    // L 只许大不许小：小了自适应八叉树那类提取器的剪枝测试就会漏（网格出洞）。
    // 量的是一张稀疏网格（几分钟内跑完），真数据上的同一断言在 bin 里（--bound 更密）。
    let coverage = coverage();
    let params = params();
    let cloud = px_verify::proxy::from_volume(&params);
    let bound = cloud_proxy::measure_gradient_bound(&cloud, &coverage, &params, 6, 24, 16);
    println!(
        "|∇粗场| ≤ {:.3}（三轴 {:.1} / {:.1} / {:.1}，在面 {} 参数 {:?}）；参数里的 scale = {:.3}",
        bound.bound, bound.axes[0], bound.axes[1], bound.axes[2], bound.face, bound.at, params.scale,
    );
    assert!(bound.bound > 1.0, "量出来的梯度只有 {:.3}，这个测试没在测东西", bound.bound);
    assert!(
        bound.bound <= params.scale,
        "量到的 |∇粗场| {:.3} 超过参数里的 scale {:.3}",
        bound.bound,
        params.scale,
    );
}
