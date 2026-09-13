#[cfg(test)]
mod tests;

pub struct MarketState {
    states: Vec<MerchandiseState>,
    forgetting: f32,
}

pub struct MerchandiseState {
    level: f32,
    drift: f32,
    volatility: f32,
}

impl MarketState {
    pub const DEFAULT_FORGETTING: f32 = 0.95;

    pub fn new(prices: &[f32]) -> Self {
        Self {
            states: prices
                .iter()
                .map(|&price| MerchandiseState {
                    level: price,
                    drift: 0.0,
                    volatility: 0.0,
                })
                .collect(),
            forgetting: Self::DEFAULT_FORGETTING,
        }
    }

    pub fn with_forgetting(mut self, forgetting: f32) -> Self {
        self.forgetting = forgetting.clamp(0.0, 1.0);
        self
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    pub fn level(&self, merchandise: usize) -> f32 {
        self.state(merchandise).map_or(0.0, |state| state.level)
    }

    pub fn drift(&self, merchandise: usize) -> f32 {
        self.state(merchandise).map_or(0.0, |state| state.drift)
    }

    pub fn volatility(&self, merchandise: usize) -> f32 {
        self.state(merchandise).map_or(0.0, |state| state.volatility)
    }

    pub fn observe(&mut self, merchandise: usize, price: f32) {
        if !price.is_finite() || price <= 0.0 {
            return;
        }
        let forgetting = self.forgetting;
        let Some(state) = self.states.get_mut(merchandise) else {
            return;
        };
        if !state.level.is_finite() || state.level <= 0.0 {
            state.level = price;
            return;
        }
        let change = (price / state.level).ln();
        if !change.is_finite() {
            state.level = price;
            return;
        }
        state.level = price;
        state.drift += (1.0 - forgetting) * (change - state.drift);
        state.volatility += (1.0 - forgetting) * ((change - state.drift).abs() - state.volatility);
    }

    fn state(&self, merchandise: usize) -> Option<&MerchandiseState> {
        self.states.get(merchandise)
    }
}
