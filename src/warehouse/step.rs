use crate::market::Market;
use crate::warehouse::{Stock, Warehouse, Warehouses};

fn update_target(stock: &Stock, rate: f32) -> f32 {
    if !(rate > 0.0) {
        return stock.target_floor;
    }
    let adjustment = stock.wanted - stock.taken;
    if !adjustment.is_finite() {
        return stock.target_volume;
    }
    (stock.target_volume + rate * adjustment).clamp(stock.target_floor, Warehouses::TARGET_CAP)
}

fn update_price(stock: &mut Stock, reference: f32, curvature: f32, inertia: f32) {
    if !stock.price.is_finite() || !(stock.price > 0.0) {
        stock.price = if reference.is_finite() && reference > 0.0 {
            reference
        } else {
            1.0
        };
    }
    if !stock.price_base.is_finite() || !(stock.price_base > 0.0) {
        stock.price_base = stock.price;
    }
    let target = if stock.target_volume.is_finite() && stock.target_volume > 0.0 {
        stock.target_volume as f64
    } else {
        1.0
    };
    let ratio = (stock.volume as f64 / target).max(1e-6);
    let log_ratio = ratio.ln().clamp(-40.0, 40.0);
    let mapped = stock.price_base as f64 * (-(curvature as f64) * log_ratio.tanh()).exp();
    let price = if inertia > 0.0 {
        (inertia as f64) * stock.price as f64 + (1.0 - inertia as f64) * mapped
    } else {
        mapped
    };
    if price.is_finite() && price > 0.0 && price <= f32::MAX as f64 {
        stock.price = price as f32;
    }
}

/// 逐地方的挂价区间：ask = 最小挂价，bid = 最大挂价。当轮挂价的纯函数。
fn local_quotes(warehouses: &[Warehouse], goods: usize) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let localities = warehouses
        .iter()
        .map(|warehouse| warehouse.locality + 1)
        .max()
        .unwrap_or(0);
    let mut ask = vec![vec![0.0f32; goods]; localities];
    let mut bid = vec![vec![0.0f32; goods]; localities];
    for warehouse in warehouses {
        let locality = warehouse.locality;
        for (k, stock) in warehouse.stocks.iter().enumerate() {
            let price = stock.price;
            if !price.is_finite() || !(price > 0.0) {
                continue;
            }
            if !(ask[locality][k] > 0.0) || price < ask[locality][k] {
                ask[locality][k] = price;
            }
            if price > bid[locality][k] {
                bid[locality][k] = price;
            }
        }
    }
    (ask, bid)
}

pub(super) fn step(warehouses: &mut Warehouses, market: &mut Market) {
    let Warehouses {
        warehouses: list,
        price_curvature,
        price_inertia,
        target_rate,
        ask,
        bid,
    } = warehouses;
    let price_curvature = *price_curvature;
    let price_inertia = *price_inertia;
    let target_rate = *target_rate;
    let goods = market.merchandises.len();

    for warehouse in list.iter_mut() {
        let reference = if warehouse.reference.len() == goods {
            warehouse.reference.clone()
        } else {
            vec![1.0; goods]
        };
        for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
            stock.target_volume = update_target(stock, target_rate);
            update_price(stock, reference[k], price_curvature, price_inertia);
        }
    }

    let (local_ask, local_bid) = local_quotes(list, goods);
    *ask = local_ask;
    *bid = local_bid;

    for (i, warehouse) in list.iter_mut().enumerate() {
        for (k, stock) in warehouse.stocks.iter().enumerate() {
            let merchandise = &mut market.traders[i].merchandises[k];
            merchandise.price = stock.price;
            merchandise.volume = stock.volume.max(0.0);
        }
    }

    market.step();

    for (i, warehouse) in list.iter_mut().enumerate() {
        for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
            stock.volume = (stock.volume - market.traders[i].merchandises[k].deal_volume()).max(0.0);
        }
    }
}
