use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_protocol::scene::{Member, PassResource, PassSpec, SceneSpec};
use px_scene::contract::{merge_named, shader_parts_of};
use serde::Deserialize;

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

/// 一条配方条目 → 文档里的 `PassSpec`（**两条路共用**：fullscreen 那条过了入口守卫的，
/// 与"kind 这一版不兑现、只校验参数"的那条）。
///
/// 打那一行审计 + 构造 `PassSpec` 只有这一份实现：两处各写一遍，漂开的那天就是
/// "有一档烘出来的 pass 少了一栏" —— 而那种缺陷在产物上只表现为一个哈希对不上。
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
        // 帧图那几栏（§125）留空 ⇒ 这一档出的仍是**老形状**的 pass，
        // 与冻在 `target/oracle/pxart-frozen/` 里那六份逐字节同形。
        // 要出几何 pass 时从这里往后加（烘图侧改的那一件事单列，不混在这一步里）。
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
    // ⚠ **第一行**：`--store <目录>` 要在任何 `begin` 之前落成 `PX_ART`
    //   （参数目录不是节点键的一部分，见 `px_graph::driver` 的模块文档）。
    //   下面那个循环不认得 `--store` ⇒ 它会把它当成位置参数，所以这一句必须在它之前跑。
    px_cook::apply_store_args().unwrap_or_else(|err| panic!("{err}"));
    // ⚠ 这张图**一个节点都不走缓存**：`begin` 只要它那一行摘要（图名 / 参数目录 / 缓存条数）。
    let _graph = px_cook::begin(px_cook::GraphSpec {
        name: "passdoc".to_string(),
    });

    // 位置参数：<场景产物> <pass 配方> [输出路径]；开关：--frame <名> / --no-frame-graph。
    let mut positional: Vec<String> = Vec::new();
    let mut frame_name = px_scene::frame::DEFAULT_FRAME.to_string();
    let mut with_graph = true;
    // ⚠ 参数走 `args_without_store()`：这个循环按**位置**读三个参数，而 `--store X`
    //    那一对会顶到位置上 ⇒ 读成"一份叫 `--store` 的场景产物"（不是报错，是读错东西）。
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

    // ⚠ **`--no-frame-graph` 这条路对 pass 配方已经不可用了**，而它现在会以一句离病因很远的话
    //    失败（`SceneSpec::check` 那条"但没有一条写 'view'：画面不会被改动"）—— 因为配方
    //    不再点名目标了，而老形状要的正是"内容 pass 自己写 `view`"。
    //
    //    为什么会这样（实测，`docs/archive/render-wgpu.md` §146.6）：老形状产物**根本没有帧图那几节**
    //    （`scene_depth` / `scene_color_*` / blit 都不在），而 wgpu 宿主必须把 `scene_depth`
    //    seed 成自己建的那张深度图 ⇒ 它在**渲染**那一侧先响，与 `view`/`writes` 无关。
    //    ⇒ 这条逃生门对内容 pass 是死的，所以这里**画一条明确的边界**，而不是让它半路撞上
    //      一句"没写 view"。帧图那六份**冻产物**的复现走的是 `--bin scene --no-frame-graph`
    //      （那条路不读帧配方，也一个字没动，判据照旧）。
    if !with_graph && file.passes.iter().all(|pass| pass.writes.is_empty()) {
        panic!(
            "--no-frame-graph 这条路对 pass 配方已经不可用了：内容配方不再点名目标\
             （目标由帧图配），而老形状要的正是「内容 pass 自己写 view」。\n  \
             · 要出这一档的图：去掉 --no-frame-graph（默认就是图形模式）；\n  \
             · 要复现六份**冻产物**：走 `--bin scene <档> --no-frame-graph`（那条路不读帧配方）。"
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
    // 组装用的模块表：入口那一条守卫要拿**组装后**的 WGSL 去问 naga，而配方里那份文本
    // 还带着 `#{MATERIAL_BIND_GROUP}` 占位符（`px_shader::assemble` 才替它）——
    // 直接拿原文去解析，报的是 `expected expression, found "#"`，离病因很远。
    // 桩表用 **Bevy 那一张**：与 `px_cook::shader_schema`（烘这份产物时用的）同一张，
    // 所以"烘图侧看到的那份文本"与"运行期拿到的那份"是同一份。
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
        // ⚠ **先判能力、再判入口名**，而且入口只对 fullscreen 那一档判。
        //
        // 反过来的话，`art/passes/compute.toml` 会先撞上"片元入口 `cs_main` 不存在"——
        // 而 `cs_main` 是印在第一行的那份 WGSL 里**根本没打算是片元**的名字，真正的理由
        // 是**这一版执行器没有 compute**。两条拒词都拦得住那份文档，但说错理由会把读的人
        // 引向"改个入口名试试" —— 与 §109.3 那条"判据碰不到影子"同一族：
        // **拦住了不等于说对了**。能力那条在 `px_pass::Plan::check` 里（装载期），
        // 这里镜像一份是为了让它在**烘图期**就响。
        if entry.kind == "compute" {
            panic!(
                "pass '{label}' 的 kind 是 compute：这一版执行器只有 fullscreen 与 geometry。\
                 声明了执行器不兑现的东西就当场拒 —— 静默跳过正是要避免的那种故障"
            );
        }
        if entry.kind == "fullscreen" {
            let (shader_source, _) = shader_parts_of(&member, &px_cook::cache_root())
                .unwrap_or_else(|err| panic!("pass '{label}'：{err}"));
            // 组装一遍再问入口：替掉 `#{MATERIAL_BIND_GROUP}`、展开 `#import`（如果这份 shader
            // 有的话）。
            //
            // ⚠⚠ 桩表是 **`wgpu_host_stub`**（不是 `bevy_stub`）：用户裁决「全屏 pass 也要
            //    支持 import」之后，全屏 shader 可以拿宿主桩表里那些符号
            //    （`px_shadow_page_slot` / `PX_PAGE_SIZE` …）。从前传 `bevy_stub` ⇒ 影子桩表
            //    不注入 ⇒ 「unknown identifier」当场拒，而报错里只看得见符号名、看不出
            //    **桩表选错了**（离病因很远）。两份桩表的差别由
            //    `px_render::stubs` 里那条钉住判据管着。
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
            // 别的 kind 这一版执行器都不兑现（`Plan::check` 会按同一条理由拒），
            // 契约那一半仍要读出来（参数还要按它校验）。
            //
            // ⚠ 这一支**不解析 WGSL**（上面那条入口守卫只在 fullscreen 那一档跑）：
            //    一份将来的材质类 shader 带 `#{MATERIAL_BIND_GROUP}`，直接解析会报
            //    "解析不过" —— 而那份文档真正的问题是"kind 这一版不兑现"（装载期拒）。
            //    两条理由都对，但先说能力那条。
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
        // 参数按**这份 shader 自己的契约**透传：烘图时就把三档（名字不认识 / 声明了没人给 /
        // 类型不符）全拦下来，不等装载时才拒 —— 那时候报的是渲染器的错，离改配方已经很远。
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
    // ---- 落位（§128 裁决 D）----
    if !with_graph {
        // 兼容逃生门：老形状就是**追加**在末尾（内容 pass 自己写 `view`）。
        let mut all = spec.passes.clone();
        all.extend(passes);
        spec.passes = all;
    } else {
        let frame = px_scene::frame::load(&frame_name).unwrap_or_else(|err| panic!("{err}"));
        // 先核对基准产物是不是这张帧图烘的：不是就**当场拒**（说清期望什么、实际是什么）。
        px_scene::frame::verify(&spec, &frame, &frame_name).unwrap_or_else(|err| panic!("{err}"));
        // 插入点：`after` 段的第一条在文档里的下标。
        //
        // ⚠ **不能拿 `before.len()` 当下标**（这里原来就是这么写的，而它每一次都越界）：
        //    `px_scene::frame::build` 在**一盏投影的点光都没有**时会把那几条影子 pass
        //    整条丢掉（§109.4），于是 `before.len()` 是**配方**的条数 6，而文档里只有
        //    5 条绘制 pass —— 6 已经不是下标了。上一版还错在第二处：它拿这个数当
        //    `after` 的第一条，而 `after` 里的 pass 在数组里的位置**本来就靠后**。
        //    ⇒ 改成按**标签**找：`verify()` 刚刚证明过文档的标签恰好是 `before ++ after`，
        //      所以"帧图里 `after` 的第一条"在文档里唯一对应一个标签。
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
            // 帧图没有 after 段：内容 pass 追加在末尾。此时`blit_at` 那一段下面会跳过
            // （没有 blit 要改输入），链尾就落在最后一个内容 pass 上。
            None => spec.passes.len(),
        };
        let (first, second) = (frame.chain_color[0].clone(), frame.chain_color[1].clone());
        // 配方自己声明的资源：**先并进文档**，它们才解析得到（执行器按 `resources` 找名字）。
        //
        // ⚠ 与帧图撞名 ⇒ 当场拒（两边都声明了同一个名字 ⇒ 说不清那张图是谁的）。
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
        // 配方声明的资源名（读它 ⇒ 这一笔在"链的中段"，不是在链头上）。
        let own: Vec<&str> = file
            .resources
            .iter()
            .map(|resource| resource.name.as_str())
            .collect();
        // ---- 写名先对账（**在"有没有人用"之前**）----
        //
        // 顺序要紧：一个拼错的写名（`scracth`）会让**真**资源没人写，于是"声明了没人用"
        // 那条会**先**响 —— 而它指的方向是错的（读的人会去删那节 `[[resources]]`，
        // 而真正该改的是那一笔的 `writes`）。**拦住了不等于说对了**（§146.3 同一个形状）。
        //
        // 只认两种取值：`view`（这一帧的画面，会被映成帧链的另一个缓冲），或这份配方
        // **自己声明**的资源。别的一律拒 —— 否则那个名字会烘进文档，而执行器会照建一张
        // 没人读的图（与下面那条 `own` 守卫同族）。
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
        // ⚠ 声明了却没人读 ⇒ 当场拒。不拒的话它会烘进文档（执行器照建一张没人用的图），
        //    而"这张暂存到底参没参与"变成一个**看不见**的事实 —— §133 那条 seed 守卫
        //    （"seed 了没人用"）是同一条理由：一个拼错的名字会悄悄建一张没人读的图。
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
        // ---- 接线：把配方那一份**局部**读/写映到帧图的**真名**上 ----
        //
        // 配方写的是它自己认得的名字（`view` 是"这一帧的画面"）与它自己声明的暂存名；
        // 帧图给的是真名（`scene_color_a` / `scene_color_b`）。三条规则：
        //
        // ① 一笔的**读名就是链头 / 上一笔的落点** —— 落在帧链上是 `first`（绘制段的输出）；
        //    落在配方自己的资源上（前一笔刚写过它）就用那个名字。
        // ② **最后一笔一定写回帧链**，否则画面白改（链尾落在一张没人读的暂存上）。
        // ③ 其余每一笔写它自己点名的那个目标（配方没点名 ⇒ 帧链的下一个，也就是交替）。
        //
        // ⚠ 为什么"最后落回帧链"是**推得出来的**而不是约定：帧图 `after` 段的 blit 只读
        //    `chain_color` 里的名字（`FrameFile::check` 就钉着这一条），所以内容链的出口
        //    只能是那两个之一。链条数在烘图时已知，落点因此也已知（§140：能算出来的别加机制）。
        //
        // ⚠ 每一笔都**打出来**：自动接线可以，"算完不吭声"不行（§73）。
        let last = passes.len().saturating_sub(1);
        let mut current = first.clone();
        for (index, pass) in passes.iter_mut().enumerate() {
            // 配方那一行 `reads`/`writes` 是**接线意图**，落到文档里的是帧的真名。
            let wanted_read = pass.reads.first().cloned();
            let wanted_write = pass.writes.first().cloned();
            let read = match &wanted_read {
                // 读的是它自己声明的资源 ⇒ 前一笔的落点就是它。
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
                // 读的是画面（`view`）或没写 ⇒ 链头 / 上一笔的落点。
                _ => current.clone(),
            };
            let write = if index == last || wanted_write.is_none() {
                // 最后一笔（或配方没点名）：落回帧链 —— 两个缓冲里**不是** `read` 的那个。
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
        // blit 读链尾那一个（链空时就是帧图绘制段的输出）。
        //
        // ⚠ 按**标签**找那条全屏 pass（帧图 `after` 段里 kind = fullscreen 的那一条），
        //    不用下标：文档里那段 pass 的**个数**由内容定（影子那几条会整条不烘），
        //    下标是"配方 + 内容"的函数，而标签是配方的原话。
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
    // ⚠ 括号里那一格是**内容键**，不是文件字节的 sha256 —— 与 `--bin scene` 同一条提醒：
    //    `docs/anchors.md` 里"逃生门"那六格判的是**文件字节**，拿键去比会全报 ✗。
    println!(
        "产物 passdoc -> {}（内容键 {}｜{} 字节）",
        artifact.display(),
        px_cook::hex_short(&key),
        bytes
    );
    // ⚠ S8-a：`px_render`（bevy 宿主）已删，这条提示改指**新宿主**，
    //   而且写全**离线出图**那条路（新宿主的 `--scene --out` 缺省是"交给在跑的服务"，
    //   离线必须显式写 `--offline`，见 §147.5 —— 少了它这条提示就又成了半句命令）。
    //   ⚠ §157（2026-09-19）：新宿主改名叫 `px_render` ⇒ 这条提示**一个字不用改**
    //   （它当时写的就是这个名字）。上面那句里的 `px_render`（"bevy 宿主已删"）按旧义读。
    println!(
        "渲染：px_render --offline --scene {} --out x.png --width 960 --height 640",
        artifact.display()
    );
}
