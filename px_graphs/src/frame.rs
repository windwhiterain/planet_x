//! 帧图（§128）：**渲染器的形状**，从配方读进来，烘进每一份场景产物。
//!
//! 为什么是一份**共用**的配方（`art/frame/<名>.toml`）而不是写进每个场景：
//! 帧图说的是"这颗渲染器怎么把一帧画出来"，那是**六个场景共同的事实** ——
//! 抄六遍只会让这件共同的事变得看不见，而漂移是必然的。场景配方用
//! `frame = "<名>"` 指过来（不写就是 `default`）。
//!
//! ⚠ `px_pass` 一个 pass 名字都不认识（§124）：这里出现的 `prepass` / `blit` 是
//! **给人的标签**，执行器只按 `kind` 与状态分派。谁要是拿标签做判断，
//! 设计就退回成"预定义了一批 pass"。
//!
//! ## 内容 pass 插在哪（§128 裁决 D）
//!
//! 帧图分两段：`[[before]]` 与 `[[after]]`。场景烘图时写的是 `before ++ after`；
//! `--bin passes` 把内容 pass 插在**两段之间**，并且：
//!
//! - 插进来的 pass **不写目标**：目标由帧图配（乒乓对 `chain_color`），工具按
//!   "上一段的输出 = 这一段的输入"轮流接过去，最后 blit 读链尾那一个。
//!   理由：内容配方描述的是**做什么**，帧图决定它**在缓冲链的哪一格** ——
//!   同一份 `invert` 配方因此能在任何帧图里用。
//! - 内容配方要是自己点名了目标，工具**当场拒**，并且说清是哪张帧图、它声明了
//!   哪些目标、该怎么改：点名一个帧图没声明的目标，就是往一张没人读的图上画。

use std::collections::BTreeMap;
use std::path::PathBuf;

use px_protocol::scene::{AlphaMode, DrawSpec, Member, Object, PassResource, PassSpec, SceneSpec};
use serde::Deserialize;

/// 不写 `frame = ...` 时用哪一张帧图。
pub const DEFAULT_FRAME: &str = "default";

/// 帧图配方住在哪里。
///
/// ⚠ 用 `workspace_root()` 拼**绝对**路径，不靠当前目录：`cargo test` 跑测试时的
/// 工作目录是**包目录**（`px_graphs/`），而 `cargo run` 是调用它的那个目录 ——
/// 相对路径会让"同一个配方在两个入口下指向两个地方"。
pub fn recipe_path(name: &str) -> PathBuf {
    px_ops::workspace_root()
        .join("art")
        .join("frame")
        .join(format!("{name}.toml"))
}

/// `select` 认的取值。**这是烘图侧的词汇**，不是渲染器的：它按物体自己带着的
/// 材质 alpha 档分组，而那正是 oracle 分相位的依据。
const SELECTS: [&str; 4] = ["opaque", "transparent", "skybox", "none"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameFile {
    /// 这一帧的中间目标。pass 按名字引用它们。
    pub resources: Vec<ResourceFile>,
    /// 内容 pass 插在 `before` 之后、`after` 之前。
    #[serde(default)]
    pub before: Vec<EntryFile>,
    #[serde(default)]
    pub after: Vec<EntryFile>,
    /// 内容链的**乒乓对**（裁决 D）：插入的 pass 按它轮流读写，恰好两个。
    #[serde(default)]
    pub chain_color: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceFile {
    pub name: String,
    pub format: String,
    pub size: String,
    #[serde(default)]
    pub usage: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryFile {
    /// 给人看的标签。⚠ 只进文档、给人读；执行器不拿它做判断。
    pub label: String,
    pub kind: String,
    /// 画哪些物体：`opaque` / `transparent` / `skybox` / `none`。
    #[serde(default)]
    pub select: Option<String>,
    /// 顶点阶段（几何 pass 用）：WGSL **文件**路径。空 = 全屏 pass（顶点由执行器自备）。
    #[serde(default)]
    pub vertex_shader: Option<String>,
    #[serde(default)]
    pub vertex_entry: String,
    /// 片元成员（**全屏 pass 用**）：CAS 里的 shader 节点名。
    /// 几何 pass 的片元阶段属于**材质**（每个物体一支，§129），不在这里。
    #[serde(default)]
    pub fragment_shader: Option<String>,
    #[serde(default)]
    pub entry: String,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub depth_target: Option<String>,
    /// 附件与固定功能状态：**`px_pass::RenderState` 的那串文本**，原样透传
    /// （协议与烘图侧都不解析它：解析器只有一份，住在 `px_pass`）。
    pub render: String,
    /// 全屏 pass 的参数：**按名字**给，按它自己声明的结构体打包（与材质同一条路）。
    #[serde(default)]
    pub params: BTreeMap<String, toml::Value>,
}

impl EntryFile {
    fn select(&self) -> &str {
        self.select.as_deref().unwrap_or("none")
    }
}

pub fn load(name: &str) -> Result<FrameFile, String> {
    let path = recipe_path(name);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不了帧图配方 {}：{err}", path.display()))?;
    let frame: FrameFile =
        toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))?;
    frame.check()?;
    Ok(frame)
}

impl FrameFile {
    fn check(&self) -> Result<(), String> {
        if self.resources.is_empty() {
            return Err("帧图一个 resources 都没有：中间目标画到哪去？".to_string());
        }
        if self.chain_color.len() != 2 {
            return Err(format!(
                "帧图的 chain_color 有 {} 个：内容链的乒乓对要**恰好两个**（帧图的绘制段写一个，\
                 链的第一笔写另一个）",
                self.chain_color.len()
            ));
        }
        for name in &self.chain_color {
            if !self.resources.iter().any(|r| &r.name == name) {
                return Err(format!(
                    "chain_color 里的 '{name}' 不在 resources 里（声明了的：{}）",
                    self.resources
                        .iter()
                        .map(|r| r.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ));
            }
        }
        for (index, entry) in self.before.iter().chain(self.after.iter()).enumerate() {
            let at = format!("帧图第 {index} 条 '{}'", entry.label);
            if !SELECTS.contains(&entry.select()) {
                return Err(format!(
                    "{at} 的 select 是 '{}'：认 {}",
                    entry.select(),
                    SELECTS.join(" / ")
                ));
            }
            match (&entry.vertex_shader, &entry.fragment_shader) {
                (Some(_), Some(_)) => {
                    return Err(format!(
                        "{at} 同时给了顶点阶段与片元成员：几何 pass 只给顶点阶段\
                         （片元阶段属于材质），全屏 pass 只给片元成员"
                    ))
                }
                (None, None) if entry.kind == "fullscreen" => {
                    return Err(format!("{at} 是 fullscreen，却没给 fragment_shader"))
                }
                (None, None) => {
                    return Err(format!("{at} 是几何 pass，却没给 vertex_shader（WGSL 文件）"))
                }
                (Some(_), None) if entry.vertex_entry.trim().is_empty() => {
                    return Err(format!("{at} 给了顶点阶段却没给 vertex_entry"))
                }
                _ => {}
            }
        }
        // 全屏那一段（blit）读的必须是乒乓对里的一个：内容链接完，它读链尾。
        if let Some(blit) = self.after.iter().find(|entry| entry.kind == "fullscreen") {
            let read = blit.reads.first().map(String::as_str).unwrap_or("");
            if !self.chain_color.iter().any(|name| name == read) {
                return Err(format!(
                    "帧图最后那条全屏 pass '{}' 读的是 '{read}'，它不在 chain_color（{}）里：\
                     内容链接完要读链尾那一个",
                    blit.label,
                    self.chain_color.join(" / ")
                ));
            }
        }
        Ok(())
    }
}

/// 把一个 `select` 展开成一串 `Draw`。
///
/// ⚠ 名字用的是**物体 id**（几何与材质都是它）：宿主拿这个 id 查它该用哪份顶点数据
/// （网格成员，或者内建图元 + 参数）与哪份材质。名字是内容，`px_pass` 一个都不认识。
pub fn draws_of(objects: &[Object], select: &str) -> Vec<DrawSpec> {
    let draw = |object: &Object| DrawSpec {
        geometry: object.id.clone(),
        material: object.id.clone(),
    };
    match select {
        "opaque" => objects
            .iter()
            .filter(|object| object.material.alpha == AlphaMode::Opaque)
            .map(draw)
            .collect(),
        "transparent" => objects
            .iter()
            .filter(|object| object.material.alpha != AlphaMode::Opaque)
            .map(draw)
            .collect(),
        // 天空盒不是物体：它是环境里那一份成员，几何是**程序化**的（没有顶点缓冲，
        // 顶点由顶点着色器按 `vertex_index` 现算）。这两个名字是帧图与宿主之间的约定。
        "skybox" => vec![DrawSpec {
            geometry: "skybox".to_string(),
            material: "skybox".to_string(),
        }],
        _ => Vec::new(),
    }
}

/// 帧图 → 文档里的 `resources` + `passes`。
///
/// `with_graph = false` 就是**兼容逃生门**（`--no-frame-graph`）：两栏都空，产物因此与
/// 没有帧图时**逐字节相同**（六份冻产物的 sha256 是这条的判据）。
///
/// ⚠ 那道开关**不是**"另一种受支持的烘法"：它存在的唯一目的是证明老产物还能逐字节复现。
/// 它要是开始长自己的功能，就该删掉它、把那六份冻成夹具（§128）。
pub fn build(
    frame: &FrameFile,
    objects: &[Object],
    with_graph: bool,
) -> Result<(Vec<PassResource>, Vec<PassSpec>), String> {
    if !with_graph {
        return Ok((Vec::new(), Vec::new()));
    }
    let resources = frame
        .resources
        .iter()
        .map(|resource| PassResource {
            name: resource.name.clone(),
            format: resource.format.clone(),
            size: resource.size.clone(),
            usage: resource.usage.clone(),
        })
        .collect();

    let mut passes: Vec<PassSpec> = Vec::new();
    for entry in frame.before.iter().chain(frame.after.iter()) {
        let at = format!("帧图 pass '{}'", entry.label);
        // 顶点阶段：文件内容**内联**进文档（自描述：读这份产物不需要再回来看配方）。
        let (vertex_shader, vertex_entry) = match &entry.vertex_shader {
            Some(path) => {
                // 配方里写的是**仓库内**的相对路径；同样不靠当前目录（见 `recipe_path`）。
                let full = px_ops::workspace_root().join(path);
                let text = std::fs::read_to_string(&full)
                    .map_err(|err| format!("{at} 读不了顶点阶段 {}：{err}", full.display()))?;
                (text, entry.vertex_entry.clone())
            }
            None => (String::new(), String::new()),
        };
        let mut params: BTreeMap<String, px_protocol::scene::Value> = BTreeMap::new();
        let shader = match &entry.fragment_shader {
            Some(node) => {
                let key = px_ops::manifest_key_of("shaders", node).map_err(|err| {
                    format!("{at} 的片元 shader '{node}'：{err}（先跑 --bin shaders 烘）")
                })?;
                let member = Member::new("shaders", node, &key);
                // 参数按**这份 shader 自己的契约**打包：烘图时就把三档
                // （名字不认识 / 声明了没人给 / 类型不符）全拦下来，不等装载时才拒。
                let layout = crate::params::schema_of(&member, &px_ops::cache_root())
                    .map_err(|err| format!("{at}：{err}"))?;
                params = crate::params::merge_named(
                    &format!("帧图 pass '{}'", entry.label),
                    &entry.params,
                    &[],
                    &layout,
                    BTreeMap::new(),
                )
                .map_err(|err| err.to_string())?;
                Some(member)
            }
            None => None,
        };
        passes.push(PassSpec {
            kind: entry.kind.clone(),
            shader,
            label: entry.label.clone(),
            entry: entry.entry.clone(),
            reads: entry.reads.clone(),
            writes: entry.writes.clone(),
            params,
            draws: if entry.kind == "geometry" {
                draws_of(objects, entry.select())
            } else {
                Vec::new()
            },
            vertex_shader,
            vertex_entry,
            render: entry.render.clone(),
            depth_target: entry.depth_target.clone(),
        });
    }
    Ok((resources, passes))
}

/// 这张帧图 `before ++ after` 的标签序列。
pub fn frame_labels(frame: &FrameFile) -> Vec<String> {
    frame
        .before
        .iter()
        .chain(frame.after.iter())
        .map(|entry| entry.label.clone())
        .collect()
}

/// 核对：这份产物是不是用这张帧图烘的。不是就**当场拒**，并说清期望什么、实际是什么。
pub fn verify(spec: &SceneSpec, frame: &FrameFile, name: &str) -> Result<(), String> {
    let expected = frame_labels(frame);
    let found: Vec<String> = spec.passes.iter().map(|pass| pass.label.clone()).collect();
    if found == expected {
        return Ok(());
    }
    Err(format!(
        "这份产物不是用帧图 '{name}' 烘的：期望 passes 的标签是 [{}]，实际是 [{}]。\
         要么改用烘它的那张帧图（--frame <名>），要么用 --no-frame-graph 走老形状",
        expected.join(" / "),
        if found.is_empty() {
            "（空）".to_string()
        } else {
            found.join(" / ")
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::scene::{CullMode, Geometry, Material, Transform};

    /// 这些判据要**读真的配方**（`art/frame/default.toml`），而 `px_ops::workspace_root()`
    /// 要上下文先开过 —— 生产里由两个 bin 的 main 开，测试里在这里开一次。
    fn begin() {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            px_ops::begin(px_ops::GraphSpec {
                name: "frame-tests".to_string(),
                version: 1,
                source_hash: 1,
                width: 0,
                height: 0,
                projection: px_ops::field::Projection::Cube,
                cameras: Vec::new(),
            });
        });
    }

    fn object(id: &str, alpha: AlphaMode) -> Object {
        let mut material = Material::new(Member::new("shaders", "surface", &"a".repeat(64)));
        material.alpha = alpha;
        material.cull = CullMode::Back;
        Object {
            id: id.to_string(),
            geometry: Geometry::primitive("icosphere", BTreeMap::new()),
            material,
            transform: Transform::default(),
            cast_shadow: true,
        }
    }

    /// `select` 按**材质自带的 alpha 档**挑物体 —— 那是 oracle 分相位的依据。
    #[test]
    fn a_select_picks_objects_by_their_material_alpha() {
        let objects = vec![
            object("planet", AlphaMode::Opaque),
            object("atmosphere", AlphaMode::Add),
            object("clouds", AlphaMode::Premultiplied),
            object("rings", AlphaMode::Blend),
        ];
        let opaque = draws_of(&objects, "opaque");
        assert_eq!(
            opaque.iter().map(|d| d.geometry.as_str()).collect::<Vec<_>>(),
            vec!["planet"]
        );
        let transparent = draws_of(&objects, "transparent");
        assert_eq!(
            transparent
                .iter()
                .map(|d| d.geometry.as_str())
                .collect::<Vec<_>>(),
            vec!["atmosphere", "clouds", "rings"]
        );
        // 一整笔的几何与材质用**同一个名字**（物体 id）：宿主按它查顶点数据与材质。
        assert_eq!(transparent[0].material, "atmosphere");
        // 天空盒不是物体：程序化的三个顶点。
        let sky = draws_of(&objects, "skybox");
        assert_eq!(sky.len(), 1);
        assert_eq!(sky[0].geometry, "skybox");
        assert_eq!(draws_of(&objects, "none").len(), 0);
    }

    /// 兼容逃生门：不给帧图 ⇒ 两栏都空（产物逐字节回到老形状）。
    #[test]
    fn the_legacy_switch_emits_nothing_at_all() {
        begin();
        let frame = load(DEFAULT_FRAME).expect("默认帧图要能读");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let (resources, passes) = build(&frame, &objects, false).expect("老形状");
        assert!(resources.is_empty(), "老形状不许有 resources");
        assert!(passes.is_empty(), "老形状不许有 passes");
        // 反过来：开着就得真的出东西。
        let (resources, passes) = build(&frame, &objects, true).expect("帧图");
        assert!(!resources.is_empty(), "帧图要声明中间目标");
        assert_eq!(
            passes.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(),
            frame_labels(&frame)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
    }

    /// 帧图的形状判据：乒乓对、select 词汇、顶点/片元各归其位。
    #[test]
    fn a_broken_frame_recipe_is_refused_by_name() {
        begin();
        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        let before = frame.chain_color.clone();
        frame.chain_color = vec![before[0].clone()];
        let err = frame.check().expect_err("乒乓对只有一个 ⇒ 拒");
        assert!(err.contains("恰好两个"), "{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        frame.chain_color = vec!["nope".to_string(), before[1].clone()];
        let err = frame.check().expect_err("乒乓对里有没声明的名字 ⇒ 拒");
        assert!(err.contains("nope"), "{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        frame.before[0].select = Some("clouds".to_string());
        let err = frame.check().expect_err("不认识的 select ⇒ 拒");
        assert!(err.contains("clouds"), "{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        frame.before[1].fragment_shader = Some("px_grade".to_string());
        let err = frame.check().expect_err("几何 pass 给片元成员 ⇒ 拒");
        assert!(err.contains("属于材质"), "{err}");
    }

    /// 核对基准产物：标签对不上就报出**期望什么**与**实际是什么**。
    #[test]
    fn verify_names_the_expected_frame_and_what_it_found() {
        begin();
        let frame = load(DEFAULT_FRAME).expect("默认帧图");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let (resources, passes) = build(&frame, &objects, true).expect("帧图");
        let spec = SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: "夹具".to_string(),
            environment: Default::default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            resources,
            passes,
            lights: Vec::new(),
            objects,
        };
        verify(&spec, &frame, DEFAULT_FRAME).expect("自己烘的自己认");

        let mut stale = spec.clone();
        stale.passes.truncate(2);
        let err = verify(&stale, &frame, DEFAULT_FRAME).expect_err("少了两条 ⇒ 拒");
        assert!(err.contains(DEFAULT_FRAME), "要说清是哪张帧图：{err}");
        assert!(err.contains("prepass"), "要列出期望的标签：{err}");
    }
}
