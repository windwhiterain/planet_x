mod step;

#[cfg(test)]
mod tests;

pub struct Market {
    pub merchandises: Vec<Merchandise>,
    pub traders: Vec<Trader>,
    /// key: deal sender, deal reciver, merchandise
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
    /// positive buy negative sell
    pub volume: f32,
    /// 申报曲线所依据的市价
    pub reference_price: f32,
    /// 量的价格弹性：量(p) = volume × (p / reference_price)^elasticity
    pub elasticity: f32,
    deal_price: f32,
    deal_volume: f32,
}

impl TraderMerchandise {
    pub fn new(price: f32, volume: f32) -> Self {
        Self {
            price,
            volume,
            reference_price: 0.0,
            elasticity: 0.0,
            deal_price: 0.0,
            deal_volume: 0.0,
        }
    }

    /// 申报曲线：市价给定时，这一轮愿意买卖多少
    pub fn volume_at(&self, price: f32) -> f32 {
        if self.elasticity == 0.0 || self.reference_price <= 0.0 || price <= 0.0 {
            return self.volume;
        }
        self.volume * (price / self.reference_price).powf(self.elasticity)
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
