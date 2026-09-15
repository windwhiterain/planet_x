use px_ops::field::{Field, Projection};
use px_ops::{PATCHES, VolumeData, VolumeGrid, VolumeSampler, direction_of, point_of};

fn volume(res: u32, layers: u32) -> VolumeData {
    let mut data = Vec::with_capacity((PATCHES * res * res * layers) as usize);
    for face in 0..PATCHES {
        for layer in 0..layers {
            for t in 0..res {
                for s in 0..res {
                    data.push(
                        face as f32 * 100.0
                            + layer as f32 * 10.0
                            + t as f32
                            + s as f32 / 1000.0,
                    );
                }
            }
        }
    }
    VolumeData {
        res,
        layers,
        inner: 1.01,
        outer: 1.06,
        data,
    }
}

#[test]
fn the_volume_blob_round_trips() {
    let volume = volume(9, 5);
    let blobs = volume.blobs();
    let restored = VolumeData::from_blob(&blobs[0]).expect("往返失败");
    assert_eq!(restored.res, volume.res);
    assert_eq!(restored.layers, volume.layers);
    assert_eq!(restored.data, volume.data);
    assert_eq!(restored.samples(), volume.samples());
    // 壳半径住在清单参数里，不在载荷里 —— 这里只保证载荷那部分无损。
    assert_eq!(restored.inner, 0.0);
}

#[test]
fn the_grid_sampler_hits_the_nodes_exactly() {
    // res/layers 都是 2 的幂 + 1 ⇒ 节点坐标在 f32 里是精确的，三线性采样必须逐位落回原值。
    let volume = volume(9, 5);
    let grid = VolumeGrid::new(&volume);
    for face in 0..PATCHES {
        for layer in 0..volume.layers {
            for t in 0..volume.res {
                for s in 0..volume.res {
                    let at = [
                        s as f32 / (volume.res - 1) as f32,
                        t as f32 / (volume.res - 1) as f32,
                        layer as f32 / (volume.layers - 1) as f32,
                    ];
                    assert_eq!(
                        grid.sample(face, at),
                        volume.at(face, layer, t, s),
                        "面 {face} 节点 ({s},{t},{layer}) 上没有落回原值"
                    );
                }
            }
        }
    }
}

#[test]
fn the_cube_sphere_faces_share_their_edges() {
    // 代理能不能焊成闭合的一张，全押在这一条上：相邻两面在共用的那条棱上给出**同一个**
    // 方向 ⇒ 两边的采样点、场值、交点位置都一样，按位置焊就能焊上。
    //
    // 只保证到浮点级（不保证逐位：两条公式算同一个方向时最后一位可能差 1 ULP），
    // 所以判据是「每条棱内点恰好有一个搭档，且两者相差远小于焊缝容差 1e-4」。
    let samples = [0.25_f32, 0.5, 0.75];
    let mut edges: Vec<[f32; 3]> = Vec::new();
    for face in 0..PATCHES {
        for value in samples {
            edges.push(direction_of(face, 0.0, value));
            edges.push(direction_of(face, 1.0, value));
            edges.push(direction_of(face, value, 0.0));
            edges.push(direction_of(face, value, 1.0));
        }
    }

    let mut worst = 0.0_f32;
    for (index, here) in edges.iter().enumerate() {
        let mut partners = Vec::new();
        for (other, there) in edges.iter().enumerate() {
            if other == index {
                continue;
            }
            let far = (0..3)
                .map(|axis| (here[axis] - there[axis]).abs())
                .fold(0.0_f32, f32::max);
            if far < 1e-6 {
                partners.push(far);
            }
        }
        assert_eq!(
            partners.len(),
            1,
            "棱上的方向 {here:?} 有 {} 个搭档（应当恰好一个：相邻面在同一条棱上）",
            partners.len(),
        );
        worst = worst.max(partners[0]);
    }
    println!(
        "{} 个棱内点各自恰好一个搭档，最大出入 {worst:e}（焊缝容差 1e-4）",
        edges.len()
    );
    assert!(worst < 1e-6, "棱上两面的方向差到了 {worst:e}");
}

#[test]
fn a_face_parameter_maps_to_the_shell() {
    let direction = direction_of(0, 0.5, 0.5);
    let low = point_of(0, [0.5, 0.5, 0.0], 1.0, 2.0);
    let high = point_of(0, [0.5, 0.5, 1.0], 1.0, 2.0);
    assert!((low[0] - direction[0]).abs() < 1e-6);
    assert!((high[0] - direction[0] * 2.0).abs() < 1e-6);
    assert!((high[1] - direction[1] * 2.0).abs() < 1e-6);
}

#[test]
fn a_field_can_hold_a_cube_map_mask() {
    // 判据仪器要拿一张覆盖图当输入；这里只确认 CubeMap 域的取方向是通的。
    let field = Field::with_projection(4, 24, vec![0.0; 96], Projection::CubeMap);
    assert_eq!(field.direction(0, 0).len(), 3);
    assert_eq!(field.sample_direction([0.0, 1.0, 0.0]), 0.0);
}
