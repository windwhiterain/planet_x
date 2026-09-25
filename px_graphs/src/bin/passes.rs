use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_protocol::scene::{Member, PassResource, PassSpec, SceneSpec};
use px_scene::contract::{merge_named, shader_parts_of};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassFile {
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
    #[serde(default)]
    writes: Vec<String>,
    #[serde(default)]
    params: BTreeMap<String, toml::Value>,
}

fn fragment_entry() -> String {
    "fs_main".to_string()
}

fn pass_spec(
    entry: &PassFileEntry,
    label: String,
    member: Member,
    params: BTreeMap<String, px_protocol::scene::Value>,
) -> PassSpec {
    println!(
        "pass {label}：{}｜shader {}（{}）｜读 [{}]｜写 [{}]｜参数 {} 个{}",
        entry.kind,
        entry.shader,
        &member.key[..member.key.len().min(12)],
        entry.reads.join(" / "),
        entry.writes.join(" / "),
        params.len(),
        if params.is_empty() {
            String::new()
        } else {
            format!(
                "（{}）",
                params.keys().cloned().collect::<Vec<_>>().join(" / ")
            )
        },
    );
    PassSpec {
        kind: entry.kind.clone(),
        shader: Some(member),
        label,
        entry: entry.entry.clone(),
        reads: entry.reads.clone(),
        writes: entry.writes.clone(),
        params,
        draws: Vec::new(),
        vertex_shader: String::new(),
        vertex_entry: String::new(),
        render: String::new(),
        depth_target: None,
        cube_face: None,
        viewport: None,
    }
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
    px_cook::apply_store_args().unwrap_or_else(|err| panic!("{err}"));
    let _graph = px_cook::begin(px_cook::GraphSpec {
        name: "passdoc".to_string(),
    });

    let mut positional: Vec<String> = Vec::new();
    let mut frame_name = px_scene::frame::DEFAULT_FRAME.to_string();
    let mut with_graph = true;
    let mut args = px_cook::args_without_store()
        .unwrap_or_else(|err| panic!("{err}"))
        .into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-frame-graph" => with_graph = false,
            "--frame" => {
                frame_name = args
                    .next()
                    .unwrap_or_else(|| panic!("--frame 后面要跟帧图名"));
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
    let file: PassFile =
        toml::from_str(&text).unwrap_or_else(|err| panic!("{} 解不开：{err}", path.display()));

    let mut spec: SceneSpec = px_protocol::scene::read_scene(Path::new(&input))
        .unwrap_or_else(|err| panic!("读不了场景产物 {input}：{err}"));

    if let Some(name) = &file.name {
        spec.name = name.clone();
    }

    if !with_graph && file.passes.iter().all(|pass| pass.writes.is_empty()) {
        panic!(
            "--no-frame-graph 这条路对 pass 配方已经不可用了：内容配方不再点名目标\
             （目标由帧图配），而老形状要的正是「内容 pass 自己写 view」。\n  \
             · 要出这一档的图：去掉 --no-frame-graph（默认就是图形模式）；\n  \
             · 要复现六份**冻产物**：走 `--bin scene <档> --no-frame-graph`（那条路不读帧配方）。"
        );
    }
    if !with_graph {
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
    let modules = px_shader::workspace_modules(&px_cook::workspace_root())
        .unwrap_or_else(|err| panic!("读不了 shader 模块表：{err}"));
    for entry in &file.passes {
        let key = px_cook::manifest_key_of("shaders", &entry.shader).unwrap_or_else(|err| {
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
        if entry.kind == "compute" {
            panic!(
                "pass '{label}' 的 kind 是 compute：这一版执行器只有 fullscreen 与 geometry。\
                 声明了执行器不兑现的东西就当场拒 —— 静默跳过正是要避免的那种故障"
            );
        }
        if entry.kind == "fullscreen" {
            let (shader_source, _) = shader_parts_of(&member, &px_cook::cache_root())
                .unwrap_or_else(|err| panic!("pass '{label}'：{err}"));
            let mut seen = Vec::new();
            let assembled = px_shader::assemble::render_source(
                &shader_source,
                &modules,
                px_shader::host_stubs::wgpu_host_stub,
                &mut seen,
            );
            let entries = px_shader::reflect::entry_points(&assembled, &format!("pass '{label}'"))
                .unwrap_or_else(|err| panic!("{err}"));
            if !entries
                .iter()
                .any(|(name, stage)| name == &entry.entry && *stage == "fragment")
            {
                let fragments: Vec<&str> = entries
                    .iter()
                    .filter(|(_, stage)| *stage == "fragment")
                    .map(|(name, _)| name.as_str())
                    .collect();
                panic!(
                    "pass '{label}' 要的片元入口 '{}' 在 shader '{}' 里不存在；它有的片段入口：{}\n\
                     ⇒ 要么入口名拼错了，要么那份 WGSL 里的函数该改名（全屏 pass 那条约定是 fs_main）",
                    entry.entry,
                    entry.shader,
                    if fragments.is_empty() {
                        "（一个都没有）".to_string()
                    } else {
                        fragments.join(" / ")
                    }
                );
            }
        } else {
            let (_, layout) = shader_parts_of(&member, &px_cook::cache_root())
                .unwrap_or_else(|err| panic!("pass '{label}'：{err}"));
            let params = merge_named(
                &format!("pass '{label}'"),
                &entry.params,
                &[],
                &layout,
                BTreeMap::new(),
            )
            .unwrap_or_else(|err| panic!("{err}"));
            passes.push(pass_spec(entry, label, member, params));
            continue;
        }
        let (_, layout) = shader_parts_of(&member, &px_cook::cache_root())
            .unwrap_or_else(|err| panic!("pass '{label}'：{err}"));
        let params = merge_named(
            &format!("pass '{label}'"),
            &entry.params,
            &[],
            &layout,
            BTreeMap::new(),
        )
        .unwrap_or_else(|err| panic!("{err}"));
        passes.push(pass_spec(entry, label, member, params));
    }
    if !with_graph {
        let mut all = spec.passes.clone();
        all.extend(passes);
        spec.passes = all;
    } else {
        let frame = px_scene::frame::load(&frame_name).unwrap_or_else(|err| panic!("{err}"));
        px_scene::frame::verify(&spec, &frame, &frame_name).unwrap_or_else(|err| panic!("{err}"));
        let insert_at = match frame.after.first() {
            Some(entry) => spec
                .passes
                .iter()
                .position(|pass| pass.label_or(0) == entry.label)
                .unwrap_or_else(|| {
                    panic!(
                        "帧图 '{frame_name}' 的 after 段第一条是 '{}'，而这份产物里没有这个标签\
                         （它有的是：{}）—— `verify` 与这里对不上是工具自己的 bug",
                        entry.label,
                        spec.passes
                            .iter()
                            .map(|pass| pass.label_or(0))
                            .collect::<Vec<_>>()
                            .join(" / ")
                    )
                }),
            None => spec.passes.len(),
        };
        let (first, second) = (frame.chain_color[0].clone(), frame.chain_color[1].clone());
        for resource in &file.resources {
            if spec
                .resources
                .iter()
                .any(|existing| existing.name == resource.name)
            {
                panic!(
                    "pass 配方 '{recipe}' 声明的资源 '{}' 与帧图 '{frame_name}' 声明的名字撞了：\
                     两边都声明了同一个名字 ⇒ 说不清那张图是谁的（帧自己的目标由帧图配，\
                     内容只需要声明它**私有**的那张暂存）",
                    resource.name
                );
            }
            println!(
                "内容 pass 的资源 '{}'（配方自己声明的）：{} / {}",
                resource.name, resource.format, resource.size
            );
            spec.resources.push(PassResource {
                name: resource.name.clone(),
                format: resource.format.clone(),
                size: resource.size.clone(),
                layers: 1,
                usage: resource.usage.clone(),
            });
        }
        let own: Vec<&str> = file
            .resources
            .iter()
            .map(|resource| resource.name.as_str())
            .collect();
        for pass in &passes {
            for name in &pass.writes {
                if own.contains(&name.as_str()) || name == px_protocol::scene::VIEW_BUILTIN {
                    continue;
                }
                panic!(
                    "内容 pass '{}' 的 `writes` 是 '{name}'：只认两种取值 —— '{}'（这一帧的画面）\
                     或这份配方**自己声明**的资源（它声明了：[{}]）。别的一律拒：那个名字会烘进\
                     文档，而执行器会照建一张没人读的图",
                    pass.label,
                    px_protocol::scene::VIEW_BUILTIN,
                    if own.is_empty() {
                        "（一个都没有）".to_string()
                    } else {
                        own.join(" / ")
                    }
                );
            }
        }
        for name in &own {
            let used = passes
                .iter()
                .any(|pass| pass.writes.iter().any(|write| write == name));
            if !used {
                panic!(
                    "pass 配方 '{recipe}' 声明了资源 '{name}'，而没有任何一条 pass 写它：\
                     一张没人写的暂存图会照建不误，而「它参没参与」就没人看得见了。\
                     要它参与就把某一笔的 `writes` 改成它；不要它就删掉这节 `[[resources]]`"
                );
            }
        }
        let last = passes.len().saturating_sub(1);
        let mut current = first.clone();
        for (index, pass) in passes.iter_mut().enumerate() {
            let wanted_read = pass.reads.first().cloned();
            let wanted_write = pass.writes.first().cloned();
            let read = match &wanted_read {
                Some(name) if own.contains(&name.as_str()) => {
                    if current != *name {
                        panic!(
                            "内容 pass '{}'（第 {} 笔）读的是它自己声明的 '{}'，而上一笔落在 \
                             '{current}'：这份配方的接线接不上（要么把读名改成上一笔写的那个，\
                             要么把上一笔的写名改成 '{name}'）",
                            pass.label,
                            index + 1,
                            name
                        );
                    }
                    name.clone()
                }
                _ => current.clone(),
            };
            let write = if index == last || wanted_write.is_none() {
                if read == first {
                    second.clone()
                } else {
                    first.clone()
                }
            } else {
                wanted_write.clone().expect("上面判过是 Some")
            };
            pass.reads = vec![read.clone()];
            pass.writes = vec![write.clone()];
            println!(
                "内容 pass '{}'（第 {} 笔）：读 {} 写 {}（帧图 '{frame_name}' 配的）",
                pass.label,
                index + 1,
                read,
                write
            );
            current = write;
        }
        if let Some(entry) = frame.after.iter().find(|entry| entry.kind == "fullscreen") {
            let at = spec
                .passes
                .iter()
                .position(|pass| pass.label_or(0) == entry.label)
                .unwrap_or_else(|| {
                    panic!(
                        "帧图 '{frame_name}' 的全屏 pass '{}' 不在这份产物的标签里：\
                         内容链的出口接不上（接不上就是往一张没人读的图上画）",
                        entry.label
                    )
                });
            let blit = &mut spec.passes[at];
            println!(
                "帧图 '{}' 的 '{}' 读 {}（链尾）",
                frame_name, blit.label, current
            );
            blit.reads = vec![current.clone()];
        } else {
            eprintln!(
                "⚠ 帧图 '{frame_name}' 的 after 段里没有全屏 pass：内容链的出口无人读 —— \
                 链尾是 '{current}'，而**没有任何一条 pass 会把它搬到 `view`**。\
                 这份产物画得出来，但它与「没有内容 pass」是同一张图。"
            );
        }
        spec.passes.splice(insert_at..insert_at, passes);
    }
    spec.check()
        .unwrap_or_else(|err| panic!("这份 pass 表不成立：{err}"));

    let spec_json = serde_json::to_string(&spec).unwrap_or_else(|err| panic!("{err}"));
    let member_keys = spec
        .members()
        .iter()
        .map(|member| member.key.clone())
        .collect::<Vec<_>>();
    let key = px_cook::scene_key(&spec_json, &member_keys);
    let artifact = match out {
        Some(path) => PathBuf::from(path),
        None => px_protocol::scene::cas_path(&px_cook::cache_root(), &px_cook::hex(&key))
            .unwrap_or_else(|err| panic!("{err}")),
    };
    let bytes = px_protocol::scene::write_scene(&artifact, &spec, px_cook::fnv1a(&spec_json))
        .unwrap_or_else(|err| panic!("{err}"));

    println!("{}", spec.audit());
    println!(
        "产物 passdoc -> {}（内容键 {}｜{} 字节）",
        artifact.display(),
        px_cook::hex_short(&key),
        bytes
    );
    println!(
        "渲染：px_render --offline --scene {} --out x.png --width 960 --height 640",
        artifact.display()
    );
}
