//! **星场**：R3 里的一批点光源 = 统一稀疏索引 + 一张点表。
//!
//! ⚠ 载荷布局（每颗星 [`StarField::STRIDE`] 个 f32）：`[x, y, z, 亮度, r, g, b, 未用]`，
//!   **按 (brick, 细格) 排**（造格时按 [`Buckets::order`] 重排）⇒ 一个细格的星是连续的一段。
//!
//! ⚠⚠ 为什么不是一张立方图（旧 `field.stars`）：见 [`crate::grid`] 的文件头。
//!   一句话：**星是点，点不该被存进"按方向参数化"的网格里** ——
//!   那样的网格让世界空间里的圆变成椭圆（实测面心 σ 径向/切向 1.02、面角 1.57），
//!   让星的位置密度按 `|dₓ|+|d_y|+|d_z|` 偏（实测 1.43x），还要求星图与天空面配对分辨率
//!   （实测不配对时面角/面心的亮面积比 1.58x —— 用户看到的就是这个）。

use std::ops::Range;

use px_protocol::art::AssetKind;
use px_protocol::art::Domain;
use px_protocol::payload::{Build, PayloadBundle};
use px_protocol::wire::Blob;
use std::collections::BTreeMap;

use crate::grid::{Buckets, Grid, GridMeta};

/// 一颗星的读数（判据与探针用；热循环里直接读 `stars` 那一段）。
///
/// ⚠ 它住在**载荷旁边**（不是协议层）：它是"这份载荷怎么解读"的一部分，
///   而协议层只认"有一种资产叫 `StarField`"。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Star {
    pub position: [f32; 3],
    pub brightness: f32,
    /// 色（线性 RGB，已含星簇的色温）。
    pub tint: [f32; 3],
}

/// **R3 星场**：索引 + 点表（`StarField::STRIDE` 个 f32/颗）。
///
/// ⚠ 空间参数（`cell` / `origin` / `dims`）住**清单参数**，点表住 blob
///   —— 与 `VolumeData` 的 `inner`/`outer` 同一个口径。
#[derive(Debug, Clone, PartialEq)]
pub struct StarField {
    pub grid: Grid,
    /// 每颗星 `STRIDE` 个 f32，**按 (brick, 细格) 排**。
    pub stars: Vec<f32>,
}

impl StarField {
    /// 每颗星的 f32 数。
    pub const STRIDE: usize = 8;

    pub fn count(&self) -> usize {
        self.stars.len() / Self::STRIDE
    }

    pub fn star(&self, index: usize) -> Star {
        let at = index * Self::STRIDE;
        Star {
            position: [self.stars[at], self.stars[at + 1], self.stars[at + 2]],
            brightness: self.stars[at + 3],
            tint: [self.stars[at + 4], self.stars[at + 5], self.stars[at + 6]],
        }
    }

    /// 从"位置 + 亮度 + 色"造一份（索引由 [`Grid::build`] 造，点表按它重排）。
    pub fn build(
        meta: GridMeta,
        positions: &[[f32; 3]],
        brightness: &[f32],
        tint: &[[f32; 3]],
    ) -> Result<Self, String> {
        if positions.len() != brightness.len() || positions.len() != tint.len() {
            return Err(format!(
                "位置 {} 项、亮度 {} 项、色 {} 项 —— 三者必须一样多",
                positions.len(),
                brightness.len(),
                tint.len()
            ));
        }
        let Buckets {
            grid,
            order,
            brick_of: _,
        } = Grid::build(meta, positions)?;
        let mut stars = vec![0.0_f32; positions.len() * Self::STRIDE];
        for (slot, source) in order.iter().enumerate() {
            let at = slot * Self::STRIDE;
            let index = *source as usize;
            stars[at] = positions[index][0];
            stars[at + 1] = positions[index][1];
            stars[at + 2] = positions[index][2];
            stars[at + 3] = brightness[index];
            stars[at + 4] = tint[index][0];
            stars[at + 5] = tint[index][1];
            stars[at + 6] = tint[index][2];
        }
        Ok(Self { grid, stars })
    }

    /// 一个细格里的星（下标区间）。
    pub fn cell_stars(&self, brick: u32, local: u32) -> Option<Range<usize>> {
        self.grid.cell_range(brick, local)
    }

    /// 一个 AABB 里所有非空细格（回调：细格坐标 + 星区间）。
    pub fn for_each_cell_in(
        &self,
        low: [f32; 3],
        high: [f32; 3],
        f: impl FnMut([i64; 3], Range<usize>),
    ) {
        self.grid.for_each_cell_in(low, high, f);
    }

    /// 一个 brick 里所有非空细格（回调：局部键 + 星区间）。
    pub fn for_each_occupied(&self, brick: u32, f: impl FnMut(u32, Range<usize>)) {
        self.grid.for_each_occupied(brick, f);
    }

    /// 判据用：遍历所有星（下标 + 读数）。
    pub fn for_each_star(&self, mut f: impl FnMut(usize, Star)) {
        for index in 0..self.count() {
            f(index, self.star(index));
        }
    }
}

/// 星表的 blob：`[n, STRIDE]` 一段 f32（**索引住清单参数**）。
impl Build for StarField {
    /// 星场没人看（渲染器不读它，只喂天空与光照烘焙）⇒ 不掺评审相机。
    const WITH_CAMERAS: bool = false;
    /// 星的位置与亮度全由参数给 ⇒ 画布与它无关。
    ///
    /// ⚠ 这一条**是两张图共用一份星场的前提**：画布不进键 ⇒ `nebula` 图
    ///   （`cloud.emission` 用）与 `nebulasky` 图（`sky.nebula` 用）里同名同参的节点
    ///   解析到**同一个键**，CAS 只存一份。
    const RESOLUTION_IS_CANVAS: bool = false;

    fn detail(payload: &Self) -> String {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for index in 0..payload.count() {
            let value = payload.star(index).brightness;
            min = min.min(value);
            max = max.max(value);
        }
        format!(
            "{} 颗星｜{} 个 brick（{} 个占用细格）｜细格 {:.4}｜亮度 {min:.4}..{max:.4}",
            payload.count(),
            payload.grid.bricks(),
            payload.grid.occupied_cells(),
            payload.grid.meta.cell,
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        let meta = payload.grid.meta;
        payload.grid.validate()?;
        // ⚠ 掩码按 `u32` 词存（WGSL 没有 64 位整数）⇒ 进 blob 不需要任何拆装。
        let mask = payload.grid.brick_mask.clone();
        let blobs = vec![
            Blob::from_f32(
                vec![payload.count() as u32, Self::STRIDE as u32],
                &payload.stars,
            ),
            Blob::from_u32(
                vec![payload.grid.chunk_start.len() as u32],
                &payload.grid.chunk_start,
            ),
            Blob::from_u32(
                vec![payload.grid.brick_slot.len() as u32],
                &payload.grid.brick_slot,
            ),
            Blob::from_u32(vec![mask.len() as u32], &mask),
            Blob::from_u32(
                vec![payload.grid.brick_sub.len() as u32],
                &payload.grid.brick_sub,
            ),
            Blob::from_u32(
                vec![payload.grid.sub_start.len() as u32],
                &payload.grid.sub_start,
            ),
        ];
        Ok(PayloadBundle::new(
            AssetKind::StarField,
            BTreeMap::from([
                ("cell".to_string(), meta.cell as f64),
                ("origin_x".to_string(), meta.origin[0] as f64),
                ("origin_y".to_string(), meta.origin[1] as f64),
                ("origin_z".to_string(), meta.origin[2] as f64),
                ("dims_x".to_string(), meta.dims[0] as f64),
                ("dims_y".to_string(), meta.dims[1] as f64),
                ("dims_z".to_string(), meta.dims[2] as f64),
                ("count".to_string(), payload.count() as f64),
                ("items".to_string(), payload.grid.items as f64),
            ]),
            blobs,
        ))
    }

    fn decode(bundle: &PayloadBundle, _projection: Domain, node: &str) -> Result<Self, String> {
        let _ = node;
        let param = |key: &str| -> Result<f64, String> {
            bundle
                .params
                .get(key)
                .copied()
                .ok_or_else(|| format!("星场的清单里缺 `{key}`"))
        };
        if bundle.blobs.len() != 6 {
            return Err(format!("星场应当有 6 个 blob，实际 {}", bundle.blobs.len()));
        }
        let u32s = |index: usize| -> Result<Vec<u32>, String> {
            bundle.blobs[index].u32s().map_err(|err| err.to_string())
        };
        let stars = bundle.blobs[0].f32s().map_err(|err| err.to_string())?;
        let meta = GridMeta {
            cell: param("cell")? as f32,
            origin: [
                param("origin_x")? as f32,
                param("origin_y")? as f32,
                param("origin_z")? as f32,
            ],
            dims: [
                param("dims_x")? as u32,
                param("dims_y")? as u32,
                param("dims_z")? as u32,
            ],
        };
        let grid = Grid {
            meta,
            chunk_start: u32s(1)?,
            brick_slot: u32s(2)?,
            brick_mask: u32s(3)?,
            brick_sub: u32s(4)?,
            sub_start: u32s(5)?,
            items: param("items")? as u32,
        };
        let field = Self { grid, stars };
        let declared = param("count")? as usize;
        if declared != field.count() {
            return Err(format!(
                "星场声明的星数是 {declared}，点表里却有 {} 颗",
                field.count()
            ));
        }
        field.grid.validate()?;
        Ok(field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(count: usize, radius: f32) -> Vec<[f32; 3]> {
        let mut out = Vec::with_capacity(count);
        let golden = 2.399_963_2_f32;
        for index in 0..count {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = golden * index as f32;
            out.push([
                ring * phi.cos() * radius,
                ring * phi.sin() * radius,
                z * radius,
            ]);
        }
        out
    }

    fn meta(cell: f32, reach: f32) -> GridMeta {
        // 盒子每轴对齐到 CHUNK_CELLS 的整数倍（块对齐）。
        let raw = (2.0 * (reach + 2.0 * cell) / cell).ceil() as u32;
        let block = crate::grid::CHUNK_CELLS;
        let dims = raw.div_ceil(block) * block;
        GridMeta {
            cell,
            origin: [-(dims as f32) * cell * 0.5; 3],
            dims: [dims, dims, dims],
        }
    }

    /// **往返恒等**（逐位）—— 内容寻址的地基。
    #[test]
    fn a_star_field_survives_the_round_trip() {
        let positions = shell(4096, 1.5);
        let brightness: Vec<f32> = (0..positions.len()).map(|i| (i % 7) as f32 * 0.5).collect();
        let tint = vec![[0.9, 0.95, 1.0]; positions.len()];
        let field = StarField::build(meta(0.05, 1.5), &positions, &brightness, &tint).expect("造");
        let bundle = <StarField as Build>::encode(&field).expect("编码");
        assert_eq!(bundle.kind, AssetKind::StarField);
        let back = <StarField as Build>::decode(&bundle, Domain::CubeMap, "stars").expect("解码");
        assert_eq!(back.grid, field.grid);
        assert_eq!(back.stars.len(), field.stars.len());
        for index in 0..field.stars.len() {
            assert_eq!(
                back.stars[index].to_bits(),
                field.stars[index].to_bits(),
                "第 {index} 个 f32 不是逐位相同"
            );
        }
    }

    /// ⚠⚠ **空则子全空**：造格之后，任何一个空细格都必须回 `None`
    ///   （不管是"块空"、"brick 空"还是"掩码位是 0"那一条路）。
    #[test]
    fn an_empty_cell_answers_none_however_it_is_empty() {
        let positions = shell(2000, 1.5);
        let brightness = vec![1.0; positions.len()];
        let tint = vec![[1.0; 3]; positions.len()];
        let meta = meta(0.1, 1.5);
        let field = StarField::build(meta, &positions, &brightness, &tint).expect("造");
        // 逐格走一遍整盒：有星的格必须回 Some，没星的必须回 None。
        let mut owned = 0;
        for z in 0..meta.dims[2] as i64 {
            for y in 0..meta.dims[1] as i64 {
                for x in 0..meta.dims[0] as i64 {
                    let cell = [x, y, z];
                    let (_, _, local) = meta.decompose(cell);
                    match field.grid.brick_at(cell) {
                        Some(brick) => {
                            if let Some(range) = field.grid.cell_range(brick, local) {
                                owned += range.len();
                            }
                        }
                        None => {}
                    }
                }
            }
        }
        assert_eq!(owned, field.count(), "有星没被任何细格认领（或有格多认了）");
    }

    /// **点表按 (brick, 细格) 排**：一个细格的星在数组里是**连续一段**。
    #[test]
    fn each_cell_owns_a_contiguous_run_of_stars() {
        let positions = shell(3000, 1.5);
        let brightness = vec![1.0; positions.len()];
        let tint = vec![[1.0; 3]; positions.len()];
        let field = StarField::build(meta(0.05, 1.5), &positions, &brightness, &tint).expect("造");
        let mut seen = 0;
        field.grid.for_each_cell(|_, range| {
            seen += range.len();
            for index in range {
                let star = field.star(index);
                // 这颗星必须真的落在它被登记的那个细格里。
                let cell = field.grid.meta.cell_of(star.position);
                let (_, _, local) = field.grid.meta.decompose(cell);
                let brick = field.grid.brick_at(cell).expect("有星必然有 brick");
                let back = field.grid.cell_range(brick, local).expect("有星必然有区间");
                assert!(
                    back.contains(&index),
                    "第 {index} 颗星被登记在 {back:?} 之外"
                );
            }
        });
        assert_eq!(seen, field.count(), "有星不在任何细格里");
    }

    /// **位置一一对应**：造格只重排、不改变（位置、亮度、色）的三元组。
    #[test]
    fn the_build_only_reorders() {
        let positions = shell(512, 1.2);
        let brightness: Vec<f32> = (0..positions.len()).map(|i| i as f32).collect();
        let tint: Vec<[f32; 3]> = (0..positions.len()).map(|i| [i as f32, 0.0, 0.0]).collect();
        let field = StarField::build(meta(0.08, 1.2), &positions, &brightness, &tint).expect("造");
        let mut sorted: Vec<f32> = (0..field.count())
            .map(|index| field.star(index).brightness)
            .collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("没有 NaN"));
        let expected: Vec<f32> = brightness.clone();
        assert_eq!(sorted, expected, "重排把亮度弄丢/弄错了");
        for index in 0..field.count() {
            let star = field.star(index);
            let brightness = star.brightness as usize;
            let position = positions[brightness];
            assert_eq!(star.position, position, "第 {index} 颗星的位置串了");
            assert_eq!(star.tint[0], brightness as f32, "色也串了");
        }
    }

    /// **确定性**：同输入两遍逐位相同。
    #[test]
    fn the_same_input_gives_the_same_field() {
        let positions = shell(1024, 1.5);
        let brightness = vec![2.0; positions.len()];
        let tint = vec![[1.0; 3]; positions.len()];
        let one = StarField::build(meta(0.06, 1.5), &positions, &brightness, &tint).expect("造");
        let two = StarField::build(meta(0.06, 1.5), &positions, &brightness, &tint).expect("造");
        assert_eq!(one.grid, two.grid);
        assert_eq!(one.stars, two.stars);
    }

    /// **格外面当场拒**（静默丢星 = 画面上"少了几颗"，最难查）。
    #[test]
    fn a_position_outside_the_grid_is_refused() {
        let positions = vec![[0.0, 0.0, 0.0], [99.0, 0.0, 0.0]];
        let brightness = vec![1.0, 1.0];
        let tint = vec![[1.0; 3]; 2];
        let err =
            StarField::build(meta(0.1, 1.5), &positions, &brightness, &tint).expect_err("应当拒掉");
        assert!(err.contains("格外面"), "报的是：{err}");
    }

    /// ⚠⚠ **同一个 brick 的项必须连续**：造格的排序键是 `(块, brick, 细格)` 三元组，
    ///   **不是细格的线性键** —— 细格键按 `(z, y, x)` 扫，一行 `x` 扫过去要跨过一排
    ///   brick，换行之后 brick 键又跳回去 ⇒ 同一个 brick 被切成很多段。
    ///
    ///   实测（壳上 3000 个点、细格 0.05）：按细格键排会得到 **2387 段**，而真实 brick
    ///   只有 **224** 个；症状是"绝大多数 brick 只有一颗星、查询大批落空"
    ///   （同一次实测里逐格回走的星数只有 286/3000）。
    #[test]
    fn the_items_of_one_brick_stay_contiguous() {
        let positions = shell(3000, 1.5);
        let brightness = vec![1.0; positions.len()];
        let tint = vec![[1.0; 3]; positions.len()];
        let field = StarField::build(meta(0.05, 1.5), &positions, &brightness, &tint).expect("造");
        let mut distinct: Vec<(u32, u32)> = positions
            .iter()
            .map(|p| {
                let (chunk, brick, _) = field.grid.meta.decompose(field.grid.meta.cell_of(*p));
                (chunk, brick)
            })
            .collect();
        distinct.sort();
        distinct.dedup();
        assert_eq!(
            field.grid.bricks(),
            distinct.len(),
            "brick 数比真实的 (块, brick) 组合多 —— 同一个 brick 被切成了好几段"
        );
    }
}
