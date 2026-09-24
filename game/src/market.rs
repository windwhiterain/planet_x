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
    /// 势流的价差尺度：`φ = tanh(ln(买价/卖价) / flow_scale)`
    pub flow_scale: f32,
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
    /// 这个仓库货架上有多少。**不是买卖申报**：方向由市场按挂价比较得出。
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
    /// 正 = 这一对里 i 卖出的量，负 = i 买入的量
    pub volume: f32,
    pub price: f32,
}

impl Market {
    pub const DEFAULT_FLOW_SCALE: f32 = 0.5;

    pub fn with_flow_scale(mut self, scale: f32) -> Self {
        self.flow_scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            Self::DEFAULT_FLOW_SCALE
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
            flow_scale: Self::DEFAULT_FLOW_SCALE,
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
