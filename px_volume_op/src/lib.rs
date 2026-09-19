//! **体积域**的算子：粗场 → 3D 标量网格（立方球参数空间）。
//!
//! `field = "coarse"`（默认）：细节场在 shader 里是乘上去的（`shape_of(cover, altitude, billows)`），
//! 而 `billows ∈ [0,1]` ⇒ `final ≤ coarse`。所以烘**粗场** `shape_of(cover, altitude, 1.0)` 的
//! 等值面就天然包住最终等值面，不需要任何经验外扩；代价是落点离真表面 40+ 步。
//! `field = "final"`：烘 shader `cloud_field` 那个量 ⇒ 落点贴近真表面，但不再包住（判据换成像素）。
//!
//! 参照场住在 `px_verify::proxy`（判据仪器也用它）——这里只负责撒点、算场、存网格。

pub mod typed;

use px_cook::field_fn::{FieldFn, SampleField};
use px_field_schema::field::{Field, tangent_frame};
use px_volume_schema::params::{self, FieldKind};
use px_volume_schema::{PATCHES, VolumeData, direction_of};
use px_verify::proxy;

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

/// 烘一份体积。⚠ `cover` 是**泛型方法**：算子只要求"给一个方向、回一个覆盖度"，
/// 至于那个覆盖度是从哪儿来的，这里一概不管。
///
/// ⚠ 现在唯一的调用点是 [`eval_sampled`]（上游那张采样场）。从前还有图脚本现写的
/// 闭式场那一档 —— 它靠"生成的单态化实例 + 动态装载"，那条路已删（原型期决定）。
pub fn bake<F: FieldFn>(params: &params::Params, cover: &F) -> VolumeData {
    let cloud = proxy::from_volume(params);
    let at = |direction: [f32; 3]| cover.cover(&cloud, direction);
    let res = params.res.max(2);
    let layers = params.layers.max(2);
    let inv_scale = 1.0 / if params.scale > 0.0 { params.scale } else { 1.0 };

    // 每个节点的覆盖度取**切向邻域**上的最大值（保守化）。邻域撒在**方向**上：
    // 面内参数是各面自己的，两个面在接缝上的同一个节点用参数撒邻域会撒出两组不同的
    // 方向 ⇒ 两边的最大值不一样 ⇒ 接缝又焊不上了（实测 2973 条开口边）。按方向撒就没有
    // 这个问题：同一个方向算出来的是同一组邻居、同一个最大值。
    let steps = 4 * params.reach as i64;
    let radius = params.reach as f32 * proxy::tangential_cell(params);
    let mut node_cover = vec![0.0_f32; (PATCHES * res * res) as usize];
    for face in 0..PATCHES {
        for t in 0..res {
            for s in 0..res {
                let direction = direction_of(
                    face,
                    s as f32 / (res - 1) as f32,
                    t as f32 / (res - 1) as f32,
                );
                let mut best = at(direction);
                if radius > 0.0 {
                    let (east, north) = tangent_frame(direction);
                    for far in -steps..=steps {
                        for side in -steps..=steps {
                            if side * side + far * far > steps * steps {
                                continue;
                            }
                            let across = radius * side as f32 / steps as f32;
                            let along = radius * far as f32 / steps as f32;
                            let neighbour = normalize([
                                direction[0] + east[0] * across + north[0] * along,
                                direction[1] + east[1] * across + north[1] * along,
                                direction[2] + east[2] * across + north[2] * along,
                            ]);
                            best = best.max(at(neighbour));
                        }
                    }
                }
                node_cover[((face * res + t) * res + s) as usize] = best;
            }
        }
    }

    let mut data = vec![0.0_f32; (PATCHES * layers * res * res) as usize];
    for face in 0..PATCHES {
        for layer in 0..layers {
            let altitude = layer as f32 / (layers - 1) as f32;
            for t in 0..res {
                for s in 0..res {
                    let cover = node_cover[((face * res + t) * res + s) as usize];
                    // 真场要在节点自己的方向上取噪声；粗场不需要 ⇒ 老路径一位没动。
                    let value = match params.field {
                        FieldKind::Coarse => proxy::coarse_at(&cloud, cover, altitude),
                        FieldKind::Final => {
                            let direction = direction_of(
                                face,
                                s as f32 / (res - 1) as f32,
                                t as f32 / (res - 1) as f32,
                            );
                            proxy::final_at(&cloud, cover, direction, altitude)
                        }
                    };
                    let slot = (((face * layers + layer) * res + t) * res + s) as usize;
                    data[slot] = (value - params.tau) * inv_scale;
                }
            }
        }
        // 径向也做一次最大值滤波：`shape` 在壳的上下之间不是单调的，
        // 逐层取最大才是膨胀（也才会把内表面往里推、外表面往外推）。
        //
        // ⚠ 但**两头那两层不许动**：壳的上下壁上场是 0（负），一动就变正，
        // 等值面就会切在域壁上 ⇒ 代理开口（实测 3027 条开口边）。所以滤波只覆盖内层，
        // 壁上仍是原值，等值面必然留在域内。
        let reach = params.reach as usize;
        if reach > 0 && layers as usize > 2 * reach + 1 {
            let mut filtered = vec![0.0_f32; (layers * res * res) as usize];
            for layer in 0..layers as usize {
                let low = layer.saturating_sub(reach);
                let high = (layer + reach).min(layers as usize - 1);
                for t in 0..res as usize {
                    for s in 0..res as usize {
                        let mut best = f32::MIN;
                        for other in low..=high {
                            let slot = (((face * layers + other as u32) * res + t as u32) * res
                                + s as u32) as usize;
                            best = best.max(data[slot]);
                        }
                        filtered[(layer * res as usize + t) * res as usize + s] = best;
                    }
                }
            }
            for layer in 1..layers as usize - 1 {
                for t in 0..res as usize {
                    for s in 0..res as usize {
                        let slot = (((face * layers + layer as u32) * res + t as u32) * res
                            + s as u32) as usize;
                        data[slot] = filtered[(layer * res as usize + t) * res as usize + s];
                    }
                }
            }
        }
    }
    VolumeData {
        res,
        layers,
        inner: params.inner,
        outer: params.outer,
        data,
    }
}

/// 老路径的入口：上游那张**采样好的场**。
///
/// ⚠ 它就是 `bake<SampleField>`，单独留一个名字是因为它是 dylib 那一半与图脚本
/// 都常用的那一档；`SampleField` 那层包装保证与 `proxy::cover_at` 逐步同一件事。
pub fn eval_sampled(params: &params::Params, coverage: &Field) -> VolumeData {
    bake(params, &SampleField { field: coverage })
}

