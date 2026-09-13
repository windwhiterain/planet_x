mod step;

#[cfg(test)]
mod tests;

use crate::market_state::MarketState;

pub struct Market {
    pub merchandises: Vec<Merchandise>,
    pub traders: Vec<Trader>,
    /// key: deal sender, deal reciver, merchandise
    pub deals: Vec<Vec<Vec<Deal>>>,
    /// key: deal sender, deal reciver
    pub relations: Vec<Vec<f32>>,
    pub state: MarketState,
    /// 软成交容差，见 [`Market::with_soft_eps`]
    pub soft_eps: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Merchandise {
    pub price: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trader {
    pub merchandises: Vec<TraderMerchandise>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraderMerchandise {
    pub price: f32,
    /// positive buy negative sell
    pub volume: f32,
    deal_price: f32,
    deal_volume: f32,
}

impl TraderMerchandise {
    pub fn new(price: f32, volume: f32) -> Self {
        Self {
            price,
            volume,
            deal_price: 0.0,
            deal_volume: 0.0,
        }
    }

    pub fn deal_price(&self) -> f32 {
        self.deal_price
    }
    pub fn deal_volume(&self) -> f32 {
        self.deal_volume
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Deal {
    price_potential: f32,
    volume_potential: f32,
    distribution: f32,
    /// 软成交的成交价（卖价略高于买价时，按 买价×(1−eps) 成交）；0 = 走几何平均那条路
    soft_price: f32,
    /// positive buy negative sell
    pub volume: f32,
    pub price: f32,
}

impl Market {
    /// 软成交的容差：卖价高出买价不超过 `eps × 买价` 时，视为卖方以
    /// `买价 × (1 − eps)` 卖出一小笔「试用品」，量随价差线性缩水，价差到顶即归零。
    ///
    /// **这是模型的一部分，默认开着**（[`Market::DEFAULT_SOFT_EPS`]）。旧规则
    /// `(买价 − 卖价).max(0)` 是个**硬截断**：报价差一点点就一件都成交不了，
    /// 于是既产生不了信息，也让部门被整段饿死（实测最差部门的执行率 0.057）。
    /// 软成交把这个不连续点拆掉，同时又顺带修了"买价 == 卖价 ⇒ 潜在量 0 ⇒
    /// 锁价账本一笔不成交"那个边角。
    ///
    /// `0.0` = 关回硬撮合，只用于对照。
    /// 默认容差。取 0.15 是**量出来的**：在 `capacity ∈ {6,12} × specialty=2` 两个
    /// 配置上，它是唯一一个同时把成交量和最差部门执行率都抬上去的值——
    ///
    /// | eps | cap6 成交 一/二 | cap6 执行率最小 | cap12 成交 一/二 | cap12 执行率最小 |
    /// |---|---|---|---|---|
    /// | 0（硬） | 35.97 / 34.05 | 0.057 | 21.23 / 20.73 | 0.314 |
    /// | 0.05 | **0** / 15.42 | 0.753 | **0** / 16.47 | 0.581 |
    /// | 0.10 | 17.81 / 18.28 | 0.723 | 9.58 / 11.28 | **0.056** |
    /// | **0.15** | **79.15 / 17.28** | **0.816** | **24.22 / 28.57** | **1.000** |
    /// | 0.20 | 11.97 / 20.45 | 0.825 | — | — |
    pub const DEFAULT_SOFT_EPS: f32 = 0.15;

    pub fn with_soft_eps(mut self, eps: f32) -> Self {
        self.soft_eps = if eps.is_finite() && eps > 0.0 {
            eps
        } else {
            0.0
        };
        self
    }

    pub fn new(merchandises: Vec<Merchandise>, traders: Vec<Trader>) -> Self {
        let merchandises_len = merchandises.len();
        for trader in &traders {
            debug_assert_eq!(
                trader.merchandises.len(),
                merchandises_len,
                "每个交易者的商品表必须与市场商品表等长且同序",
            );
        }
        let traders_len = traders.len();
        let deals = vec![vec![vec![Deal::default(); merchandises_len]; traders_len]; traders_len];
        let prices: Vec<f32> = merchandises.iter().map(|item| item.price).collect();
        let state = MarketState::new(&prices);
        let relations = vec![vec![1.0; traders_len]; traders_len];
        Self {
            merchandises,
            traders,
            deals,
            relations,
            state,
            soft_eps: Self::DEFAULT_SOFT_EPS,
        }
    }

    pub fn set_relations(&mut self, relations: &[Vec<f32>]) {
        for (i, row) in relations.iter().enumerate() {
            if i >= self.relations.len() {
                break;
            }
            for (j, relation) in row.iter().enumerate() {
                if j >= self.relations[i].len() {
                    break;
                }
                self.relations[i][j] = relation.clamp(0.0, 1.0);
            }
        }
    }

    pub fn step(&mut self) {
        step::step(self);
    }
}
