use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_graphs::params::{merge_named, schema_of};
use px_protocol::scene::{Member, PassResource, PassSpec, SceneSpec};
use serde::Deserialize;

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_ops::noise::fnv1a(include_str!("passes.rs"));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassFile {
    /// 可选：换掉文档的名字（不写就沿用基准场景的名字）。
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    resources: Vec<PassResourceFile>,
    #[serde(default)]
    passes: Vec<PassFileEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassResourceFile {
    name: String,
    format: String,
    size: String,
    #[serde(default)]
    usage: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassFileEntry {
    kind: String,
    shader: String,
    #[serde(default)]
    label: String,
    #[serde(default = "fragment_entry")]
    entry: String,
    #[serde(default)]
    reads: Vec<String>,
    /// 老形状（`--no-frame-graph`）里这一栏是**必填**的；图形模式下**必须留空** ——
    /// 目标由帧图配（乒乓对），内容配方只描述"做什么"（§128 裁决 D）。
    #[serde(default)]
    writes: Vec<String>,
    /// 传给这份 pass shader 的参数：**按名字**给，按它自己声明的结构体打包。
    /// 名字不认识 / 声明了没人给 / 类型不符 —— 三档都在**烘图时**红，与材质同一条路。
    #[serde(default)]
    params: BTreeMap<String, toml::Value>,
}

fn fragment_entry() -> String {
    "fs_main".to_string()
}

fn usage() -> String {
    "用法：passes <场景产物 .pxart> <pass 配方名> [输出路径] [--frame <帧图名>] [--no-frame-graph]\n\
     例：cargo run -p px_graphs --bin passes -- target/pcg/ab/xx/base.pxart invert\n\
     配方住在 art/passes/<名>.toml；它引用的 shader 由 cargo run -p px_graphs --bin shaders 烘。\n\
     ⚠ 基准产物带帧图时（默认），内容 pass 插在帧图的 before 与 after 之间，\
     目标由**帧图**配（乒乓对）—— 内容配方只管写「做什么」。老形状走 --no-frame-graph。"
        .to_string()
}

fn main() {
    px_ops::begin(px_ops::GraphSpec {
        name: "passdoc".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 0,
        height: 0,
        projection: px_ops::field::Projection::Cube,
        cameras: Vec::new(),
    });

    // 位置参数：<场景产物> <pass 配方> [输出路径]；开关：--frame <名> / --no-frame-graph。
    let mut positional: Vec<String> = Vec::new();
    let mut frame_name = px_graphs::frame::DEFAULT_FRAME.to_string();
    let mut with_graph = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-frame-graph" => with_graph = false,
            "--frame" => {
                frame_name = args.next().unwrap_or_else(|| panic!("--frame 后面要跟帧图名"));
            }
            other => positional.push(other.to_string()),
        }
    }
    let (Some(input), Some(recipe)) = (positional.first(), positional.get(1)) else {
        panic!("{}", usage());
    };
    let (input, recipe) = (input.clone(), recipe.clone());
    let out = positional.get(2).cloned();

    let path = PathBuf::from("art")
        .join("passes")
        .join(format!("{recipe}.toml"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("读不了 {}：{err}\n{}", path.display(), usage()));
    let file: PassFile = toml::from_str(&text)
        .unwrap_or_else(|err| panic!("{} 解不开：{err}", path.display()));

    let mut spec: SceneSpec = px_protocol::scene::read_scene(Path::new(&input))
        .unwrap_or_else(|err| panic!("读不了场景产物 {input}：{err}"));

    if let Some(name) = &file.name {
        spec.name = name.clone();
    }

    if with_graph && !file.resources.is_empty() {
        panic!(
            "pass 配方 '{}' 自己声明了 {} 个 resources：图形模式下目标由**帧图**配             （{} 声明了 {} 个）—— 内容配方只描述做什么。要走老形状请加 --no-frame-graph",
            recipe,
            file.resources.len(),
            frame_name,
            spec.resources.len()
        );
    }
    if !with_graph {
        // 老形状：把配方声明的 resources **并**到基准产物已有的那些上（按名字去重）。
        // ⚠ 不能直接覆盖：基准产物可能是别的东西烘的、自己带着 resources，
        //    覆盖掉它就会让它的 pass 引用一个没声明的名字（`check` 会拒，但那时
        //    报的是"没声明"，离真正的原因已经很远）。
        for resource in &file.resources {
            if spec
                .resources
                .iter()
                .any(|existing| existing.name == resource.name)
            {
                continue;
            }
            spec.resources.push(PassResource {
                name: resource.name.clone(),
                format: resource.format.clone(),
                size: resource.size.clone(),
                layers: 1,
                usage: resource.usage.clone(),
            });
        }
    }

    let mut passes: Vec<PassSpec> = Vec::new();
    for entry in &file.passes {
        let key = px_ops::manifest_key_of("shaders", &entry.shader).unwrap_or_else(|err| {
            panic!(
                "pass 要的 shader '{}' 不在 shaders 清单里：{err}\n\
                 先跑 cargo run -p px_graphs --bin shaders",
                entry.shader
            )
        });
        let label = if entry.label.is_empty() {
            entry.shader.clone()
        } else {
            entry.label.clone()
        };
        let member = Member::new("shaders", &entry.shader, &key);
        // 参数按**这份 shader 自己的契约**透传：烘图时就把三档（名字不认识 / 声明了没人给 /
        // 类型不符）全拦下来，不等装载时才拒 —— 那时候报的是渲染器的错，离改配方已经很远。
        let layout = schema_of(&member, &px_ops::cache_root())
            .unwrap_or_else(|err| panic!("pass '{label}'：{err}"));
        let params = merge_named(
            &format!("pass '{label}'"),
            &entry.params,
            &[],
            &layout,
            BTreeMap::new(),
        )
        .unwrap_or_else(|err| panic!("{err}"));
        println!(
            "pass {label}：{}｜shader {}（{}）｜读 [{}]｜写 [{}]｜参数 {} 个{}",
            entry.kind,
            entry.shader,
            &key[..key.len().min(12)],
            entry.reads.join(" / "),
            entry.writes.join(" / "),
            params.len(),
            if params.is_empty() {
                String::new()
            } else {
                format!(
                    "（{}）",
                    params
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" / ")
                )
            },
        );
        passes.push(PassSpec {
            kind: entry.kind.clone(),
            shader: Some(member),
            label,
            entry: entry.entry.clone(),
            reads: entry.reads.clone(),
            writes: entry.writes.clone(),
            params,
            // 帧图那几栏（§125）留空 ⇒ 这一档出的仍是**老形状**的 pass，
            // 与冻在 `target/oracle/pxart-frozen/` 里那六份逐字节同形。
            // 要出几何 pass 时从这里往后加（烘图侧改的那一件事单列，不混在这一步里）。
            draws: Vec::new(),
            vertex_shader: String::new(),
            vertex_entry: String::new(),
            render: String::new(),
            depth_target: None,
            cube_face: None,
        });
    }
    // ---- 落位（§128 裁决 D）----
    if !with_graph {
        // 兼容逃生门：老形状就是**追加**在末尾（内容 pass 自己写 `view`）。
        let mut all = spec.passes.clone();
        all.extend(passes);
        spec.passes = all;
    } else {
        let frame = px_graphs::frame::load(&frame_name).unwrap_or_else(|err| panic!("{err}"));
        // 先核对基准产物是不是这张帧图烘的：不是就**当场拒**（说清期望什么、实际是什么）。
        px_graphs::frame::verify(&spec, &frame, &frame_name)
            .unwrap_or_else(|err| panic!("{err}"));
        let insert_at = frame.before.len();
        let declared: Vec<&str> = spec
            .resources
            .iter()
            .map(|resource| resource.name.as_str())
            .collect();
        let (first, second) = (frame.chain_color[0].clone(), frame.chain_color[1].clone());
        // 内容链的接线：上一段的输出 = 这一段的输入，两个缓冲轮流用。
        // ⚠ 自动接线的每一步都**打出来**：自动算可以，"算完不吭声"不行（§73）。
        let mut current = first.clone();
        for (index, pass) in passes.iter_mut().enumerate() {
            // 配方自己点名了目标 ⇒ 当场拒，并说清是哪张帧图、它声明了哪些目标、该怎么改。
            if !pass.reads.is_empty() || !pass.writes.is_empty() {
                panic!(
                    "内容 pass '{}' 自己点名了目标（reads [{}] / writes [{}]）：\
                     帧图 '{frame_name}' 由**帧图**配目标（乒乓对 {} / {}）。\
                     内容配方只描述**做什么** —— 去掉 reads/writes 再来；\
                     要走老形状（内容 pass 自己写 view）请加 --no-frame-graph。\
                     帧图声明的目标：[{}]",
                    pass.label,
                    pass.reads.join(" / "),
                    pass.writes.join(" / "),
                    first,
                    second,
                    declared.join(" / ")
                );
            }
            let next = if index % 2 == 0 {
                second.clone()
            } else {
                first.clone()
            };
            pass.reads = vec![current.clone()];
            pass.writes = vec![next.clone()];
            println!(
                "内容 pass '{}'（第 {} 笔）：读 {} 写 {}（帧图 '{frame_name}' 配的）",
                pass.label,
                index + 1,
                current,
                next
            );
            current = next;
        }
        // blit 读链尾那一个（链空时就是帧图绘制段的输出）。
        // ⚠ blit 的位置按**插入前**那个数组算：`before.len()` 就是 `after` 的第一条。
        //    （插进去之后它才右移 —— 拿插入后的下标去查插入前的数组，就会差出内容 pass 的条数。）
        let blit_at = insert_at;
        if let Some(blit) = spec.passes.get_mut(blit_at) {
            println!(
                "帧图 '{}' 的 '{}' 读 {}（链尾）",
                frame_name, blit.label, current
            );
            blit.reads = vec![current.clone()];
        } else {
            panic!(
                "帧图 '{frame_name}' 里第 {blit_at} 条之后没有 pass 了：基准产物只有 {} 条，插不进内容 pass",
                spec.passes.len()
            );
        }
        spec.passes.splice(insert_at..insert_at, passes);
    }
    spec.check().unwrap_or_else(|err| panic!("这份 pass 表不成立：{err}"));

    let spec_json = serde_json::to_string(&spec).unwrap_or_else(|err| panic!("{err}"));
    let member_keys = spec
        .members()
        .iter()
        .map(|member| member.key.clone())
        .collect::<Vec<_>>();
    let key = px_ops::scene_key(&spec_json, &member_keys);
    let artifact = match out {
        Some(path) => PathBuf::from(path),
        None => px_protocol::scene::cas_path(&px_ops::cache_root(), &px_ops::hex(&key))
            .unwrap_or_else(|err| panic!("{err}")),
    };
    let bytes =
        px_protocol::scene::write_scene(&artifact, &spec, px_ops::noise::fnv1a(&spec_json))
            .unwrap_or_else(|err| panic!("{err}"));

    println!("{}", spec.audit());
    println!(
        "产物 passdoc -> {}（{}｜{} 字节）",
        artifact.display(),
        px_ops::hex_short(&key),
        bytes
    );
    println!("请求：px_render --scene {}", artifact.display());
}
