mod step;

#[cfg(test)]
mod tests;

pub struct Market {
    pub merchandises: Vec<Merchandise>,
    pub traders: Vec<Trader>,
    /// 键：交易者发起者，交易接受者，商品
    pub deals: Vec<Vec<Vec<Deal>>>,
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
    /// 正卖负买
    pub volume: f32,
    deal_price: f32,
    deal_volume: f32,
}

impl TraderMerchandise {
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
    potential: f32,
    /// 正卖负买
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
        Self {
            merchandises,
            traders,
            deals,
        }
    }

    pub fn step(&mut self) {
        step::step(self);
    }
}
