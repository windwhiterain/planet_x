use std::path::Path;

struct Bitmap {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl Bitmap {
    fn load(path: &Path) -> Result<Bitmap, String> {
        let bytes =
            std::fs::read(path).map_err(|err| format!("读 {} 失败：{err}", path.display()))?;
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

pub struct Diff {
    pub width: u32,
    pub height: u32,
    pub left_path: std::path::PathBuf,
    pub right_path: std::path::PathBuf,
    pub background: [u8; 3],
    pub background_pixels: usize,
    pub oracle_background: [u8; 3],
    pub oracle_background_pixels: usize,
    pub silhouette: Region,
    pub differing: Region,
    pub inside: Region,
    pub outside: Region,
    pub silhouette_radius: f64,
    pub center: (f64, f64),
    pub outside_ring: usize,
    pub outside_far: usize,
    pub outside_min_radius: f64,
    pub outside_max_radius: f64,
    pub band_inner: Region,
    pub band_limb: Region,
    pub band_ring: Region,
    pub band_far: Region,
    pub inside_luma_correlation: f64,
    pub inside_contrast_correlation: f64,
    pub large_pixels: usize,
    pub large_min_radius: f64,
    pub large_max_radius: f64,
    pub large_mean_radius: f64,
    pub large_map: [[usize; MAP_COLUMNS]; MAP_ROWS],
    pub inside_histogram: [usize; HISTOGRAM_BINS],
    pub fine_histogram: [usize; FINE_BINS],
    pub band_buckets: [[usize; 8]; 4],
    pub inside_on_edge: usize,
    pub large_on_edge: usize,
    pub isolated: Vec<Isolated>,
    pub isolated_count: usize,
    pub isolated_inside: usize,
    pub isolated_outside: usize,
    pub far_not_background: usize,
}

const DELTA_BUCKETS_LEN: usize = 8;
const HISTOGRAM_BINS: usize = 12;
const FINE_BINS: usize = 10;
const ISOLATED_CAP: usize = 4000;
const ISOLATED_PRINT: usize = 16;
const MAP_COLUMNS: usize = 48;
const MAP_ROWS: usize = 24;
const MAP_RADIUS_LIMIT: f64 = 1.0;
const BACKGROUND_RADIUS: f64 = 1.2;

const DELTA_BUCKETS: [&str; 8] = ["1", "2", "3–4", "5–8", "9–16", "17–32", "33–64", "65+"];

fn delta_bucket(delta: u32) -> usize {
    match delta {
        0 | 1 => 0,
        2 => 1,
        3..=4 => 2,
        5..=8 => 3,
        9..=16 => 4,
        17..=32 => 5,
        33..=64 => 6,
        _ => 7,
    }
}

fn is_edge(bitmap: &Bitmap, x: u32, y: u32) -> bool {
    const THRESHOLD: u32 = 8;
    let center = bitmap.at(x, y);
    let neighbours = [
        (x.wrapping_sub(1), y),
        (x + 1, y),
        (x, y.wrapping_sub(1)),
        (x, y + 1),
    ];
    neighbours.iter().any(|(nx, ny)| {
        if *nx >= bitmap.width || *ny >= bitmap.height || (x == 0 && *nx == u32::MAX) {
            return false;
        }
        let other = bitmap.at(*nx, *ny);
        (0..3).any(|channel| u32::from(center[channel].abs_diff(other[channel])) > THRESHOLD)
    })
}

#[derive(Debug, Clone, Copy)]
pub struct Isolated {
    pub x: u32,
    pub y: u32,
    pub ours: [u8; 3],
    pub oracle: [u8; 3],
    pub radius: f64,
    pub on_edge: bool,
    pub inside: bool,
    pub delta: u32,
}

const LARGE_DELTA: u32 = 8;

fn band_text(band: &Region) -> String {
    if band.pixels == 0 {
        return "0 像素".to_string();
    }
    format!(
        "{} 像素（max Δ {}，平均 Δ {:.3}，包围盒 {}）",
        band.pixels,
        band.max_delta,
        band.mean_delta(),
        band.bbox_text()
    )
}

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

    let mut ring = 0;
    let mut far = 0;
    let mut min_radius = f64::INFINITY;
    let mut max_radius = 0.0_f64;
    let mut band_inner = Region::default();
    let mut band_limb = Region::default();
    let mut band_ring = Region::default();
    let mut band_far = Region::default();
    let (mut large_pixels, mut large_min_radius, mut large_max_radius, mut large_sum_radius) =
        (0_usize, f64::INFINITY, 0.0_f64, 0.0_f64);
    let mut inside_histogram = [0_usize; HISTOGRAM_BINS];
    let mut fine_histogram = [0_usize; FINE_BINS];
    let mut band_buckets = [[0_usize; 8]; 4];
    let mut inside_on_edge = 0_usize;
    let mut large_on_edge = 0_usize;
    let mut isolated: Vec<Isolated> = Vec::new();
    let mut isolated_count = 0_usize;
    let mut isolated_inside = 0_usize;
    let mut isolated_outside = 0_usize;
    let mut far_not_background = 0_usize;
    let mut large_map = [[0_usize; MAP_COLUMNS]; MAP_ROWS];
    let (
        mut count,
        mut sum_mine,
        mut sum_theirs,
        mut sum_sq_mine,
        mut sum_sq_theirs,
        mut sum_cross,
    ) = (0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
    for y in 0..left.height {
        for x in 0..left.width {
            let mine = left.at(x, y);
            let theirs = right.at(x, y);
            if mine != background {
                let dx = f64::from(x) - center.0;
                let dy = f64::from(y) - center.1;
                if (dx * dx + dy * dy).sqrt() > BACKGROUND_RADIUS * radius {
                    far_not_background += 1;
                }
            }
            let dx = f64::from(x) - center.0;
            let dy = f64::from(y) - center.1;
            let distance = (dx * dx + dy * dy).sqrt() / radius;
            if mine != background {
                let luma = |pixel: [u8; 3]| -> i64 {
                    (299 * i64::from(pixel[0])
                        + 587 * i64::from(pixel[1])
                        + 114 * i64::from(pixel[2]))
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
            let delta = (0..3)
                .map(|channel| u32::from(mine[channel].abs_diff(theirs[channel])))
                .max()
                .unwrap_or(0);
            if distance < 0.95 {
                band_inner.add(x, y, delta);
                band_buckets[0][delta_bucket(delta)] += 1;
            } else if distance < 1.0 {
                band_limb.add(x, y, delta);
                band_buckets[1][delta_bucket(delta)] += 1;
            } else if distance < 1.15 {
                band_ring.add(x, y, delta);
                band_buckets[2][delta_bucket(delta)] += 1;
            } else {
                band_far.add(x, y, delta);
                band_buckets[3][delta_bucket(delta)] += 1;
            }
            let edge = is_edge(&left, x, y);
            if mine != background {
                if edge {
                    inside_on_edge += 1;
                }
                let neighbours_same = [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ]
                .iter()
                .all(|(nx, ny)| {
                    let outside = *nx >= left.width || *ny >= left.height;
                    outside
                        || (x == 0 && *nx == u32::MAX)
                        || left.at(*nx, *ny) == right.at(*nx, *ny)
                });
                if neighbours_same {
                    isolated_count += 1;
                    if mine != background {
                        isolated_inside += 1;
                    } else {
                        isolated_outside += 1;
                    }
                    if isolated.len() < ISOLATED_CAP {
                        isolated.push(Isolated {
                            x,
                            y,
                            ours: mine,
                            oracle: theirs,
                            radius: distance,
                            on_edge: edge,
                            inside: mine != background,
                            delta,
                        });
                    }
                }
            }
            if delta > LARGE_DELTA {
                large_pixels += 1;
                large_min_radius = large_min_radius.min(distance);
                large_max_radius = large_max_radius.max(distance);
                large_sum_radius += distance;
                if is_edge(&left, x, y) {
                    large_on_edge += 1;
                }
                if distance <= MAP_RADIUS_LIMIT {
                    let column =
                        (x as usize * MAP_COLUMNS / left.width as usize).min(MAP_COLUMNS - 1);
                    let row = (y as usize * MAP_ROWS / left.height as usize).min(MAP_ROWS - 1);
                    large_map[row][column] += 1;
                }
            }
            if mine != background {
                let bin = ((distance / 0.1) as usize).min(HISTOGRAM_BINS - 1);
                inside_histogram[bin] += 1;
                if (0.80..1.00).contains(&distance) {
                    let fine = ((distance - 0.80) / 0.02) as usize;
                    fine_histogram[fine.min(FINE_BINS - 1)] += 1;
                }
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

    let (
        mut count,
        mut sum_mine,
        mut sum_theirs,
        mut sum_sq_mine,
        mut sum_sq_theirs,
        mut sum_cross,
    ) = (0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
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
        left_path: ours.to_path_buf(),
        right_path: oracle.to_path_buf(),
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
        outside_min_radius: if min_radius.is_finite() {
            min_radius
        } else {
            0.0
        },
        outside_max_radius: max_radius,
        band_inner,
        band_limb,
        band_ring,
        band_far,
        inside_luma_correlation: correlation,
        inside_contrast_correlation: contrast_correlation,
        large_pixels,
        large_min_radius: if large_min_radius.is_finite() {
            large_min_radius
        } else {
            0.0
        },
        large_max_radius,
        large_mean_radius: if large_pixels == 0 {
            0.0
        } else {
            large_sum_radius / large_pixels as f64
        },
        large_map,
        inside_histogram,
        fine_histogram,
        band_buckets,
        inside_on_edge,
        large_on_edge,
        isolated,
        isolated_count,
        isolated_inside,
        isolated_outside,
        far_not_background,
    })
}

impl Diff {
    pub fn report(&self) -> String {
        let total = self.width as usize * self.height as usize;
        let percent = |count: usize| 100.0 * count as f64 / total as f64;
        let mut lines = vec![
            format!(
                "⚠ 本工具**不认识角色**：下面一律按**位置**称呼 —— \
                 左图 = 第一个参数，右图 = 第二个参数；\
                 背景色与剪影**只按左图**定（约定是『左 = 本宿主那张 / 右 = oracle』）。\
                 传反了，整篇读数就反着读。"
            ),
            format!("  左：{}", self.left_path.display()),
            format!("  右：{}", self.right_path.display()),
            format!("尺寸：{}×{}（{} 像素）", self.width, self.height, total),
            format!(
                "左图的背景色：{:?}（{} 像素，{:.2}%）—— 剪影按它定义",
                self.background,
                self.background_pixels,
                percent(self.background_pixels)
            ),
            format!(
                "右图的众数色：{:?}（{} 像素，{:.2}%）{}",
                self.oracle_background,
                self.oracle_background_pixels,
                percent(self.oracle_background_pixels),
                if self.oracle_background == self.background {
                    "—— 与左图的背景色**同一个值**"
                } else {
                    "—— ⚠ 与左图的背景色**不是同一个值**"
                }
            ),
            format!(
                "行星剪影（左图里 != 背景色的像素）：{} 像素（{:.2}%）｜包围盒 {}｜质心 ({:.2}, {:.2})｜等效半径 {:.2}",
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
                band_text(&self.band_inner),
                band_text(&self.band_limb),
                band_text(&self.band_ring),
                band_text(&self.band_far)
            ),
            format!(
                "剪影内**亮度相关系数** {:.6}（1 = 同一片花纹，只是被叠了一层；掉下来 = 花纹本身就不一样）",
                self.inside_luma_correlation
            ),
            format!(
                "剪影内**局部对比相关系数**（横向 lag-2 差分）{:.6}",
                self.inside_contrast_correlation
            ),
            format!(
                "**大差异**（Δ>{}）：{} 像素｜半径 {:.3}–{:.3}（均值 {:.3}，以等效半径为单位）\
                 —— 铺成一圈是边缘/舍入，挤在一处才是『哪个东西画错了』",
                LARGE_DELTA,
                self.large_pixels,
                self.large_min_radius,
                self.large_max_radius,
                self.large_mean_radius
            ),
            format!(
                "剪影内差异的**半径直方图**（每 0.1 个等效半径一格，最后一格是 ≥{}）：{}",
                ((HISTOGRAM_BINS - 1) as f64) / 10.0,
                self.inside_histogram
                    .iter()
                    .enumerate()
                    .map(|(bin, count)| format!(
                        "{:.1}–{:.1}: {count}",
                        bin as f64 / 10.0,
                        (bin + 1) as f64 / 10.0
                    ))
                    .collect::<Vec<_>>()
                    .join("｜")
            ),
        ];
        let band_names = [
            "盘内(<0.95)",
            "边缘(0.95–1.00)",
            "环上(1.00–1.15)",
            "更远(≥1.15)",
        ];
        for (index, name) in band_names.iter().enumerate() {
            let buckets = self.band_buckets[index];
            if buckets.iter().all(|count| *count == 0) {
                continue;
            }
            lines.push(format!(
                "  带的 Δ 分档 {}：{}",
                name,
                DELTA_BUCKETS
                    .iter()
                    .zip(buckets.iter())
                    .map(|(label, count)| format!("Δ{label}: {count}"))
                    .collect::<Vec<_>>()
                    .join("｜")
            ));
        }
        lines.push(format!(
            "剪影内差异像素**在几何边上**的：{} / {}（大差异里在边上的：{} / {}）\
             —— 边上占多数 ⇒ 指向光栅化边缘/取样位置；平处占多数 ⇒ 指向着色公式",
            self.inside_on_edge, self.inside.pixels, self.large_on_edge, self.large_pixels
        ));
        lines.push(format!(
            "剪影内差异的**细分直方图**（0.80–1.00R 每 0.02R 一格；行星轮廓 ≈0.87R、\
             大气壳外缘 ≈1.00R）：{}",
            self.fine_histogram
                .iter()
                .enumerate()
                .map(|(bin, count)| format!(
                    "{:.2}–{:.2}: {count}",
                    0.80 + bin as f64 * 0.02,
                    0.82 + bin as f64 * 0.02
                ))
                .collect::<Vec<_>>()
                .join("｜")
        ));
        lines.push(format!(
            "大差异（Δ>{LARGE_DELTA}）落在画面的哪一块（**只算等效半径 {MAP_RADIUS_LIMIT} 以内**；\
             地图是 {MAP_COLUMNS} 列 × {MAP_ROWS} 行，一格 = {:.2}×{:.2} 像素，数字是该格里的大差异像素数，9 封顶）：",
            self.width as f64 / MAP_COLUMNS as f64,
            self.height as f64 / MAP_ROWS as f64
        ));
        for row in self.large_map.iter() {
            lines.push(format!(
                "    {}",
                row.iter()
                    .map(|count| if *count == 0 {
                        '.'
                    } else {
                        char::from_digit((*count).min(9) as u32, 10).unwrap_or('?')
                    })
                    .collect::<String>()
            ));
        }
        lines.push(format!(
            "**远离任何几何处**（≥{BACKGROUND_RADIUS}R，那里只该有清屏色）：左图里有 {} 个像素\
             不等于背景色 —— **0 ⇒ 均匀输入没被任何一条路扰动**（blit / sRGB 往返那一类\
             『处处生效』的病因就此排除）；>0 ⇒ 那条路有扰动",
            self.far_not_background
        ));
        if self.isolated.is_empty() && self.isolated_count == 0 {
            lines.push("孤立差异像素（四邻居全都逐位相同）：一个都没有".to_string());
        } else {
            let mut listed = self.isolated.clone();
            listed.sort_by(|a, b| a.radius.total_cmp(&b.radius));
            lines.push(format!(
                "孤立差异像素（四邻居全都逐位相同）：共 {} 个 —— **剪影内 {} / 剪影外 {}**\
                 （⚠ 剪影外是均匀清屏色、没有任何着色：那边也有 ⇒ 病因在处处生效的那条路；\
                 只在剪影内 ⇒ 病因在着色那几条路）",
                self.isolated_count, self.isolated_inside, self.isolated_outside
            ));
            lines.push(format!(
                "  其中剪影内的：Δ1 有 {} 个（单通道一个台阶）｜剪影外的：Δ1 有 {} 个",
                self.isolated
                    .iter()
                    .filter(|pixel| pixel.inside && pixel.delta == 1)
                    .count(),
                self.isolated
                    .iter()
                    .filter(|pixel| !pixel.inside && pixel.delta == 1)
                    .count()
            ));
            for pixel in listed.iter().take(ISOLATED_PRINT) {
                lines.push(format!(
                    "    ({}, {})｜r = {:.4}｜{}｜Δ{}｜左 {:?} vs 右 {:?}｜{}",
                    pixel.x,
                    pixel.y,
                    pixel.radius,
                    if pixel.inside {
                        "剪影内"
                    } else {
                        "剪影外"
                    },
                    pixel.delta,
                    pixel.ours,
                    pixel.oracle,
                    if pixel.on_edge {
                        "在边上"
                    } else {
                        "在平处"
                    }
                ));
            }
        }
        if self.inside.pixels == 0 {
            lines.push(
                "结论（只关于剪影）：剪影里那一片像素**逐位相同**；全部差异都在剪影之外。"
                    .to_string(),
            );
        } else {
            lines.push(format!(
                "结论（只关于剪影）：剪影里有 {} 个像素与右图不同（最大通道差 {}）。",
                self.inside.pixels, self.inside.max_delta
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_silhouette_is_everything_that_is_not_the_background() {
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
