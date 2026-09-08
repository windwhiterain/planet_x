//! CLI rendering: a colored ASCII star map plus structured tables
//! (rendered with `comfy-table`) and a compact report.

use colored::{Color, Colorize};
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, ContentArrangement, Table};
use crate::model::*;

const COLS: usize = 66;
const ROWS: usize = 22;

/// Distinct marker colors per faction id.
const PALETTE: [Color; 9] = [
    Color::BrightBlue,
    Color::Blue,
    Color::BrightCyan,
    Color::Red,
    Color::Magenta,
    Color::Yellow,
    Color::Green,
    Color::White,
    Color::BrightMagenta,
];

/// Sub-linear (root) transform for plotting so the dense inner planets don't
/// collapse onto the sun while far Kuiper-belt dwarfs stay on-screen.
fn tx(v: f64) -> f64 {
    if v >= 0.0 {
        v.powf(0.4)
    } else {
        -((-v).powf(0.4))
    }
}

fn body_letter(id: BodyId) -> char {
    (b'A' + (id % 26) as u8) as char
}

fn map_to_grid(p: [f64; 2], minx: f64, spanx: f64, maxy: f64, spany: f64) -> Option<(usize, usize)> {
    if spanx <= 0.0 || spany <= 0.0 || !p[0].is_finite() || !p[1].is_finite() {
        return None;
    }
    let col = (((p[0] - minx) / spanx) * (COLS - 1) as f64).round() as isize;
    let row = (((maxy - p[1]) / spany) * (ROWS - 1) as f64).round() as isize;
    if (0..COLS as isize).contains(&col) && (0..ROWS as isize).contains(&row) {
        Some((col as usize, row as usize))
    } else {
        None
    }
}

fn faction_color(fid: FactionId) -> Color {
    PALETTE[(fid as usize) % PALETTE.len()]
}

fn faction_name(state: &State, id: FactionId) -> String {
    state
        .faction(id)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| format!("#{}", id))
}

fn building_label(config: &GameConfig, b: &Building) -> String {
    let spec = config.building_spec(&b.kind);
    match &b.resource {
        Some(r) => format!("{}·{}×{:.1}", spec.label, config.resource_name(r), b.deployed),
        None => format!("{}×{:.1}", spec.label, b.deployed),
    }
}

/// Render a colored ASCII map of the system at the current time.
pub fn render_map(state: &State) -> String {
    let mut pts: Vec<[f64; 2]> = state
        .bodies
        .iter()
        .map(|b| {
            let p = b.orbit.position(state.time_month as f32);
            [tx(p[0]), tx(p[1])]
        })
        .collect();
    for s in &state.ships {
        pts.push([tx(s.position[0]), tx(s.position[1])]);
    }
    pts.push([tx(0.0), tx(0.0)]); // the sun

    let minx = pts.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let maxx = pts.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
    let miny = pts.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let maxy = pts.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
    let spanx = (maxx - minx).max(1e-3);
    let spany = (maxy - miny).max(1e-3);

    let mut grid: Vec<String> = vec![" ".to_string(); COLS * ROWS];

    let put = |grid: &mut Vec<String>, c: usize, r: usize, s: String| {
        grid[r * COLS + c] = s;
    };

    if let Some((c, r)) = map_to_grid([tx(0.0), tx(0.0)], minx, spanx, maxy, spany) {
        put(&mut grid, c, r, "@".yellow().to_string());
    }
    for b in &state.bodies {
        let p = b.orbit.position(state.time_month as f32);
        if let Some((c, r)) = map_to_grid([tx(p[0]), tx(p[1])], minx, spanx, maxy, spany) {
            put(&mut grid, c, r, body_letter(b.id).to_string().cyan().to_string());
        }
    }
    for s in &state.ships {
        if let Some((c, r)) = map_to_grid([tx(s.position[0]), tx(s.position[1])], minx, spanx, maxy, spany) {
            let ch = state.faction(s.faction_id).map(|f| f.color).unwrap_or('?');
            let color = faction_color(s.faction_id);
            put(&mut grid, c, r, ch.to_string().color(color).to_string());
        }
    }

    let mut out = String::new();
    out.push_str(&format!("┌{}┐\n", "─".repeat(COLS)));
    for row in 0..ROWS {
        out.push('│');
        for c in 0..COLS {
            out.push_str(&grid[row * COLS + c]);
        }
        out.push_str("│\n");
    }
    out.push_str(&format!("└{}┘\n", "─".repeat(COLS)));
    out.push_str(&render_legend(state));
    out
}

fn render_legend(state: &State) -> String {
    let mut out = String::new();
    let mut bodies: Vec<String> = state
        .bodies
        .iter()
        .map(|b| format!("{}={}", body_letter(b.id).to_string().cyan(), b.name))
        .collect();
    bodies.push(format!("{}={}", "@".yellow(), "太阳"));
    out.push_str(&format!("  天体: {}\n", bodies.join("  ")));

    let facs: Vec<String> = state
        .factions
        .iter()
        .map(|f| {
            format!(
                "{}=[{}]",
                f.color.to_string().color(faction_color(f.id)),
                f.name
            )
        })
        .collect();
    out.push_str(&format!("  势力: {}\n", facs.join("  ")));
    out
}

fn heading(s: &str) -> String {
    format!("\n── {} ──\n", s.bold().green())
}

fn new_table() -> Table {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL);
    t.set_content_arrangement(ContentArrangement::Dynamic);
    t
}

fn cell(s: impl Into<String>) -> Cell {
    Cell::new(s.into())
}

fn fmt_pos(p: [f64; 2]) -> String {
    format!("({:+.1}, {:+.1})", p[0], p[1])
}

fn fmt_resource_map(config: &GameConfig, m: &ResourceMap) -> String {
    if m.is_empty() {
        return "—".to_string();
    }
    let mut items: Vec<String> = m
        .iter()
        .filter(|(_, v)| **v >= 0.05)
        .map(|(k, v)| format!("{}{:.1}", config.resource_name(k), v))
        .collect();
    items.sort();
    items.join(" ")
}

/// Structured report of the current state.
pub fn render_summary(state: &State, config: &GameConfig) -> String {
    let mut s = String::new();

    s.push_str(&format!(
        "{}  第 {} 回合 ｜ 时间 {:.0} 个月\n",
        "行星X".cyan().bold(),
        state.round,
        state.time_month,
    ));

    // --- Bodies ---
    s.push_str(&heading("天体"));
    let mut t = new_table();
    t.set_header(vec!["编号", "名称", "位置 (AU)", "状态", "城市"]);
    for b in &state.bodies {
        let p = b.orbit.position(state.time_month as f32);
        let cities: Vec<&str> = state
            .cities
            .iter()
            .filter(|c| c.body_id == b.id)
            .map(|c| c.name.as_str())
            .collect();
        t.add_row(vec![
            cell(body_letter(b.id).to_string().cyan().to_string()),
            cell(b.name.clone()),
            cell(fmt_pos(p)),
            cell(if b.settlement.is_some() { "定居点" } else { "无人" }),
            cell(if cities.is_empty() { "—".to_string() } else { cities.join("、") }),
        ]);
    }
    s.push_str(&t.to_string());

    // --- Cities ---
    s.push_str(&heading("城市"));
    let mut t = new_table();
    t.set_header(vec!["名称", "控制势力", "人口", "防御", "建筑 (面积)"]);
    for c in &state.cities {
        let buildings: String = c
            .buildings
            .iter()
            .map(|b| building_label(config, b))
            .collect::<Vec<_>>()
            .join("  ");
        t.add_row(vec![
            cell(c.name.clone()),
            cell(faction_name(state, c.faction_id)),
            cell(c.population.to_string()),
            cell(format!("{:.0}", c.defense)),
            cell(buildings),
        ]);
    }
    s.push_str(&t.to_string());

    // --- Ships ---
    s.push_str(&heading("飞船"));
    let mut t = new_table();
    t.set_header(vec!["ID", "名称", "势力", "位置 (AU)", "目标", "航速", "耐久%"]);
    for sh in &state.ships {
        let spec = config.ship_spec(&sh.class);
        let target = match sh.target {
            Some(ShipTarget::Ship(id)) => format!("船#{}", id),
            Some(ShipTarget::City(id)) => format!("城#{}", id),
            Some(ShipTarget::Body(id)) => format!("体#{}", id),
            Some(ShipTarget::Position(_)) => "点".to_string(),
            None => "待命".to_string(),
        };
        let color = faction_color(sh.faction_id);
        t.add_row(vec![
            cell(format!("#{}", sh.id)),
            cell(format!("{}-{}", spec.label, sh.faction_id)),
            cell(faction_name(state, sh.faction_id).color(color).to_string()),
            cell(fmt_pos(sh.position)),
            cell(target),
            cell(format!("{:.2}", spec.speed)),
            cell(format!("{:.0}", (sh.hull / spec.hull * 100.0).clamp(0.0, 100.0))),
        ]);
    }
    if !state.ships.is_empty() {
        s.push_str(&t.to_string());
    } else {
        s.push_str(&"  无飞船在轨".dimmed().to_string());
    }

    // --- Factions & diplomacy ---
    s.push_str(&heading("势力与外交"));
    let mut t = new_table();
    t.set_header(vec!["势力", "资源", "交战对象"]);
    for f in &state.factions {
        let enemies: Vec<String> = f
            .relations
            .iter()
            .filter(|(_, v)| **v <= config.combat.war_threshold)
            .map(|(id, v)| format!("{} ({:.0})", faction_name(state, *id), v))
            .collect();
        t.add_row(vec![
            cell(format!(
                "{}=[{}]",
                f.color.to_string().color(faction_color(f.id)),
                f.name
            )),
            cell(fmt_resource_map(config, &f.resources)),
            cell(if enemies.is_empty() { "—".to_string() } else { enemies.join(" ") }),
        ]);
    }
    s.push_str(&t.to_string());

    s
}
