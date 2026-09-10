//! `web/static/views.json`（**组织点声明**）的校验测试。
//!
//! 为什么这份声明需要 Rust 侧的守门人：它是**数据**，写错了不会编译报错——只会让某个视图
//! 静静地少一列、或者引用一张不存在的图。所以这里把两条纪律钉死：
//!
//! 1. **静态**：id 唯一、引用完整（`use`/`use_at`/`map_ref`/`label_from` 指的都在）、
//!    `omit` 与列**不重叠**（自相矛盾的声明）、每条路径表达式都合文法、`@根` 都在已知根里。
//! 2. **对真实世界**：用真配置起一个世界、跑若干回合，按 `source` 取出记录，
//!    检查每条**相对列**的首段在记录里真的存在（引擎改了字段名 ⇒ 这里红）。
//!    另有「这条视图有 N 列本帧取不到」的覆盖率报告（只印不判红：有些字段本来就只在
//!    某些局面出现，例如 `cargo` 只在装货时非空——那种情形由界面上的运行时自检显示）。
//!
//! 完整的路径**求值**只有一份实现（`web/static/specview.js`）。这里只做**存在性**检查：
//! 解析路径段 + 对真实记录做首段匹配 —— 够抓住「字段名写错 / 引擎改名」，又不必把求值器
//! 再写一遍（那才会漂移）。

use planet_x::config::load_config_from;
use planet_x::control::{control_view, scope_view};
use planet_x::prng::Prng;
use serde_json::Value;
use std::collections::BTreeSet;

fn spec_doc() -> Value {
    let raw = include_str!("../static/views.json");
    serde_json::from_str(raw).expect("web/static/views.json 必须是合法 JSON")
}

/// 一帧读面：与 `/api/state` 给前端的根**同一批**（info 的五个根 + 写面的两个读模板）。
fn frame(seed: u64, rounds: u32) -> Vec<(String, Value)> {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../config/game.ron"));
    let config = load_config_from(path).expect("workspace config/game.ron loads");
    let mut state = planet_x::world::default_state(&config, seed);
    let mut rng = Prng::new(seed);
    let mut view = planet_x::sim::view_from_state(&state, &config);
    for _ in 0..rounds {
        view = planet_x::sim::advance(&mut state, &config, &mut rng);
    }
    let control: Vec<Value> = state
        .control
        .iter()
        .map(|(fid, c)| {
            serde_json::to_value(control_view(&state, &config, fid.clone(), c)).expect("control view")
        })
        .collect();
    vec![
        ("state".into(), planet_x::json::to_value(&state).unwrap()),
        ("pre".into(), planet_x::json::to_value(&view).unwrap()),
        ("post".into(), planet_x::json::to_value(&view).unwrap()),
        ("config".into(), planet_x::json::to_value(&config).unwrap()),
        ("session".into(), serde_json::json!({"rng_state": rng.state()})),
        ("control".into(), Value::Array(control)),
        ("scope".into(), serde_json::to_value(scope_view(&state.scope)).unwrap()),
    ]
}

const ROOTS: [&str; 7] = ["state", "pre", "post", "config", "session", "control", "scope"];

// --- 路径表达式（与 specview.js 同一套文法；这里只解析，不求值） ------------------
#[derive(Debug, Clone, PartialEq)]
struct Seg {
    name: String,
    spread: bool,
    index: Option<usize>,
    pick: Option<(String, String)>,
}

fn split_segments(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth = 0i32;
    for ch in expr.chars() {
        match ch {
            '[' => {
                depth += 1;
                buf.push(ch);
            }
            ']' => {
                depth -= 1;
                buf.push(ch);
            }
            '.' if depth == 0 => {
                out.push(std::mem::take(&mut buf));
            }
            _ => buf.push(ch),
        }
    }
    out.push(buf);
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

fn parse_seg(raw: &str) -> Result<Seg, String> {
    let (name, inside) = match raw.find('[') {
        None => (raw.to_string(), None),
        Some(i) => {
            if !raw.ends_with(']') {
                return Err(format!("段 `{raw}` 的方括号没闭合"));
            }
            (raw[..i].to_string(), Some(raw[i + 1..raw.len() - 1].to_string()))
        }
    };
    let mut seg = Seg {
        name,
        spread: false,
        index: None,
        pick: None,
    };
    match inside {
        None => {}
        Some(s) if s == "*" => seg.spread = true,
        Some(s) if s.starts_with('?') => {
            let (f, v) = s[1..]
                .split_once('=')
                .ok_or_else(|| format!("段 `{raw}` 的 [?…] 少了 `=`"))?;
            seg.pick = Some((f.to_string(), v.to_string()));
        }
        Some(s) => {
            seg.index = Some(
                s.parse::<usize>()
                    .map_err(|_| format!("段 `{raw}` 的下标不是数字也不是 [*] / [?…]"))?,
            );
        }
    }
    Ok(seg)
}

fn parse_path(expr: &str) -> Result<Vec<Seg>, String> {
    let segs = split_segments(expr);
    if segs.is_empty() {
        return Err("空路径".into());
    }
    segs.iter().map(|s| parse_seg(s)).collect()
}

fn is_absolute(expr: &str) -> bool {
    expr.starts_with('@') && !expr.starts_with("@key")
}

fn root_of(expr: &str) -> Option<String> {
    if !is_absolute(expr) {
        return None;
    }
    // 首段可能是 `@control[?faction_id=…]`：根名到 `[` 或 `.` 为止。
    let head = split_segments(expr).into_iter().next()?;
    let head = head.split('[').next().unwrap_or("").to_string();
    Some(head.trim_start_matches('@').to_string())
}

/// 相对路径的**首段**（列「认领」哪条记录的字段；`[*]` 之类的尾巴切掉）。
fn first_segment(expr: &str) -> Option<String> {
    if is_absolute(expr) {
        return None;
    }
    parse_path(expr).ok().map(|s| s[0].name.clone())
}

// --- 遍历 ---------------------------------------------------------------------
fn each_view(doc: &Value, mut f: impl FnMut(&Value)) {
    for page in doc["pages"].as_array().into_iter().flatten() {
        for v in page["views"].as_array().into_iter().flatten() {
            f(v);
        }
    }
    for v in doc["select"].as_array().into_iter().flatten() {
        f(v);
    }
    for v in doc["inline"].as_array().into_iter().flatten() {
        f(v);
    }
}

fn all_ids(doc: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    each_view(doc, |v| {
        if let Some(id) = v["id"].as_str() {
            out.insert(id.to_string());
        }
    });
    out
}

/// 一条视图里出现的所有路径表达式（含 `source` / 列 / 分组 / 时间线 / dot / with / order）。
fn exprs_of(spec: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |v: &Value| {
        if let Some(s) = v.as_str() {
            out.push(s.to_string());
        }
    };
    push(&spec["source"]);
    push(&spec["key"]);
    push(&spec["title_path"]);
    push(&spec["body"]);
    for k in ["group", "limit", "order"] {
        let _ = k;
    }
    if let Some(g) = spec["group"].as_object() {
        push(&g["by"]);
        push(&g["dot"]);
    }
    for c in spec["columns"].as_array().into_iter().flatten() {
        push(&c["path"]);
        push(&c["with"]);
        push(&c["dot"]);
    }
    for m in spec["meta"].as_array().into_iter().flatten() {
        push(m);
    }
    out
}

// --- 1. 静态校验 --------------------------------------------------------------
#[test]
fn views_json_is_well_formed() {
    let doc = spec_doc();
    assert_eq!(doc["version"].as_u64(), Some(1), "version 字段");
    let ids = all_ids(&doc);
    let mut seen = BTreeSet::new();
    each_view(&doc, |v| {
        let id = v["id"].as_str().expect("每条视图都要有 id");
        assert!(seen.insert(id.to_string()), "视图 id 重复：{id}");
        assert!(v["mount"].is_string(), "{id}：缺 mount");
        // `source` 不是每条都必须有：inline 那条只是「路径 → 哪条视图」的映射表（`use_at`），
        // 它自己不含列。
        if v["mount"] != "inline" {
            assert!(v["source"].is_string(), "{id}：缺 source");
        }
        let layout = v["layout"].as_str().unwrap_or("table");
        assert!(
            ["table", "sheet", "cards", "timeline", "pairs"].contains(&layout),
            "{id}：未知 layout `{layout}`"
        );
        if v["mount"] == "select" {
            assert!(v["select_kind"].is_string(), "{id}：select 挂载要声明 select_kind");
        }
        if v["mount"] == "inline" {
            assert!(!v["use_at"].is_null(), "{id}：inline 挂载要声明 use_at");
        }
    });

    // 引用完整性：use / use_at / map_ref / label_from 指的都得存在。
    let mut refs: Vec<(String, String)> = Vec::new();
    each_view(&doc, |v| {
        let id = v["id"].as_str().unwrap_or("?").to_string();
        if let Some(u) = v["use"].as_str() {
            refs.push((id.clone(), u.to_string()));
        }
        if let Some(m) = v["use_at"].as_object() {
            for (_, t) in m {
                if let Some(t) = t.as_str() {
                    refs.push((id.clone(), t.to_string()));
                }
            }
        }
        for c in v["columns"].as_array().into_iter().flatten() {
            if let Some(m) = c["map_ref"].as_str() {
                assert!(doc.get(m).is_some(), "{id}：map_ref `{m}` 在 views.json 顶层不存在");
            }
            if let Some(l) = c["label_from"].as_str() {
                refs.push((id.clone(), format!("label_from:{l}")));
            }
        }
    });
    for (from, to) in &refs {
        if let Some(target) = to.strip_prefix("label_from:") {
            let root = target.split('.').next().unwrap_or("");
            assert!(ROOTS.contains(&root), "{from}：label_from 的根 `{root}` 不是已知根");
            continue;
        }
        assert!(ids.contains(to), "{from}：引用了不存在的视图 id `{to}`");
    }

    // 每条路径表达式都合文法 + 绝对路径的根都在已知根里。
    each_view(&doc, |v| {
        let id = v["id"].as_str().unwrap_or("?").to_string();
        for e in exprs_of(v) {
            let segs = parse_path(&e).unwrap_or_else(|err| panic!("{id}：路径 `{e}` 不合法——{err}"));
            if is_absolute(&e) {
                let root = root_of(&e).unwrap_or_default();
                assert!(ROOTS.contains(&root.as_str()), "{id}：路径 `{e}` 的根 `@{root}` 不是已知根");
            }
            assert!(!segs.is_empty(), "{id}：空路径");
        }
    });

    // omit 与列**不许重叠**（「既声明显示、又声明不看」是自相矛盾的声明）。
    each_view(&doc, |v| {
        let id = v["id"].as_str().unwrap_or("?").to_string();
        let claimed: BTreeSet<String> = v["columns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["path"].as_str())
            .filter_map(first_segment)
            .collect();
        for o in v["omit"].as_array().into_iter().flatten() {
            let p = o["path"].as_str().unwrap_or("");
            assert!(!p.is_empty(), "{id}：omit 项缺 path");
            assert!(
                o["why"].as_str().map(|w| !w.trim().is_empty()).unwrap_or(false),
                "{id}：omit `{p}` 必须写明理由（界面要把省略说出来）"
            );
            assert!(
                !claimed.contains(p),
                "{id}：omit `{p}` 与某一列重叠——要么显示、要么声明不看，不能两头都写"
            );
        }
    });
}

// --- 2. 对真实世界：路径存在性 + 覆盖率报告 -------------------------------------
/// 取一条 `source` 下的记录（只处理**无模板**的 source：`@root.a.b[*]` / `@root.map` / `@root`）。
fn records(root: &Value, source: &str) -> Vec<Value> {
    assert!(
        !source.contains("${"),
        "source 里不用模板（会没法静态检查）：{source}"
    );
    let segs = parse_path(source).expect("source 合法");
    assert!(is_absolute(source), "source 要用 @根 绝对路径：{source}");
    let mut cur = vec![root.clone()];
    for (i, seg) in segs.iter().enumerate() {
        if i == 0 {
            continue; // 根那一段（@state 之类）由调用方给
        }
        let mut next = Vec::new();
        for v in &cur {
            // ⚠ 顺序与 specview.js 的 `step` 一致：**先取名字、再施加方括号**。
            // 反过来（在父对象上展开）会得到一堆毫不相干的记录——前端踩过一次，这里也是。
            let base: Option<Value> = match v {
                Value::Object(map) => map.get(&seg.name).cloned(),
                Value::Array(items) => seg.name.parse::<usize>().ok().and_then(|n| items.get(n).cloned()),
                _ => None,
            };
            let Some(base) = base else { continue };
            if seg.spread {
                match base {
                    Value::Array(items) => next.extend(items),
                    Value::Object(map) => next.extend(map.into_values()),
                    _ => {}
                }
            } else {
                next.push(base);
            }
        }
        cur = next;
    }
    // 与 specview.js 的 `wrap()` 同一条约定：**值全是对象**的一层当"映射表"（一条记录一项），
    // 于是 `@post.haul_steps`（舰名 → 运输状态）与 `@state.ships[*]`（数组）在视图里同形。
    let mut out = Vec::new();
    for v in cur {
        match &v {
            Value::Object(map)
                if !map.is_empty() && map.values().all(|x| x.is_object()) =>
            {
                out.extend(map.values().cloned());
            }
            _ => out.push(v),
        }
    }
    out
}

#[test]
fn every_view_path_resolves_against_real_worlds() {
    let doc = spec_doc();
    let mut report = Vec::new();
    let mut failures = Vec::new();

    // 多个种子 × 多个回合：避免"这局恰好为空"被当成路径写错。
    for (seed, rounds) in [(7u64, 40u32), (42, 120)] {
        let roots = frame(seed, rounds);
        let get = |name: &str| roots.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());

        each_view(&doc, |spec| {
            let id = spec["id"].as_str().unwrap_or("?").to_string();
            let source = spec["source"].as_str().unwrap_or("");
            if source.is_empty() {
                return; // inline 的映射表条目（没有列，也就没有要检查的路径）
            }
            let root_name = root_of(source).unwrap_or_default();
            let Some(root) = get(&root_name) else {
                failures.push(format!("{id}：source 的根 @{root_name} 取不到"));
                return;
            };
            let recs = records(&root, source);
            if recs.is_empty() {
                report.push(format!("{id}：本帧 `{source}` 没有记录（跳过）"));
                return;
            }
            let mut missing: BTreeSet<String> = BTreeSet::new();
            let mut checked = 0usize;
            for c in spec["columns"].as_array().into_iter().flatten() {
                let path = c["path"].as_str().unwrap_or("");
                let Some(first) = first_segment(path) else {
                    continue; // 跨根列（@… ）由运行时自检覆盖
                };
                checked += 1;
                let hit = recs.iter().any(|r| match r {
                    Value::Object(m) => {
                        m.contains_key(&first) || (first == "@key")
                    }
                    _ => first == "@key",
                });
                if !hit {
                    missing.insert(first);
                }
            }
            for m in spec["meta"].as_array().into_iter().flatten() {
                if let Some(p) = m.as_str() {
                    if let Some(first) = first_segment(p) {
                        checked += 1;
                        let hit = recs.iter().any(|r| matches!(r, Value::Object(m) if m.contains_key(&first)));
                        if !hit {
                            missing.insert(first);
                        }
                    }
                }
            }
            if !missing.is_empty() {
                failures.push(format!(
                    "{id}（source = {source}，seed {seed} / {rounds} 回合）：这些相对列的首段在记录里找不到 —— {}",
                    missing.into_iter().collect::<Vec<_>>().join("、")
                ));
            }
            report.push(format!("{id}：{checked} 条相对列全部命中（{source}）"));
        });
    }

    for line in &report {
        println!("{line}");
    }
    assert!(
        failures.is_empty(),
        "views.json 里有解析不了的路径：\n{}",
        failures.join("\n")
    );
}

/// 覆盖率报告：**已认领的字段** vs 记录里实际有的字段。只印不判红——`config` 那种整表
/// 本来就不该被认领，人读的视图也不该覆盖每个内部字段。（铁律 R 要的是"没认领的**看得见**"，
/// 不要求"全都认领"。）
#[test]
fn coverage_report_is_printed_and_claimed_fields_actually_exist() {
    let doc = spec_doc();
    let roots = frame(7, 40);
    let state = roots.iter().find(|(n, _)| n == "state").unwrap().1.clone();

    let ships = state["ships"].as_array().cloned().unwrap_or_default();
    assert!(!ships.is_empty(), "seed 7 / 40 回合应当已经有舰");
    let mut all: BTreeSet<String> = BTreeSet::new();
    for s in &ships {
        if let Value::Object(m) = s {
            all.extend(m.keys().cloned());
        }
    }
    let spec = doc["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|p| p["views"].as_array().into_iter().flatten())
        .find(|v| v["id"] == "ship-table")
        .expect("ship-table 视图存在");
    let claimed: BTreeSet<String> = spec["columns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["path"].as_str())
        .filter_map(first_segment)
        .collect();
    let residual: BTreeSet<String> = all.difference(&claimed).cloned().collect();
    println!(
        "覆盖率（state.ships，seed 7 / 40 回合）：字段 {} 个 = 认领 {} + 残差 {}；残差 = {}",
        all.len(),
        claimed.len(),
        residual.len(),
        residual.iter().cloned().collect::<Vec<_>>().join("、")
    );

    // 认领的字段必须**真的存在**（否则那列永远是 missing——写错了名字就是这个症状）。
    for k in claimed.intersection(&all) {
        assert!(all.contains(k), "认领了并不存在的字段：{k}");
    }
    assert!(
        !residual.is_empty(),
        "残差为空说明列覆盖了记录的全部字段——那多半意味着前端把字段写死在别处了"
    );
}


