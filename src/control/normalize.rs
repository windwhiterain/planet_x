//! 行为叶的规范化（tagged 形式 → 枚举形式）与整份 diff 的预规范化。

/// The tags the tagged form accepts, in the order an agent should read them.
/// Kept next to [`normalize_behavior`] so the error message can never drift from
/// the parser: **the list of legal tags is written once**.
pub const BEHAVIOR_TAGS: &[&str] = &["idle", "move", "follow", "dock_city", "dock", "colonize"];

/// Normalize a single ship `behavior` value so `apply` accepts BOTH shapes:
///   * the default serde enum form (`{"Follow":{"ship":"华盛顿"}}`,
///     `"Idle"`) — what the `control` template emits and what `.ron` uses; and
///   * the tagged agent-state form (`{"type":"follow","ship":"华盛顿"}`,
///     `{"type":"idle"}`) — exactly what an agent sees in a ship's `order`.
/// The latter is rewritten into the former so the rest of the pipeline stays
/// unchanged.
///
/// **An unknown tag is an error, not a pass-through.** It used to be left as-is
/// "so it fails downstream cleanly" — but the downstream failure was serde's
/// `invalid value: map, expected map with a single key`, which names neither the
/// field nor the legal alternatives, and the single most likely cause is an agent
/// following a stale manual (the removed `target_ship` / `target_settlement`
/// behaviors). So this reports the offending tag **and** what to write instead.
pub fn normalize_behavior(v: &mut serde_json::Value, where_: &str) -> Result<(), String> {
    let Some(ty) = v.get("type").and_then(|t| t.as_str()).map(str::to_string) else {
        return Ok(()); // already the default form (object or "Idle")
    };
    let obj = v.as_object().expect("behavior with type is an object");
    let mut inner = serde_json::Map::new();
    for (k, val) in obj {
        if k != "type" {
            inner.insert(k.clone(), val.clone());
        }
    }
    let variant = match ty.as_str() {
        "idle" => {
            *v = serde_json::Value::String("Idle".to_string());
            return Ok(());
        }
        "move" => "Move",
        "follow" => "Follow",
        "dock_city" => "DockCity",
        "dock" => "Dock",
        "colonize" => "Colonize",
        _ => {
            // 「攻击」与「守卫」曾经是行为，现在不是——这两条最常被写错，
            // 所以把替代写法直接写进错误里。
            let hint = match ty.as_str() {
                "target_ship" | "attackship" | "targetship" => {
                    "「攻击」不再是行为：敌舰进入射程会自动开火。要追袭某舰写 {\"type\":\"follow\",\"ship\":\"<敌舰名>\"}；要守卫友舰写 {\"type\":\"follow\",\"ship\":\"<友舰名>\"}。"
                }
                "target_settlement" | "bombard" | "targetsettlement" => {
                    "「轰炸」不再是行为：敌对城进入围城射程会自动轰炸。要压向某城写 {\"type\":\"dock_city\",\"city\":\"<城名>\"}。"
                }
                "guard" | "escort" | "guard_ship" => {
                    "没有 guard 这个行为：守卫友舰 = {\"type\":\"follow\",\"ship\":\"<友舰名>\"}（Follow 不主动开火，但射程内会自动接战）。"
                }
                _ => "",
            };
            return Err(format!(
                "{where_}: 行为 \"type\":\"{ty}\" 不是合法行为（合法：{}）。{hint}",
                BEHAVIOR_TAGS.join(" / ")
            ));
        }
    };
    let mut m = serde_json::Map::new();
    m.insert(variant.to_string(), serde_json::Value::Object(inner));
    *v = serde_json::Value::Object(m);
    Ok(())
}

/// Walk a control diff and normalize every ship behavior leaf
/// (`ship_orders[].behavior`)：唯一还能写"行为"的地方就是**逐舰指令叶**
/// （舰队默认指令那片叶已删，见 [`crate::model::State::ship_behavior`]）。
/// Only the apply-side JSON path; the state's `order` view is untouched. Errors carry
/// the **diff path** of the offending order so the agent knows which line to fix.
pub fn normalize_control_diffs(value: &mut serde_json::Value) -> Result<(), String> {
    let Some(control) = value.get_mut("control").and_then(|c| c.as_array_mut()) else {
        return Ok(());
    };
    for (fi, fac) in control.iter_mut().enumerate() {
        let Some(orders) = fac.get_mut("ship_orders").and_then(|o| o.as_array_mut()) else {
            continue;
        };
        for (oi, order) in orders.iter_mut().enumerate() {
            let ship = order
                .get("ship")
                .and_then(|s| s.as_str())
                .unwrap_or("?")
                .to_string();
            if let Some(behavior) = order.get_mut("behavior") {
                normalize_behavior(
                    behavior,
                    &format!("control[{fi}].ship_orders[{oi}] (ship 「{ship}」)"),
                )?;
            }
        }
    }
    Ok(())
}
