//! **实测差异**：两张 PNG 逐像素比，并把差异**归因到区域**上。
//!
//! 为什么不是哈希：切片 1 故意只画了"背景 + 行星"，而 oracle 那张图上还有星空（切片 2）
//! 与大气（切片 3）。哈希只会说"不一样"，那对定位一个像素级的错**一文不值**；
//! 这一篇要回答的是"**差在哪、差多少、行星自己的像素动没动**"。
//!
//! 口径（每一条都会改变读数，所以都写死在这里）：
//!
//! - 比的是 **8 位 sRGB 通道**（`to_rgb8()`，丢掉 alpha）—— 与 `shot::write_png` 落盘的那一份
//!   同源，也就是"人眼看到的那张图"；
//! - **背景色**取**本宿主这张图**里出现次数最多的那个三元组。切片 1 的图按构造只有两种东西：
//!   清屏色与行星 ⇒ "不等于背景色"的那些像素**就是行星的剪影**。这是本切片自己的定义，
//!   不是从 oracle 猜出来的；
//! - 所有"里面 / 外面"的读数都相对**这个剪影**说，而且**两个方向都报**：
//!   剪影内的差异（那才是"行星画错了"）与剪影外的差异（星空与大气）。
//!
//! ⚠ 这一篇**不是**判据：它什么都不判，只报数。判据是报告里那几行数本身。

use std::path::Path;

/// 一张图的样子：宽、高、紧凑的 RGB8。
struct Bitmap {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl Bitmap {
    fn load(path: &Path) -> Result<Bitmap, String> {
        let bytes = std::fs::read(path).map_err(|err| format!("读 {} 失败：{err}", path.display()))?;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|err| format!("{} 不是一张读得动的 PNG：{err}", path.display()))?;
        let rgb = image.to_rgb8();
        let (width, height) = rgb.dimensions();
        Ok(Bitmap {
            width,
            height,
            rgb: rgb.into_raw(),
        })
    }

    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * self.width + x) * 3) as usize;
        [self.rgb[at], self.rgb[at + 1], self.rgb[at + 2]]
    }
}

/// 一个区域的读数。
#[derive(Clone, Copy, Debug, Default)]
pub struct Region {
    pub pixels: usize,
    pub max_delta: u32,
    pub sum_delta: u64,
    pub channels: u64,
    pub bbox: Option<(u32, u32, u32, u32)>,
}

impl Region {
    fn add(&mut self, x: u32, y: u32, delta: u32) {
        self.pixels += 1;
        self.max_delta = self.max_delta.max(delta);
        self.sum_delta += u64::from(delta);
        self.channels += 3;
        self.bbox = Some(match self.bbox {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }

    /// 平均**通道**差（`sum_delta / (3 × 像素)`）—— 与报告里那个口径同一个。
    pub fn mean_delta(&self) -> f64 {
        if self.channels == 0 {
            0.0
        } else {
            self.sum_delta as f64 / self.channels as f64
        }
    }

    pub fn bbox_text(&self) -> String {
        match self.bbox {
            Some((x0, y0, x1, y1)) => format!("({x0},{y0})-({x1},{y1})"),
            None => "（没有像素）".to_string(),
        }
    }
}

/// 一份差异报告。**每一个数都只来自这两张图**。
pub struct Diff {
    pub width: u32,
    pub height: u32,
    /// 本宿主那张图的背景色（出现次数最多的三元组）。
    pub background: [u8; 3],
    pub background_pixels: usize,
    /// oracle 那张图自己的众数（"它的背景色"）与它的像素数 —— 用来回答"清屏色一样吗"。
    pub oracle_background: [u8; 3],
    pub oracle_background_pixels: usize,
    /// 剪影：本宿主这张图里**不等于背景色**的像素（切片 1 里它就是行星自己）。
    pub silhouette: Region,
    /// 全部差异像素。
    pub differing: Region,
    /// 差异 ∩ 剪影（"行星自己的像素动了没有"）。
    pub inside: Region,
    /// 差异 \ 剪影（星空与大气的地盘）。
    pub outside: Region,
    /// 剪影的等效半径（`sqrt(面积/π)`）与质心 —— 用来把"外面那些差异"说成"离边缘多远"。
    pub silhouette_radius: f64,
    pub center: (f64, f64),
    /// 剪影外差异像素的**半径分布**（以等效半径为单位）：落在 [1.0,1.15) / [1.15,∞) 的个数。
    pub outside_ring: usize,
    pub outside_far: usize,
    pub outside_min_radius: f64,
    pub outside_max_radius: f64,
    /// 差异像素按半径分成四带（以剪影等效半径为单位）：盘内 / 边缘 / 环上 / 更远。
    pub band_inner: usize,
    pub band_limb: usize,
    pub band_ring: usize,
    pub band_far: usize,
    /// 剪影内**亮度**的皮尔逊相关系数：行星自己那一片的"花纹"在两张图里是不是同一片。
    ///
    /// ⚠ 它**不是**逐位判据（逐位那一条看 `inside.pixels == 0`）。它能回答的是另一个问题：
    /// 差异是"同一片花纹上盖了一层东西"（相关系数接近 1），还是"花纹本身就不一样"
    /// （相关系数掉下来）—— 前者是叠了别的东西，后者是这颗行星画错了。
    pub inside_luma_correlation: f64,
    /// 剪影内**局部对比**（横向 lag-2 差分）的相关系数。
    ///
    /// 为什么还要这一条：一层**随位置变强弱的**平滑覆盖（大气的面纱就是这样）会把亮度相关
    /// 拉下来，却几乎不动"细节"。所以"花纹一样"这件事要看**高通**那一份：同一片花纹 ⇒
    /// 局部对比几乎逐位对齐；花纹换了（UV / 法线 / 贴图错了）⇒ 它当场掉下来。
    pub inside_contrast_correlation: f64,
}

/// 两张图比一遍。尺寸不同直接拒（"尺寸不同"不是一种差异，是两份不同的东西）。
pub fn compare(ours: &Path, oracle: &Path) -> Result<Diff, String> {
    let left = Bitmap::load(ours).map_err(|err| format!("本宿主那张：{err}"))?;
    let right = Bitmap::load(oracle).map_err(|err| format!("oracle 那张：{err}"))?;
    if (left.width, left.height) != (right.width, right.height) {
        return Err(format!(
            "两张图尺寸不同：{} 是 {}×{}，{} 是 {}×{} —— 这不是一种差异，是两份不同的东西",
            ours.display(),
            left.width,
            left.height,
            oracle.display(),
            right.width,
            right.height
        ));
    }

    // 背景色 = 本宿主这张图里的众数。切片 1 的图只有背景与行星 ⇒ 它就是多数派。
    let mut counts: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
    for y in 0..left.height {
        for x in 0..left.width {
            *counts.entry(left.at(x, y)).or_insert(0) += 1;
        }
    }
    let (background, background_pixels) = counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .ok_or_else(|| "本宿主那张图一个像素都没有".to_string())?;

    let mut silhouette = Region::default();
    let mut differing = Region::default();
    let mut inside = Region::default();
    let mut outside = Region::default();
    let mut sum_x = 0.0_f64;
    let mut sum_y = 0.0_f64;
    // oracle 那张图自己的众数：用来回答"我们的背景色与它的背景色是不是同一个"。
    // ⚠ 它**不参与**剪影的定义（剪影只按本宿主那张图定），只是另一条读数。
    let mut oracle_counts: std::collections::HashMap<[u8; 3], usize> =
        std::collections::HashMap::new();
    let mut oracle_background_pixels = 0_usize;
    for y in 0..left.height {
        for x in 0..left.width {
            let mine = left.at(x, y);
            let theirs = right.at(x, y);
            *oracle_counts.entry(theirs).or_insert(0) += 1;
            if mine != background {
                silhouette.add(x, y, 0);
                sum_x += f64::from(x);
                sum_y += f64::from(y);
            }
            if mine != theirs {
                let delta = (0..3)
                    .map(|channel| u32::from(mine[channel].abs_diff(theirs[channel])))
                    .max()
                    .unwrap_or(0);
                differing.add(x, y, delta);
                if mine != background {
                    inside.add(x, y, delta);
                } else {
                    outside.add(x, y, delta);
                }
            }
        }
    }
    let (oracle_background, _) = oracle_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .unwrap_or(([0, 0, 0], 0));
    for y in 0..left.height {
        for x in 0..left.width {
            if right.at(x, y) == oracle_background {
                oracle_background_pixels += 1;
            }
        }
    }

    let radius = (silhouette.pixels as f64 / std::f64::consts::PI).sqrt();
    let center = if silhouette.pixels == 0 {
        (0.0, 0.0)
    } else {
        (
            sum_x / silhouette.pixels as f64,
            sum_y / silhouette.pixels as f64,
        )
    };

    // 剪影外的差异离边缘多远（以等效半径为单位）：星空在很远，大气的环贴着边缘。
    let mut ring = 0;
    let mut far = 0;
    let mut min_radius = f64::INFINITY;
    let mut max_radius = 0.0_f64;
    // 四带 + 剪影内亮度的相关（两趟一起走：半径要先知道才谈得上分带）。
    let (mut band_inner, mut band_limb, mut band_ring, mut band_far) = (0, 0, 0, 0);
    // 相关系数用**整数累加器**：亮度和、平方和、乘积和都是整数，避免了浮点求和的次序问题。
    let (mut count, mut sum_mine, mut sum_theirs, mut sum_sq_mine, mut sum_sq_theirs, mut sum_cross) =
        (0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
    for y in 0..left.height {
        for x in 0..left.width {
            let mine = left.at(x, y);
            let theirs = right.at(x, y);
            let dx = f64::from(x) - center.0;
            let dy = f64::from(y) - center.1;
            let distance = (dx * dx + dy * dy).sqrt() / radius;
            if mine != background {
                // 亮度用整数权重（Rec.601 的近似，四舍五入到整数）：相关系数对权重不敏感，
                // 而整数让它与"谁先谁后"无关。
                let luma = |pixel: [u8; 3]| -> i64 {
                    (299 * i64::from(pixel[0]) + 587 * i64::from(pixel[1]) + 114 * i64::from(pixel[2]))
                        / 1000
                };
                let one = luma(mine);
                let other = luma(theirs);
                count += 1;
                sum_mine += one;
                sum_theirs += other;
                sum_sq_mine += one * one;
                sum_sq_theirs += other * other;
                sum_cross += one * other;
            }
            if mine == theirs {
                continue;
            }
            if distance < 0.95 {
                band_inner += 1;
            } else if distance < 1.0 {
                band_limb += 1;
            } else if distance < 1.15 {
                band_ring += 1;
            } else {
                band_far += 1;
            }
            if mine != background {
                continue;
            }
            min_radius = min_radius.min(distance);
            max_radius = max_radius.max(distance);
            if distance < 1.15 {
                ring += 1;
            } else {
                far += 1;
            }
        }
    }
    let correlation = {
        let n = count as f64;
        let covariance = n * sum_cross as f64 - sum_mine as f64 * sum_theirs as f64;
        let spread_mine = n * sum_sq_mine as f64 - (sum_mine * sum_mine) as f64;
        let spread_theirs = n * sum_sq_theirs as f64 - (sum_theirs * sum_theirs) as f64;
        if spread_mine <= 0.0 || spread_theirs <= 0.0 {
            f64::NAN
        } else {
            covariance / (spread_mine.sqrt() * spread_theirs.sqrt())
        }
    };

    // 高通那一份：横向 lag-2 差分（同一行的 `x` 与 `x+2` 都在剪影里才算）。
    let (mut count, mut sum_mine, mut sum_theirs, mut sum_sq_mine, mut sum_sq_theirs, mut sum_cross) =
        (0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
    for y in 0..left.height {
        for x in 0..left.width.saturating_sub(2) {
            if left.at(x, y) == background || left.at(x + 2, y) == background {
                continue;
            }
            let luma = |pixel: [u8; 3]| -> i64 {
                (299 * i64::from(pixel[0]) + 587 * i64::from(pixel[1]) + 114 * i64::from(pixel[2]))
                    / 1000
            };
            let one = luma(left.at(x + 2, y)) - luma(left.at(x, y));
            let other = luma(right.at(x + 2, y)) - luma(right.at(x, y));
            count += 1;
            sum_mine += one;
            sum_theirs += other;
            sum_sq_mine += one * one;
            sum_sq_theirs += other * other;
            sum_cross += one * other;
        }
    }
    let contrast_correlation = {
        let n = count as f64;
        let covariance = n * sum_cross as f64 - sum_mine as f64 * sum_theirs as f64;
        let spread_mine = n * sum_sq_mine as f64 - (sum_mine * sum_mine) as f64;
        let spread_theirs = n * sum_sq_theirs as f64 - (sum_theirs * sum_theirs) as f64;
        if spread_mine <= 0.0 || spread_theirs <= 0.0 {
            f64::NAN
        } else {
            covariance / (spread_mine.sqrt() * spread_theirs.sqrt())
        }
    };

    Ok(Diff {
        width: left.width,
        height: left.height,
        background,
        background_pixels,
        oracle_background,
        oracle_background_pixels,
        silhouette,
        differing,
        inside,
        outside,
        silhouette_radius: radius,
        center,
        outside_ring: ring,
        outside_far: far,
        outside_min_radius: if min_radius.is_finite() { min_radius } else { 0.0 },
        outside_max_radius: max_radius,
        band_inner,
        band_limb,
        band_ring,
        band_far,
        inside_luma_correlation: correlation,
        inside_contrast_correlation: contrast_correlation,
    })
}

impl Diff {
    /// 一行一个数的报告。⚠ 这里**不下结论**（"对上了"不是这一篇的事），只把读数列全。
    pub fn report(&self) -> String {
        let total = self.width as usize * self.height as usize;
        let percent = |count: usize| 100.0 * count as f64 / total as f64;
        let mut lines = vec![
            format!("尺寸：{}×{}（{} 像素）", self.width, self.height, total),
            format!(
                "本宿主那张图的背景色：{:?}（{} 像素，{:.2}%）—— 剪影按它定义",
                self.background,
                self.background_pixels,
                percent(self.background_pixels)
            ),
            format!(
                "oracle 那张图的众数色：{:?}（{} 像素，{:.2}%）{}",
                self.oracle_background,
                self.oracle_background_pixels,
                percent(self.oracle_background_pixels),
                if self.oracle_background == self.background {
                    "—— 与我们的背景色**同一个值**"
                } else {
                    "—— ⚠ 与我们的背景色**不是同一个值**"
                }
            ),
            format!(
                "行星剪影（本宿主这张图里 != 背景色的像素）：{} 像素（{:.2}%）｜包围盒 {}｜质心 ({:.2}, {:.2})｜等效半径 {:.2}",
                self.silhouette.pixels,
                percent(self.silhouette.pixels),
                self.silhouette.bbox_text(),
                self.center.0,
                self.center.1,
                self.silhouette_radius
            ),
            format!(
                "**差异像素**：{} / {}（{:.3}%）｜最大通道差 {}｜平均通道差（全图） {:.6}｜平均通道差（差异像素上） {:.6}",
                self.differing.pixels,
                total,
                percent(self.differing.pixels),
                self.differing.max_delta,
                self.differing.mean_delta() * self.differing.pixels as f64 / total as f64,
                self.differing.mean_delta()
            ),
            format!("差异包围盒：{}", self.differing.bbox_text()),
            format!(
                "剪影**内**的差异：{} 像素{}｜最大通道差 {}｜平均通道差 {:.6}",
                self.inside.pixels,
                if self.inside.pixels == 0 {
                    "（⇒ 行星自己的像素逐位相同）".to_string()
                } else {
                    format!("｜包围盒 {}", self.inside.bbox_text())
                },
                self.inside.max_delta,
                self.inside.mean_delta()
            ),
            format!(
                "剪影**外**的差异：{} 像素（{:.2}%）｜最大通道差 {}｜平均通道差 {:.6}",
                self.outside.pixels,
                percent(self.outside.pixels),
                self.outside.max_delta,
                self.outside.mean_delta()
            ),
            format!(
                "剪影外差异的半径分布（以等效半径 {:.2} 为单位）：最近 {:.3}｜最远 {:.3}｜[1.00,1.15) 里 {} 个｜其余 {} 个",
                self.silhouette_radius,
                self.outside_min_radius,
                self.outside_max_radius,
                self.outside_ring,
                self.outside_far
            ),
            format!(
                "差异像素按半径分四带（以等效半径为单位）：盘内(<0.95) {}｜边缘(0.95–1.00) {}｜环上(1.00–1.15) {}｜更远(≥1.15) {}",
                self.band_inner, self.band_limb, self.band_ring, self.band_far
            ),
            format!(
                "剪影内**亮度相关系数** {:.6}（1 = 同一片花纹，只是被叠了一层；掉下来 = 花纹本身就不一样）",
                self.inside_luma_correlation
            ),
            format!(
                "剪影内**局部对比相关系数**（横向 lag-2 差分）{:.6}",
                self.inside_contrast_correlation
            ),
        ];
        lines.push(if self.inside.pixels == 0 {
            "结论（只关于剪影）：行星自己那一片像素**逐位相同**；全部差异都在剪影之外。".to_string()
        } else {
            format!(
                "结论（只关于剪影）：行星自己那一片像素里有 {} 个与 oracle 不同（最大通道差 {}）。",
                self.inside.pixels, self.inside.max_delta
            )
        });
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 背景色取众数、剪影按它定义 —— 这一条是整份报告的地基，所以用一张**造出来的**小图钉住。
    #[test]
    fn the_silhouette_is_everything_that_is_not_the_background() {
        // 4×2：背景 0x112233 有 6 个像素，另外两个是"行星"。
        let mut rgb = Vec::new();
        for index in 0..8 {
            let pixel: [u8; 3] = if index == 2 || index == 5 {
                [200, 100, 50]
            } else {
                [0x11, 0x22, 0x33]
            };
            rgb.extend_from_slice(&pixel);
        }
        let bitmap = Bitmap {
            width: 4,
            height: 2,
            rgb,
        };
        assert_eq!(bitmap.at(0, 0), [0x11, 0x22, 0x33]);
        assert_eq!(bitmap.at(2, 0), [200, 100, 50]);
        assert_eq!(bitmap.at(1, 1), [200, 100, 50]);
        let mut counts: std::collections::HashMap<[u8; 3], usize> =
            std::collections::HashMap::new();
        for y in 0..bitmap.height {
            for x in 0..bitmap.width {
                *counts.entry(bitmap.at(x, y)).or_insert(0) += 1;
            }
        }
        let (background, count) = counts.into_iter().max_by_key(|(_, count)| *count).unwrap();
        assert_eq!(background, [0x11, 0x22, 0x33]);
        assert_eq!(count, 6);
    }

    /// 区域统计：包围盒、最大/平均通道差。少一样，"差在哪"就说不清。
    ///
    /// ⚠ 平均那一格是**通道**平均（`sum / (3 × 像素)`）：两个像素、差值 7 与 2
    /// ⇒ 1.5（不是 4.5 —— 4.5 是"每像素取最大通道差再平均"，那是另一个口径）。
    #[test]
    fn a_region_reports_box_max_and_mean() {
        let mut region = Region::default();
        region.add(3, 4, 7);
        region.add(1, 9, 2);
        assert_eq!(region.pixels, 2);
        assert_eq!(region.max_delta, 7);
        assert_eq!(region.bbox, Some((1, 4, 3, 9)));
        assert!(
            (region.mean_delta() - 1.5).abs() < 1e-12,
            "平均**通道**差 = (7+2)/(3×2) = 1.5，实际 {}",
            region.mean_delta()
        );
        assert_eq!(Region::default().bbox_text(), "（没有像素）");
    }
}
