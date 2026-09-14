use px_protocol::sim::{DepartmentView, GoodView, Totals, WorldView};
use px_protocol::SCHEMA_VERSION;

use crate::{DEPARTMENTS, DEPARTMENT_NAMES, GOODS, GOOD_NAMES, Snapshot};

pub fn world_view(snapshot: &Snapshot) -> WorldView {
    let goods = (0..GOODS)
        .map(|good| GoodView {
            name: GOOD_NAMES[good].to_string(),
            price: snapshot.prices[good],
        })
        .collect();

    let departments = (0..DEPARTMENTS)
        .map(|department| DepartmentView {
            name: DEPARTMENT_NAMES[department].to_string(),
            // `executions`（执行率）随 §19.4 的换名删掉了，物理活跃度由 `intake` 承担。
            execution: snapshot.intake[department],
            revenue: snapshot.revenues[department],
            payment: snapshot.payments[department],
            holdings: (0..GOODS)
                .map(|good| snapshot.holdings[department][good])
                .collect(),
        })
        .collect();

    WorldView {
        schema_version: SCHEMA_VERSION,
        round: snapshot.round as u32,
        goods,
        departments,
        totals: Totals {
            cpi: snapshot.cpi,
            output: snapshot.output,
            consumption: snapshot.consumption,
            turnover: snapshot.turnover,
            granted: snapshot.granted,
            treasury: snapshot.treasury,
        },
    }
}
