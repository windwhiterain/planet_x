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
    /// positive buy negative sell
    pub volume: f32,
    pub price: f32,
}

impl Market {
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
