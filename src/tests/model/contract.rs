//! 承包单模型的单元测试。

use super::*;

fn sample() -> Contract {
    let mut cs = ContractState::default();
    cs.post(
        "中国".to_string(),
        "碳".to_string(),
        3.0,
        "金星".to_string(),
        "地球".to_string(),
        0.15,
        3,
        0.6,
    );
    cs.contracts[0].clone()
}

/// 单号单调递增且不复用（历史事件里的单号永远指得准）。
#[test]
fn contract_ids_are_allocated_monotonically() {
    let mut cs = ContractState::default();
    let a = cs.post("中国".into(), "碳".into(), 1.0, "金星".into(), "地球".into(), 0.1, 0, 0.6);
    cs.contracts.clear(); // 即使单子被移出，号也不回头
    let b = cs.post("美国".into(), "铁".into(), 1.0, "水星".into(), "火星".into(), 0.1, 1, 0.6);
    assert_eq!((a, b), (0, 1), "号必须单调递增、不复用");
}

/// **抽成 = 承运人的报酬**（Q10）：它交出 `units` 里的 `1-share`，自留 `share`。
#[test]
fn the_carrier_keeps_its_share_of_each_delivery() {
    let c = sample();
    assert!((c.carrier_cut(10.0) - 1.5).abs() < 1e-9, "15% 抽成 ⇒ 10 件里自留 1.5");
    assert!((10.0 - c.carrier_cut(10.0) - 8.5).abs() < 1e-9, "余数 8.5 进雇主首都池");
    // 抽成比例越界也要被夹住（配置写错不该让货凭空翻倍）。
    let mut wild = c.clone();
    wild.share = 3.0;
    assert!((wild.carrier_cut(10.0) - 10.0).abs() < 1e-9, "比例被夹到 1.0");
}

/// **达标率要连着「账期够不够」一起看**：产出期不满两个来回（= 账上该有的产出还不到
/// 一个货舱）就不评——拿半个账期去判分，判出来的必然是 0。
///
/// 这一条同时挡住了「深空小单的尺子坏掉」：要求运力 0.05 件/回合的线，一个来回 120 回合，
/// 只按「过了几个回合」算的话随便搬一趟就是几十倍达标率。现在账期不够就**不评**，
/// 够的时候期望本身就至少是一舱货，达标率才真的是「几舱 ÷ 该几舱」。
#[test]
fn the_throughput_ratio_waits_until_a_full_hold_is_due_and_skips_the_transit_leg() {
    let config = crate::config::load_config();
    let mut c = sample();
    c.capacity = 3.0; // ⇒ 在途宽免 = nominal_hold(6) / 3.0 = 2 回合
    assert_eq!(c.grace_rounds(&config), 2.0, "宽免 = 一个往返回合数（由条文自己推出来）");
    assert_eq!(c.throughput_ratio(&config), None, "还没开工 ⇒ 无从考核");
    c.served_rounds = 2;
    assert_eq!(c.throughput_ratio(&config), None, "产出期还是 0（前两个回合是在途）");
    c.served_rounds = 4; // 产出期 2 回合 ⇒ 该产出 3.0×2 = 6 件 = 恰好一个货舱
    assert_eq!(c.throughput_ratio(&config), Some(0.0), "有货可运却一件没交 ⇒ 达标率 0");
    c.delivered = 6.0;
    assert_eq!(c.throughput_ratio(&config), Some(1.0), "账期该产出 6 件、实交 6 ⇒ 恰好达标");
    c.delivered = 9.0;
    assert_eq!(c.throughput_ratio(&config), Some(1.5), "搬了 1.5 倍 ⇒ 雇到了更好的运力");
    // **深空小单**：要求运力 0.1/回合 ⇒ 一个来回 60 回合。账期不足 60 个产出回合就
    // 不该开评（否则「搬了一趟」会被算成几十倍达标率）。
    let mut deep = sample();
    deep.capacity = 0.1;
    deep.served_rounds = 100; // 产出期 40 回合 ⇒ 该产出 4 件，还不够一舱
    assert_eq!(deep.throughput_ratio(&config), None, "账期不足一个货舱 ⇒ 不评");
    deep.served_rounds = 130; // 产出期 70 ⇒ 该产出 7 件 ≥ 一舱
    deep.delivered = 7.0;
    assert_eq!(deep.throughput_ratio(&config), Some(1.0), "账期够了才评，且 1.0 仍是「一条参考船」");
    // 要求运力为 0（退化配置）也不能除出 NaN/∞。
    c.capacity = 0.0;
    assert_eq!(c.throughput_ratio(&config), None);
}

/// **一处货栈只有一张未接单**：它是需求信号，不是报价单。
///
/// 挂成一片只会把同一份缺口反复请人；而**已接单**的那张不属于 `open_mut`——它是承诺。
#[test]
fn there_is_at_most_one_open_order_per_lane() {
    let mut cs = ContractState::default();
    cs.post("中国".into(), "碳".into(), 3.0, "金星".into(), "地球".into(), 0.1, 0, 0.6);
    assert!(cs.open_mut("中国", "金星", "地球").is_some());
    assert!(cs.open_mut("美国", "金星", "地球").is_none(), "别人的单不算我的");
    assert!(cs.open_mut("中国", "水星", "地球").is_none(), "别的货栈各自成单");
    // 有人接了 ⇒ 不再可改（承诺冻结）。
    let id = cs.contracts[0].id;
    cs.get_mut(id).unwrap().carrier = Some("美国".into());
    cs.get_mut(id).unwrap().accepted_round = Some(0);
    assert!(cs.open_mut("中国", "金星", "地球").is_none(), "已接单的合同不再是需求信号");
}

/// **一张单可以同时有多艘舰，也可以随时换**（用户：「对方派几艘船都无所谓」）。
#[test]
fn a_contract_can_run_any_number_of_ships() {
    let mut cs = ContractState::default();
    let id = cs.post("中国".into(), "碳".into(), 3.0, "金星".into(), "地球".into(), 0.1, 0, 0.6);
    cs.assign("甲".into(), id);
    cs.assign("乙".into(), id);
    let got = cs.ships_of(id);
    assert_eq!(got.len(), 2, "多对一：一张单同时跑两条船，实为 {got:?}");
    assert!(got.contains(&"甲".to_string()) && got.contains(&"乙".to_string()));
    assert_eq!(cs.assignment_of("甲"), Some(id));
    // 收尾：合同结束时一次放掉全部（不留幽灵引用）。
    assert_eq!(cs.release(id).len(), 2);
    assert!(cs.ships_of(id).is_empty());
}
