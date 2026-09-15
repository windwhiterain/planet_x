mod step;

#[cfg(test)]
mod tests;

use crate::market::Market;

pub struct Warehouses {
    pub warehouses: Vec<Warehouse>,
    pub price_curvature: f32,
    pub price_inertia: f32,
    pub target_rate: f32,
    /// index 地方 × 商品：当轮挂价的最小值
    pub ask: Vec<Vec<f32>>,
    /// index 地方 × 商品：当轮挂价的最大值
    pub bid: Vec<Vec<f32>>,
}

/// index with [`crate::market::Trader`]
pub struct Warehouse {
    pub stocks: Vec<Stock>,
    /// 价格水平的计量基准（常数）
    pub reference: Vec<f32>,
    pub locality: usize,
}

/// index with [`crate::market::Merchandise`]
pub struct Stock {
    pub volume: f32,
    pub target_volume: f32,
    pub target_floor: f32,
    pub price: f32,
    pub price_base: f32,
    /// 部门本轮在货架不受限时会取走的量
    pub wanted: f32,
    /// 部门本轮实际取走的量
    pub taken: f32,
}

impl Warehouses {
    pub const DEFAULT_PRICE_CURVATURE: f32 = 1.0;
    pub const DEFAULT_PRICE_INERTIA: f32 = 0.8;
    pub const DEFAULT_TARGET_RATE: f32 = 0.01;
    pub const TARGET_CAP: f32 = 1e6;

    pub fn new(warehouses: Vec<Warehouse>) -> Self {
        Self {
            warehouses,
            price_curvature: Self::DEFAULT_PRICE_CURVATURE,
            price_inertia: Self::DEFAULT_PRICE_INERTIA,
            target_rate: Self::DEFAULT_TARGET_RATE,
            ask: Vec::new(),
            bid: Vec::new(),
        }
    }

    pub fn set_price_law(&mut self, curvature: f32, inertia: f32, target_rate: f32) {
        self.price_curvature = if curvature.is_finite() && curvature > 0.0 {
            curvature
        } else {
            Self::DEFAULT_PRICE_CURVATURE
        };
        self.price_inertia = if inertia.is_finite() {
            inertia.clamp(0.0, 1.0)
        } else {
            Self::DEFAULT_PRICE_INERTIA
        };
        self.target_rate = if target_rate.is_finite() && target_rate >= 0.0 {
            target_rate
        } else {
            Self::DEFAULT_TARGET_RATE
        };
    }

    /// 银河指数 = 各仓库挂价的几何平均。只用于报表，任何决策都不读它。
    pub fn quoted_index(&self, goods: usize) -> Vec<f32> {
        let mut index = vec![0.0f32; goods];
        for (k, slot) in index.iter_mut().enumerate() {
            let mut logs = 0.0f64;
            let mut count = 0.0f64;
            for warehouse in &self.warehouses {
                let Some(stock) = warehouse.stocks.get(k) else {
                    continue;
                };
                let price = stock.price;
                if !price.is_finite() || !(price > 0.0) {
                    continue;
                }
                logs += (price as f64).ln();
                count += 1.0;
            }
            *slot = if count > 0.0 {
                (logs / count).exp() as f32
            } else {
                0.0
            };
        }
        index
    }

    /// 某个地方每种商品的挂价相对指数的比值
    pub fn local_ratios(&self, index: &[f32]) -> Vec<Vec<f32>> {
        let localities = self
            .warehouses
            .iter()
            .map(|warehouse| warehouse.locality + 1)
            .max()
            .unwrap_or(0);
        let goods = self
            .warehouses
            .first()
            .map(|warehouse| warehouse.stocks.len())
            .unwrap_or(0);
        let mut ratios = vec![vec![1.0f32; goods]; localities];
        for warehouse in &self.warehouses {
            for (k, stock) in warehouse.stocks.iter().enumerate() {
                let reference = index.get(k).copied().unwrap_or(0.0);
                if reference > 0.0 && stock.price.is_finite() && stock.price > 0.0 {
                    ratios[warehouse.locality][k] = stock.price / reference;
                }
            }
        }
        ratios
    }

    pub fn step(&mut self, market: &mut Market) {
        step::step(self, market);
    }
}

impl Warehouse {
    pub fn new(stocks: Vec<Stock>) -> Self {
        Self {
            stocks,
            reference: Vec::new(),
            locality: 0,
        }
    }

    pub fn with_locality(mut self, locality: usize) -> Self {
        self.locality = locality;
        self
    }

    pub fn with_reference(mut self, reference: Vec<f32>) -> Self {
        self.reference = reference;
        self
    }
}

impl Stock {
    pub fn new(volume: f32, target_volume: f32) -> Self {
        Self {
            volume,
            target_floor: target_volume,
            target_volume,
            price: 0.0,
            price_base: 0.0,
            wanted: 0.0,
            taken: 0.0,
        }
    }

    pub fn record_take(&mut self, wanted: f32, volume: f32) {
        self.wanted = if wanted.is_finite() {
            wanted.max(0.0)
        } else {
            0.0
        };
        self.taken = if volume.is_finite() {
            volume.max(0.0)
        } else {
            0.0
        };
    }
}
