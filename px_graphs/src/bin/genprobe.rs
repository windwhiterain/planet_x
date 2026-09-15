//! **临时**仪器：把 `px_ops::generate` 造出来的贴图与 `target/legacy-gen` 的基线逐字节对账。
//!
//! 它守的是"搬运没改内容"这一条，用完即删。判据是**八份全部逐字节相同**：
//! 任何一项字节数或 SHA256 不同 ⇒ 打印差异并以非零退出码结束。
//!
//! 跑法（worktree 根目录）：`cargo run -p px_graphs --bin genprobe`
//!
//! 基线是怎么来的：`px_render/tests/spike_legacy_dump.rs`（渲染器**自己**那套生成器
//! 原样倒出来的）。这里读的是**同一批**产物（`target/pcg/*/manifest.json` 里的键），
//! 所以两边除生成器以外没有任何自由度。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_ops::generate::{self, Palette, TextureData, TextureFormat};
use px_ops::noise::fnv1a;
use px_ops::{GraphSpec, begin};
use px_protocol::wire::DType;
use sha2::{Digest, Sha256};

const SEA_LEVEL: f32 = 0.52;
const STAR_FACE: u32 = 512;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graphs 住在 workspace 下")
        .to_path_buf()
}

fn artifact_of(graph: &str, node: &str) -> PathBuf {
    let root = root();
    let text = std::fs::read_to_string(root.join("target/pcg").join(graph).join("manifest.json"))
        .unwrap_or_else(|err| panic!("读不了 {graph} 的清单：{err}（先烘这张图）"));
    let entries: serde_json::Value = serde_json::from_str(&text).expect("清单不是 JSON");
    let key = entries
        .as_array()
        .expect("清单应当是数组")
        .iter()
        .find(|entry| entry["node"] == node)
        .unwrap_or_else(|| panic!("{graph} 的清单里没有节点 {node}"))["key"]
        .as_str()
        .expect("key 应当是字符串")
        .to_string();
    root.join("target/pcg/ab")
        .join(&key[..2])
        .join(format!("{key}.pxart"))
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn dtype_of(format: TextureFormat) -> DType {
    match format {
        TextureFormat::Rgba8Srgb => DType::U8,
        TextureFormat::Rgba16Float => DType::U16,
    }
}

/// 基线的对账表：`名字 字节数 sha256`。
struct Baseline {
    name: String,
    bytes: u64,
    sha: String,
}

fn read_baseline() -> Vec<Baseline> {
    let path = root().join("target/legacy-gen/hashes.txt");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next().expect("基线行缺名字").to_string();
            let bytes: u64 = parts
                .next()
                .expect("基线行缺字节数")
                .parse()
                .expect("基线行的字节数不是整数");
            let sha = parts.next().expect("基线行缺 SHA256").to_string();
            Baseline { name, bytes, sha }
        })
        .collect()
}

/// 第一处不同的偏移（两侧都拿得到字节时）。对不上时打印它是判据要求的一部分：
/// 光说"哈希不一样"没法查是取数口径错了还是排布错了。
fn first_difference(ours: &[u8], theirs: &[u8]) -> String {
    if ours.len() != theirs.len() {
        return format!("字节数就不同（新 {} / 基线 {}）", ours.len(), theirs.len());
    }
    for (offset, (mine, base)) in ours.iter().zip(theirs.iter()).enumerate() {
        if mine != base {
            let pixel = offset / 4;
            let channel = offset % 4;
            return format!(
                "第一处不同在偏移 {offset}（第 {pixel} 个 texel 的第 {channel} 通道）：新 {mine} / 基线 {base}"
            );
        }
    }
    "字节完全相同（那么差异只可能在哈希函数上）".to_string()
}

fn main() {
    // ---- 1. 读场（键取清单，投影由产物的 AssetKind 决定） ----
    let height = generate::load_field(&artifact_of("planet", "height").display().to_string())
        .expect("载入行星高度场失败");
    let mixed = generate::load_field(&artifact_of("clouds", "mixed").display().to_string())
        .expect("载入云覆盖度失败");
    let slopes = ["slope_x", "slope_y", "slope_z"].map(|node| {
        generate::load_field(&artifact_of("clouds", node).display().to_string())
            .unwrap_or_else(|err| panic!("载入 {node} 失败：{err}"))
    });

    println!(
        "输入 height：{}×{}｜{:?}｜值域 {:.6}..{:.6}",
        height.width,
        height.height,
        height.projection,
        height.stats().min,
        height.stats().max,
    );
    println!(
        "输入 mixed：{}×{}｜{:?}",
        mixed.width, mixed.height, mixed.projection
    );

    // ---- 2. 生成 ----
    let mut produced: BTreeMap<String, TextureData> = BTreeMap::new();
    for name in Palette::NAMES {
        let palette = Palette::parse(name).expect("色板名不对");
        let (color, glow, audit) = generate::surface_color(&height, palette, SEA_LEVEL);
        println!("--- {name}：{audit}");
        if let Some(glow) = glow {
            produced.insert(format!("glow-{name}.bin"), glow);
        }
        produced.insert(format!("color-{name}.bin"), color);
    }
    produced.insert(
        "coverage.bin".to_string(),
        generate::coverage_cube(&mixed, [&slopes[0], &slopes[1], &slopes[2]])
            .expect("覆盖度立方图造不出来"),
    );
    produced.insert("stars.bin".to_string(), generate::stars(STAR_FACE));

    // ---- 3. 写到 target/ops-gen/ 并打印 ----
    let out_dir = root().join("target/ops-gen");
    std::fs::create_dir_all(&out_dir).expect("建不了输出目录");
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (name, data) in &produced {
        let shape = data.shape();
        println!(
            "{name}：{}×{} ×{} 层｜{:?}｜mip {}｜{} 字节（形状算 {} 字节）",
            data.width,
            data.height,
            data.layers,
            data.format,
            data.levels,
            data.bytes.len(),
            shape.chain_bytes(),
        );
        std::fs::write(out_dir.join(name), &data.bytes).expect("写不了取样文件");
        println!("{name} {} {}", data.bytes.len(), sha256(&data.bytes));
        written.insert(name.clone(), data.bytes.clone());
    }

    // ---- 4. 与基线逐项比对 ----
    let baseline = read_baseline();
    println!();
    println!(
        "{:<18} {:>10} {:>10}  {:<64} {:<64} 判定",
        "名字", "字节数(新)", "字节数(基线)", "SHA256(新)", "SHA256(基线)"
    );
    let mut failed = 0_usize;
    for entry in &baseline {
        let Some(bytes) = written.get(&entry.name) else {
            println!("{:<18} {:>10} {:>10}  —— 新的一侧没造出来", entry.name, "-", entry.bytes);
            failed += 1;
            continue;
        };
        let sha = sha256(bytes);
        let ok = bytes.len() as u64 == entry.bytes && sha == entry.sha;
        if !ok {
            failed += 1;
        }
        println!(
            "{:<18} {:>10} {:>10}  {:<64} {:<64} {}",
            entry.name,
            bytes.len(),
            entry.bytes,
            sha,
            entry.sha,
            if ok { "相同" } else { "**不同**" },
        );
        if !ok {
            let path = root().join("target/legacy-gen").join(&entry.name);
            match std::fs::read(&path) {
                Ok(base) => println!("    {}", first_difference(bytes, &base)),
                Err(err) => println!("    （读不了基线 {}：{err}）", path.display()),
            }
        }
    }
    for name in written.keys() {
        if !baseline.iter().any(|entry| &entry.name == name) {
            println!("{name}：基线里没有这一项（多造了）");
            failed += 1;
        }
    }

    // ---- 5. 顺手过一遍落盘 API（键 = 内容；第二次跑应当全是命中） ----
    begin(GraphSpec {
        name: "genprobe".to_string(),
        version: 1,
        source_hash: fnv1a(include_str!("genprobe.rs")),
        width: 780,
        height: 520,
        projection: px_ops::field::Projection::Cube,
        cameras: Vec::new(),
    });
    println!();
    for (name, data) in &produced {
        let id = name.trim_end_matches(".bin");
        match generate::write_texture(id, data.shape(), &data.bytes, dtype_of(data.format)) {
            Ok(generated) => println!(
                "落盘 {:<18} {}  {} 字节  {} ms  {}",
                generated.id,
                &px_ops::hex(&generated.key)[..12],
                generated.bytes,
                generated.millis,
                if generated.hit { "命中（没重写）" } else { "写入" },
            ),
            Err(err) => {
                println!("落盘 {id} 失败：{err}");
                failed += 1;
            }
        }
    }
    let ring = generate::ring_mesh(1.30, 1.45, 384);
    match generate::write_generated_mesh("ring", &ring) {
        Ok(generated) => println!(
            "落盘 {:<18} {}  {} 字节（{} 顶点 / {} 三角形）  {} ms  {}",
            generated.id,
            &px_ops::hex(&generated.key)[..12],
            generated.bytes,
            ring.vertices(),
            ring.triangles(),
            generated.millis,
            if generated.hit { "命中（没重写）" } else { "写入" },
        ),
        Err(err) => {
            println!("落盘 ring 失败：{err}");
            failed += 1;
        }
    }

    println!();
    if failed == 0 {
        println!(
            "判据通过：{} 份与 target/legacy-gen 逐字节相同",
            baseline.len()
        );
    } else {
        println!("判据失败：{} 项对不上", failed);
        std::process::exit(1);
    }

    // ---- 6. coverage_cube 那三条校验还在不在（投影 / 行数 = 面 × 6 / 三张梯度同形） ----
    let texels = mixed.width as usize * mixed.height as usize;
    let flat = px_ops::Field::with_projection(
        mixed.width,
        mixed.height,
        mixed.data.clone(),
        px_ops::field::Projection::Equirect,
    );
    let short = px_ops::Field::with_projection(
        mixed.width,
        mixed.height - 1,
        mixed.data[..texels - mixed.width as usize].to_vec(),
        px_ops::field::Projection::CubeMap,
    );
    let small = px_ops::Field::with_projection(
        128,
        768,
        vec![0.0; 128 * 768],
        px_ops::field::Projection::CubeMap,
    );
    for (what, result) in [
        (
            "投影不是 CubeMap",
            generate::coverage_cube(&flat, [&slopes[0], &slopes[1], &slopes[2]]),
        ),
        (
            "行数 != 面 × 6",
            generate::coverage_cube(&short, [&slopes[0], &slopes[1], &slopes[2]]),
        ),
        (
            "梯度场与覆盖度不同形",
            generate::coverage_cube(&mixed, [&small, &slopes[1], &slopes[2]]),
        ),
    ] {
        match result {
            Ok(_) => {
                println!("校验缺失：{what} 竟然通过了");
                std::process::exit(1);
            }
            Err(err) => println!("校验还在（{what}）：{err}"),
        }
    }
}
