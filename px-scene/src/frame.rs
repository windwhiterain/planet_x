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

use px_protocol::material::ParamKind;
use px_protocol::scene::{
    AlphaMode, DrawSpec, FrameMaterial, MaterialInstance, Member, Object, PassCubeFace,
    PassResource, PassSpec, SceneSpec, Value,
};
use serde::Deserialize;

/// 不写 `frame = ...` 时用哪一张帧图。
pub const DEFAULT_FRAME: &str = "default";

/// 帧图配方住在哪里。
///
/// ⚠ 用 `workspace_root()` 拼**绝对**路径，不靠当前目录：`cargo test` 跑测试时的
/// 工作目录是**包目录**（`px_graphs/`），而 `cargo run` 是调用它的那个目录 ——
/// 相对路径会让"同一个配方在两个入口下指向两个地方"。
pub fn recipe_path(name: &str) -> PathBuf {
    px_graph::workspace_root()
        .join("art")
        .join("frame")
        .join(format!("{name}.toml"))
}

/// `select` 认的取值。**这是烘图侧的词汇**，不是渲染器的：它按物体自己带着的
/// 材质 alpha 档（或者"投不投影"那一格）分组，而那正是 oracle 分相位的依据。
const SELECTS: [&str; 5] = ["opaque", "transparent", "shadow_casters", "skybox", "none"];

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
    /// **帧自有**的材质（§135）：名字 + WGSL **文件** + 入口 + 参数来源。
    ///
    /// ⚠ 它烘进文档时是**内联全文**（见 [`MaterialFile`]）：改 `art/frame/**` 下的 WGSL
    /// 必须重烘。
    #[serde(default)]
    pub materials: Vec<MaterialFile>,
}

/// 帧配方里的 `[[materials]]`：一份**帧自有材质**（§135）—— 天空盒那种"属于这颗渲染器、
/// 不是可换内容"的材质。
///
/// ⚠ `params` 写的是**来源**，不是值：`brightness = "environment.skybox_brightness"`。
/// 帧图是六个场景**共用**的一份，往这里写死一个数就是 §133 那颗雷
/// （"六份场景今天恰好都是 900.0" —— 第一份设了别的亮度的场景会得到整幅背景错的图，
/// 而且不会有人报错）。烘图时按 `environment` 现取，文档里落的是**值**。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialFile {
    /// 名字：一条 draw 的 `material` 按它解析（今天只有 `skybox`）。
    pub name: String,
    /// WGSL **文件**路径（仓库内相对路径，与 `vertex_shader` 同规矩）：全文内联进产物。
    pub shader: String,
    pub entry: String,
    /// 参数名 → **来源名**（认得的见 [`SOURCES`]）。
    #[serde(default)]
    pub params: BTreeMap<String, toml::Value>,
}

/// 帧材质参数能从**文档**里取到的那几个内容值（今天的词汇表只有 `environment.*`）。
///
/// 词汇表故意窄：每加一个来源，都要在这里写清"它是什么、哪一档类型" ——
/// 而"哪一档"正是烘图时能拒掉类型不符的依据（`brightness` 是 `f32`，
/// 拿一个贴图成员塞进去必须是当场拒，不是打包时按 `f32` 硬写四个字节）。
#[derive(Debug, Clone, Copy)]
pub struct Sources {
    pub ambient: f32,
    pub skybox_brightness: f32,
    /// **投影的点光有几盏**（`lights[].shadows == true` 的点光）。
    ///
    /// 帧图里那一份 atlas 的层数与"展开几条影子 pass"都由它算出来：
    /// 每盏投影的点光一个 cube、每面一层，而**一盏都没有时整份资源与那些 pass 都不烘**
    /// （oracle 那边也不渲染影子图 —— 它照样分配那张纹理，而"分配"不出图，
    /// 不值得我们往文档里放一份没人写的图）。
    ///
    /// ⚠ 它是**内容**（灯表里的一格），所以走 `Sources` 这条通道进烘图，
    /// 与 `ambient` / `skybox_brightness` 同一条口径（§133：配方里不写内容值）。
    pub shadow_lights: usize,
}

/// 认得的来源名。⚠ 报错时**必须把它列出来** —— 不列的话作者只能靠猜，
/// 而"猜一个来源名"正是这套东西想避免的那种试错。
pub const SOURCES: [&str; 2] = ["environment.ambient", "environment.skybox_brightness"];

/// cube 的六面。⚠ 与 `bevy_camera-0.19.1/src/primitives.rs:347-390` 的 `CUBE_MAP_FACES`
/// 同序（`+X −X +Y −Y +Z −Z`），面名与同文件 `face_index_to_name` 一致。
///
/// 这一份是**文档侧**的：烘图时用它算层号、拼标签。宿主侧另有一份"面 → 朝向/上方向"
/// （它要算矩阵），两边的**次序**必须一致 —— 一致性由 `layer = light×6+face` 那条对账
/// 钉住（宿主拿到的 face 与 layer 一起进来，对不上就拒）。
pub const CUBE_FACES: u32 = 6;
pub const FACE_NAMES: [&str; 6] = ["+x", "-x", "+y", "-y", "+z", "-z"];

/// 来源名 →（值，那一档类型）。`None` = 不认识这个名字。
fn source_of(name: &str, sources: &Sources) -> Option<(Value, ParamKind)> {
    match name {
        "environment.ambient" => Some((Value::Num(f64::from(sources.ambient)), ParamKind::F32)),
        "environment.skybox_brightness" => Some((
            Value::Num(f64::from(sources.skybox_brightness)),
            ParamKind::F32,
        )),
        _ => None,
    }
}

/// `px_protocol::scene::Value` → `toml::Value`。
///
/// 为什么绕一圈：参数的校验与打包只有一份实现（`px_scene::contract::merge_named` +
/// `MaterialLayout::pack`，材质与全屏 pass 都走它），而它的入口是 toml 值
/// （配方那一侧本来就是 toml）。在烘图侧再写一份"按名字打包"就是第二个真相。
fn toml_of(value: &Value) -> toml::Value {
    match value {
        Value::Num(number) => toml::Value::Float(*number),
        Value::Text(text) => toml::Value::String(text.clone()),
        Value::Triple(items) => toml::Value::Array(
            items
                .iter()
                .map(|v| toml::Value::Float(f64::from(*v)))
                .collect(),
        ),
        Value::Quad(items) => toml::Value::Array(
            items
                .iter()
                .map(|v| toml::Value::Float(f64::from(*v)))
                .collect(),
        ),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceFile {
    pub name: String,
    pub format: String,
    pub size: String,
    /// 层数：**来源**（不是数）。今天只认一个来源 `"shadow_faces"` ——
    /// "每盏投影的点光一个 cube，每面一层"，由 [`Sources::shadow_lights`] 解出来
    /// （§133 同一条口径：配方里不写内容值，文档里落**解出来的整数**）。
    #[serde(default)]
    pub layers: Option<String>,
    #[serde(default)]
    pub usage: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryFile {
    /// 给人看的标签。⚠ 只进文档、给人读；执行器不拿它做判断。
    pub label: String,
    pub kind: String,
    /// 画哪些物体：`opaque` / `transparent` / `shadow_casters` / `skybox` / `none`。
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
    /// **这一条展开成几条**：`None` = 一条（老形状）；`Some(6)` = **点光 cube 的六面**，
    /// 每面一条单层 pass（§109.1：Bevy 是 6 个单层 pass，`multiview_mask: None`）。
    ///
    /// ⚠ 为什么把"6"写在配方里而不是烘图侧写死：面数是 **cube 的定义**（六面），
    /// 而这里正是"渲染器的形状"住的地方；写死在烘图侧等于让配方说一套、代码做一套。
    /// ⚠ 展开是**按投影的灯 × 6 面**：一盏都不投时**一条都不展开**（oracle 那边也不渲染
    /// 影子图）；而那六面各自的 `cube_face = { light, face, layer }` 由烘图侧算好写进文档 ——
    /// 算式（`layer = light × 6 + face`）只有烘图侧知道，宿主拿到的是**数**并当场对账。
    ///
    /// ⚠ 虚拟影图（§本轮）之后它展开的还是**六面**："每一面要画哪几页"是宿主侧的事
    /// （页数由 `shadow_density` 与物体的包围球算出来，见 `px_render::vshadow`）——
    /// 一页一笔 draw 落在**同一面这一条 pass** 上，靠 `MaterialInstance` + 动态偏移区分。
    /// 所以这一栏一个字都不用改：**页不是 pass 的展开维，是 draw 的展开维**。
    #[serde(default, alias = "cube_faces")]
    pub shadow_faces: Option<u32>,
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
                // ⚠ copy **先判**（§131）：它不画任何东西 —— 顶点阶段一个都不跑、片元阶段
                //    也没有（拷贝是 `copy_texture_to_texture`，不建管线）。
                //    所以这两栏**都必须是空的**，而"两栏都空"对别的 kind 又是错的。
                (None, None) if entry.kind == "copy" => {}
                (_, _) if entry.kind == "copy" => {
                    return Err(format!(
                        "{at} 的 kind 是 copy：拷贝不画东西，`vertex_shader` / `vertex_entry` / \
                         `fragment_shader` / `entry` 四栏都该是空的\
                         （顶点阶段是几何 pass 那一栏，片元成员是全屏 pass 那一栏）"
                    ));
                }
                (Some(_), Some(_)) => {
                    return Err(format!(
                        "{at} 同时给了顶点阶段与片元成员：几何 pass 只给顶点阶段\
                         （片元阶段属于材质），全屏 pass 只给片元成员"
                    ));
                }
                (None, None) if entry.kind == "fullscreen" => {
                    return Err(format!("{at} 是 fullscreen，却没给 fragment_shader"));
                }
                (None, None) => {
                    return Err(format!(
                        "{at} 是几何 pass，却没给 vertex_shader（WGSL 文件）"
                    ));
                }
                (Some(_), None) if entry.vertex_entry.trim().is_empty() => {
                    return Err(format!("{at} 给了顶点阶段却没给 vertex_entry"));
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
        // ---- 帧自有材质（§135）：**配方形状**那一半 ----------------------------
        //
        // 参数里的**来源对不对**（认不认识、类型对不对）要反射 WGSL 才知道，那一步在
        // `bake_material` 里；这里只判"名字/入口/文件这些一眼能看出来的"。
        let mut names: Vec<&str> = Vec::new();
        for material in &self.materials {
            let at = format!("帧图材质 '{}'", material.name);
            if material.name.trim().is_empty() {
                return Err(
                    "帧图有一份 `[[materials]]` 没给 name：draws 是按名字引用它的".to_string(),
                );
            }
            if names.contains(&material.name.as_str()) {
                return Err(format!(
                    "帧图里材质名重了：'{}'（两份材质同名 ⇒ 宿主只能猜一个）",
                    material.name
                ));
            }
            if material.shader.trim().is_empty() {
                return Err(format!("{at} 没给 shader（WGSL 文件路径）"));
            }
            if material.entry.trim().is_empty() {
                return Err(format!("{at} 没给 entry（@fragment 那个函数叫什么）"));
            }
            let full = px_graph::workspace_root().join(&material.shader);
            if !full.is_file() {
                return Err(format!(
                    "{at} 的 shader '{}' 不是一份文件（{}）：它会被**内联**进产物，\
                     所以必须读得到",
                    material.shader,
                    full.display()
                ));
            }
            names.push(&material.name);
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
        // ⚠ 透明那一档的**次序是算出来的**，不是 `objects[]` 的顺序 —— 这是实测出来的，
        // 不是顺手写的。规则两条，缺一条就错：
        //
        //   ① **主键 = 材质的 `depth_bias`（升序）** —— 更小的先画；
        //   ② **平局时用 `objects[]` 的反序**。
        //
        // 也就是"先把 `objects[]` 反过来，再对它做一次**稳定**排序"。`sort_by` 是稳定排序，
        // 所以 `depth_bias` 相等的那些会保持反序 —— 两条规则一次落地。
        //
        // **oracle 那边为什么是这样**：透明物体进的是 `Transparent3d`
        // （`bevy_core_pipeline-0.19.1/src/core_3d/mod.rs:426-449`），排序键是
        // `ViewRangefinder3d::distance(world_from_local * mesh.aabb_center) + depth_bias`、
        // **升序**（远的先画），排序是稳定的（`IndexMap::sort_by_key`）。本仓今天的内容里，
        // 距离那一项要么**精确平局**（环与大气）、要么**差三个数量级翻不过来**（云，见下），
        // 所以次序实际由 `depth_bias` 与平局规则决定。
        //
        // **实测**（仪器 `target/rings/anchor-*.ps1`：拿 `target/debug/px_render.exe` 这个**锚宿主**
        // 喂**改过的冻结 legacy 文档**，比对哈希）：
        //
        // ⚠ S8-c 标注：那台仪器与那支锚宿主**都已经不在了**（§154；`art/anchor/hashes.txt` §五）
        // ⇒ 这张表**取不回来了**，它是**记录**。而它定出的次序规则仍然有效，并且由六档逐字节回归
        // （J1 六张与 `art/anchor/*.png`）替它背书 —— 记录留下，提问的能力没了。
        // ⚠ §157（2026-09-19）就地更正：上面最后那半句**在写下时不成立** —— 当时还有一支
        // **够格**的 Bevy oracle 活在一支未登记的 exe 里（六张判据图逐字节全中）；**裁决是不留**，
        // 而改名覆盖了那个路径 ⇒ **从 §157 起它成立**。见 `art/anchor/README.md` 与 §157。
        //
        // | 扰动 | oracle 的结果 | 说明 |
        // |---|---|---|
        // | `orbit-rings` 基线（objects[] = planet, atmosphere, rings，两笔都 bias 0） | 画的是 [rings, atmosphere] | 平局 ⇒ **反序** |
        // | 把 objects[] 前两个对调 | 跟着对调 | 次序**依赖 `objects[]`** ⇒ 距离项是平局 |
        // | 复制一份 rings 成 ringsB（3 笔透明） | 六种排列里**只有一个**中：[ringsB, rings, atmosphere] | 反序 |
        // | objects[] 换成 [planet, ringsB, atmosphere, rings] | 预测 [rings, atmosphere, ringsB]，**命中** | 反序 |
        // | 再加一份 atmosphere2（4 笔透明） | 预测 [ringsB, rings, atmosphere2, atmosphere]，**命中** | 反序 |
        // | `orbit-soft` 的 `clouds.depth_bias` −1 → **+1**（objects[] 不动） | **哈希变了** | bias **参与** |
        // | `orbit-soft` 的 objects[] 换成 [planet, **clouds, atmosphere**]（bias 不动） | **哈希不变** | bias **压过** `objects[]` 次序 |
        //
        // ⚠ "按距离排序"这条假设被第 2 行直接否证：若真是距离说了算，对调 `objects[]`
        // 不会改变画面 —— 而它改变了。
        //
        // ⚠ **没实现的那一项，以及为什么**：上面那个和里还有 `distance(mesh.aabb_center)`。
        // 它**依赖相机**，而帧图是**每份文档烘一次**、相机是请求时才选的（`--cam` / `--sheet`
        // 的 12 台）⇒ 一份烘好的次序**表达不了**相机相关的量。本仓今天的六个场景里它也
        // **从不决定次序**，两个数都量过：
        //   · 环网格的 aabb_center = **(0, 0, 0) 精确**（`ring_mesh` 顶点 y 恒为 0、x/z 对称）；
        //     大气是 icosphere（中心对称）⇒ 两者**精确平局**（这正是 `orbit-rings` 走规则 ② 的原因）；
        //   · 云的 proxy 网格 aabb_center = `(-0.000598, 0, -0.001809)`（**不**在原点 ——
        //     它是阈值化生成的闭合壳，本来就左右不对称），整整 0.0019；而它跟大气的
        //     `depth_bias` 差 **1.0**，差三个数量级 ⇒ 距离项翻不过来。
        // ⇒ 距离项要真的参与，得先有"哪台相机"这个信息；那是**设计岔路**，不在这一档里挑。
        //
        // ⚠ 这个次序**不是**"政策"（渲染器不需要认识它）：文档里的 `draws` 就是一条有序的
        // 指令流，宿主照单画；次序只在**烘图侧**把 `objects[]` 转录成 draw 列表这一步产生。
        "transparent" => {
            let mut picked: Vec<&Object> = objects
                .iter()
                .rev()
                .filter(|object| object.material.alpha != AlphaMode::Opaque)
                .collect();
            // 稳定排序 ⇒ `depth_bias` 相等的保持上面那个反序（规则 ②）。
            picked.sort_by(|a, b| a.material.depth_bias.total_cmp(&b.material.depth_bias));
            picked.into_iter().map(draw).collect()
        }
        // **投影的那些物体**（§109：`cast_shadow: true`）。
        // ⚠ 判据是物体自己那一格，**不是**它的透明度：大气（Add）与云（Premultiplied）
        //    都是透明的，而它们**不投影**（Bevy 的 `NotShadowCaster`）—— 按透明度挑
        //    会把一整颗云壳塞进 shadow map，画面上是一颗球形硬影。
        "shadow_casters" => objects
            .iter()
            .filter(|object| object.cast_shadow)
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

/// 帧图烘出来的三节：中间目标、pass 表、**帧自有材质**。
///
/// 打成一个结构体而不是元组：§135 之后是三节了，而"三节的次序"是一个没人会去读的约定。
pub struct Baked {
    pub resources: Vec<PassResource>,
    pub passes: Vec<PassSpec>,
    pub materials: Vec<FrameMaterial>,
    /// **生成出来的材质实例**（§139）：点光 cube 影子六面各要**各自的名字**
    /// （一个名字恰好一套组，执行器那边没有覆盖、没有优先级）。
    pub material_instances: Vec<MaterialInstance>,
}

/// 帧图 → 文档里的 `resources` + `passes` + `frame_materials`。
///
/// `sources` 是**内容**那一边的值（今天就是环境里那两个数）：帧材质的参数写的是**来源**，
/// 这里才换成值 —— 帧配方里一个内容值都不许写死（§133）。
///
/// `with_graph = false` 就是**兼容逃生门**（`--no-frame-graph`）：三节都空，产物因此与
/// 没有帧图时**逐字节相同**（六份冻产物的 sha256 是这条的判据）。
///
/// ⚠ 那道开关**不是**"另一种受支持的烘法"：它存在的唯一目的是证明老产物还能逐字节复现。
/// 它要是开始长自己的功能，就该删掉它、把那六份冻成夹具（§128）。
pub fn build(
    frame: &FrameFile,
    objects: &[Object],
    sources: &Sources,
    // 这一帧的灯表（文档那一边的次序）。**只为了拿灯的位置** —— 虚拟影图的分页
    // 要知道"灯在哪"（物体相对灯的位置决定它落在哪一面、哪几页）。
    lights: &[px_protocol::scene::Light],
    with_graph: bool,
) -> Result<Baked, String> {
    if !with_graph {
        return Ok(Baked {
            resources: Vec::new(),
            passes: Vec::new(),
            materials: Vec::new(),
            material_instances: Vec::new(),
        });
    }
    // ---- 虚拟影图的分配：**在这里算**（页是 `.pxart` 的一部分，见模块那一节）----
    //
    // ⚠ 分配只依赖"物体 / 灯 / 密度"，而那些在这一步全都有 ⇒ 它是**烘图**的活。
    //    宿主拿到的是"画哪几页、每页哪一块"，一个决定都不做。
    let allocation = shadow_allocation(lights, objects)?;
    // ---- 中间目标：`layers` 那一栏写的是**来源**，在这里解成整数 ----
    //
    // ⚠ "一盏投影的灯都没有" ⇒ 整份 cube 影图**不烘**（连它带那些 pass 一起不烘）：
    //    oracle 那边照样会分配那张纹理（`max(1,count)*6` 层），但"分配"不出图 ——
    //    往里放一份没有任何 pass 写的资源，只会让宿主与文档对"存在什么"多一处分歧。
    let mut resources: Vec<PassResource> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for resource in &frame.resources {
        let Some(source) = &resource.layers else {
            resources.push(PassResource {
                name: resource.name.clone(),
                format: resource.format.clone(),
                size: resource.size.clone(),
                layers: 1,
                usage: resource.usage.clone(),
            });
            continue;
        };
        let layers = match source.as_str() {
            // 每盏投影的点光一个 cube、**每面一层**（层号 = 灯 × 6 + 面）。
            "shadow_faces" => sources.shadow_lights * CUBE_FACES as usize,
            other => {
                return Err(format!(
                    "帧图资源 '{}' 的 layers 来源是 '{other}'：这一版只认 'shadow_faces'\
                     （每盏投影的点光一个 cube，每面一层）",
                    resource.name
                ));
            }
        };
        if layers == 0 {
            dropped.push(resource.name.clone());
            continue;
        }
        // ⚠ **尺寸由分配算出来**，不抄配方里那个数：配方里写的 `4096x4096` 是**上限**
        //    （"这一版最多用多大"），而实际要多大只有 `shadow_allocation` 知道
        //    （页数按物体的包围球与密度算）。文档里必须落**真的**那一份 ——
        //    执行器会拿它去建池子里的纹理，而对不上就是"绑定与附件不是同一张图"。
        let mut size = resource.size.clone();
        if source == "shadow_faces" {
            let allocation = allocation.as_ref().ok_or_else(|| {
                format!(
                    "帧图资源 '{}' 要按分配定尺寸，而这一帧一盏投影灯都没有",
                    resource.name
                )
            })?;
            size = format!("{}x{}", allocation.atlas.0, allocation.atlas.1);
            let declared = parse_fixed_size(&resource.size)?;
            if allocation.atlas.0 > declared.0 || allocation.atlas.1 > declared.1 {
                return Err(format!(
                    "虚拟影图的 atlas 要 {}×{}，而帧图资源 '{}' 声明的是 {}：\
                     把那一栏调大，或者把这一帧的 shadow_density 调小。\
                     这一版**不降精度** —— 降了之后画面照样出得来，只是影比要求糊",
                    allocation.atlas.0, allocation.atlas.1, resource.name, resource.size
                ));
            }
        }
        resources.push(PassResource {
            name: resource.name.clone(),
            format: resource.format.clone(),
            size,
            layers: layers as u32,
            usage: resource.usage.clone(),
        });
    }
    if !dropped.is_empty() {
        println!(
            "⚠ 这一帧没有投影的点光（{} 盏）⇒ 不烘这些资源：{} —— 也不烘写它们的那几条 pass\
             （oracle 那边同样不渲染影子图）",
            sources.shadow_lights,
            dropped.join(" / ")
        );
    }

    let mut materials = Vec::with_capacity(frame.materials.len());
    if !frame.materials.is_empty() {
        let modules = px_shader::workspace_modules(&px_graph::workspace_root())?;
        for material in &frame.materials {
            materials.push(bake_material(material, sources, &modules)?);
        }
    }

    let mut passes: Vec<PassSpec> = Vec::new();
    // ⚠ §本轮起**空的**：从前影子靠"每面一材质实例"把那一面的 `PassView` 塞给 pass，
    //    现在页的视图走 `PassSpec.params`（几何 pass 的参数块）⇒ 实例那一维没了。
    //    这一栏留着（协议里还有它）是因为"一个名字恰好一套组"那条契约还在 ——
    //    它将来服务的是**运行时参数按需迁移**那一类东西，不是影子。
    let material_instances: Vec<MaterialInstance> = Vec::new();
    for entry in frame.before.iter().chain(frame.after.iter()) {
        let at = format!("帧图 pass '{}'", entry.label);
        // ⚠ 写的是**没烘出来的那份资源**（一盏投影的灯都没有）⇒ 这条 pass 也不烘：
        //    否则文档里会出现一条"深度附件指向一个不存在的名字"的 pass，
        //    而那份文档在装载时会被拒 —— 一份自相矛盾的产物比少一条 pass 糟得多。
        if let Some(target) = &entry.depth_target {
            if dropped.contains(target) {
                println!(
                    "⚠ 帧图 pass '{}' 写的是没烘出来的 '{}' ⇒ 这一条也不烘",
                    entry.label, target
                );
                continue;
            }
        }
        // 顶点阶段：文件内容**内联**进文档（自描述：读这份产物不需要再回来看配方）。
        let (vertex_shader, vertex_entry) = match &entry.vertex_shader {
            Some(path) => {
                // 配方里写的是**仓库内**的相对路径；同样不靠当前目录（见 `recipe_path`）。
                let full = px_graph::workspace_root().join(path);
                let text = std::fs::read_to_string(&full)
                    .map_err(|err| format!("{at} 读不了顶点阶段 {}：{err}", full.display()))?;
                (text, entry.vertex_entry.clone())
            }
            None => (String::new(), String::new()),
        };
        let mut params: BTreeMap<String, px_protocol::scene::Value> = BTreeMap::new();
        let shader = match &entry.fragment_shader {
            Some(node) => {
                let key = px_graph::manifest_key_of("shaders", node).map_err(|err| {
                    format!("{at} 的片元 shader '{node}'：{err}（先跑 --bin shaders 烘）")
                })?;
                let member = Member::new("shaders", node, &key);
                // 参数按**这份 shader 自己的契约**打包：烘图时就把三档
                // （名字不认识 / 声明了没人给 / 类型不符）全拦下来，不等装载时才拒。
                let layout = crate::contract::schema_of(&member, &px_graph::cache_root())
                    .map_err(|err| format!("{at}：{err}"))?;
                params = crate::contract::merge_named(
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
        let draws = if entry.kind == "geometry" {
            draws_of(objects, entry.select())
        } else {
            Vec::new()
        };
        // ---- 展开：`shadow_faces = 6` 的那一条 → **每盏投影的灯 × 六面 × 每一页** ----
        //
        // ⚠ 只有点光的影子走这条路（§109：`light_id` 是 cube 的下标，层号 = light*6+face）。
        //    第 m 盏投影点光的 cube 下标恒为 m —— 因为宿主那一步排序（开影子的在前、
        //    同档稳定）把投影的那些灯**原样**排在最前面，不依赖那个不可实测的 entity 次序。
        //
        // ⚠ **页是烘图侧算的**（`shadow_allocation`）：`.pxart` 本来就该是"一份展开好的
        //    指令流"，而"画哪几页"只依赖物体 / 灯 / 密度 —— 那些在这里全都有。
        //    宿主因此一个分配决定都不做（它只建纹理、发 draw）。
        let Some(faces) = entry.shadow_faces else {
            passes.push(PassSpec {
                kind: entry.kind.clone(),
                shader,
                label: entry.label.clone(),
                entry: entry.entry.clone(),
                reads: entry.reads.clone(),
                writes: entry.writes.clone(),
                params,
                draws,
                vertex_shader,
                vertex_entry,
                render: entry.render.clone(),
                depth_target: entry.depth_target.clone(),
                cube_face: None,
                viewport: None,
            });
            continue;
        };
        if faces != CUBE_FACES {
            return Err(format!(
                "{at} 的 shadow_faces 是 {faces}：cube 只有 {CUBE_FACES} 面\
                 （§109.1：Bevy 就是 6 个单层 pass）"
            ));
        }
        let allocation = allocation.as_ref().ok_or_else(|| {
            format!(
                "{at} 要展开影子 pass，而这一帧没有分配出任何页 —— \
                 帧图里不该出现这一条（一盏投影的灯都没有时它整条都不烘）"
            )
        })?;
        for light in 0..sources.shadow_lights as u32 {
            // 每一面的名字：投影的每一笔 draw（哪个物体投影，由 `select` 挑）在这里
            // **各起一个名字**（`<物体 id>@shadow_<灯>_<面名>`）。
            //
            // ⚠ 名字是**生成**的：人写的那一层（帧配方）仍然只有一份意图 ——
            //    "这一条 pass 画投影的那些物体" + "影子要六面"。手写层长出 N 份材质
            //    才是这个形状要避免的事；生成层里"每个 (pass, 灯, 面) 一个名字"
            //    正是 `.pxart` 作为一种指令流该有的样子（§139 的用户裁决）。
            //
            // ⚠ **名字按 (物体, 灯, 面) 一份，不按页**：页是会变的（灯一动、物体一动，
            //    页数就变），而"一个名字恰好一套组"这条契约不该跟着页数漂。页与页的差别
            //    只有一块 viewport 与一个缩放后的投影 —— 前者是 pass 的一栏，后者由
            //    宿主按那一块窗口现算。
            for face in 0..CUBE_FACES {
                let face_label = format!("{}_{}_{}", entry.label, light, FACE_NAMES[face as usize]);
                let layer = light * CUBE_FACES + face;
                // ---- 这一面的每一页：先清一格，再画**落在这一页上的那几个物体** ----
                //
                // ⚠ 一页一笔 draw（而不是一页里把该画的物体都画一遍）的理由：一笔 draw
                //    只挂一份材质（一套组 1），而"每页只有少数几个物体落进去"是常态 ——
                //    按 caster 拆开之后，每一笔的实例下标是唯一的一个，
                //    而"这一页的 view"由这一笔自己的 `viewport` 说。
                for patch in allocation
                    .patches
                    .iter()
                    .filter(|patch| patch.light == light && patch.face == face)
                {
                    let window = patch.window;
                    let clear = PassSpec {
                        kind: entry.kind.clone(),
                        shader: shader.clone(),
                        label: page_label(&face_label, patch.page_y, patch.page_x),
                        entry: entry.entry.clone(),
                        reads: entry.reads.clone(),
                        writes: entry.writes.clone(),
                        params: params.clone(),
                        draws: Vec::new(),
                        vertex_shader: vertex_shader.clone(),
                        vertex_entry: vertex_entry.clone(),
                        render: entry.render.clone(),
                        depth_target: entry.depth_target.clone(),
                        cube_face: Some(PassCubeFace { light, face, layer }),
                        viewport: None,
                    };
                    passes.push(page_clear_pass(&clear, window, clear.label.clone()));
                    // 这一页的几何：只画 `patch.casters` 里点名的那些物体。
                    //
                    // ⚠ **第一笔顺手清这一格**（`depth=clear(0)`），后面几笔 `load` ——
                    //    atlas 是共享的，而每一页占的是**自己那一格**（互不重叠），
                    //    所以"load"读到的一定是本页第一笔写下的东西。省掉一条专为清屏
                    //    而存在的 pass（一页一条），而"清哪一格"这件事一次都没少。
                    for (index, caster) in patch.casters.iter().enumerate() {
                        // 这一页的那一笔：材质名照**这一面**那一份实例（它带着"哪一面"），
                        // 几何是那个物体自己（`draws_of` 里 geometry == material == 物体 id）。
                        let Some(draw) = draws.iter().find(|draw| &draw.geometry == caster) else {
                            return Err(format!(
                                "{at} 的第 {light} 盏灯第 {} 面页 ({}, {}) 要画 '{}'，\
                                 而这一条的 `select` 里没有它（内部不一致）",
                                FACE_NAMES[face as usize], patch.page_x, patch.page_y, caster
                            ));
                        };
                        let mut page = clear.clone();
                        page.label = format!(
                            "{}_c{}",
                            page_label(&face_label, patch.page_y, patch.page_x),
                            passes.len()
                        );
                        page.draws = vec![DrawSpec {
                            geometry: draw.geometry.clone(),
                            // ⚠ 不加 `@shadow_<灯>_<面>` 那个实例后缀：**这一页的视图不再
                            //    从材质那条路走**（§本轮）。页与页的差别是"这一条 pass 落在
                            //    atlas 的哪一格、用的是哪一小块投影"，两样都是 **pass 自己的
                            //    参数**（`viewport` + `params`），与材质无关。
                            material: draw.material.clone(),
                        }];
                        page.viewport = Some([
                            window[0] as f32,
                            window[1] as f32,
                            (window[2] - window[0]) as f32,
                            (window[3] - window[1]) as f32,
                        ]);
                        // 这一页在**面 NDC** 里的矩形：`(中心 x, 中心 y, 半宽, 半高)`，
                        // 四个数都在 `[-1, 1]`。
                        //
                        // ⚠ 传它、而不是传整个 `view_proj`：面的视图（灯位 × 六面朝向 ×
                        //    π/2 投影）今天由宿主按 `cube_face` 算，而那一串是**逐位复刻
                        //    glam/Bevy** 的（§110.1.1，1 ulp 敏感）—— 搬一遍就是再引入一处
                        //    1 ulp 风险。这一页相对那一面只多了"缩到哪一小块"，而那完全由
                        //    `window` 与虚拟面尺寸定，正是这四个数。顶点阶段把面的裁剪坐标
                        //    映射进这一块，等价于给那一页一个缩放后的投影。
                        page.params = BTreeMap::from([(
                            "view_page".to_string(),
                            px_protocol::scene::Value::Quad(page_rect_in_face(
                                window,
                                allocation.lights[light as usize].pages_per_side,
                            )),
                        )]);
                        if index > 0 {
                            // 已经不是本页第一笔了：接着本页第一笔的深度，不清。
                            page.render = load_state(&page.render);
                        }
                        passes.push(page);
                    }
                }
            }
        }
    }
    Ok(Baked {
        resources,
        passes,
        materials,
        material_instances,
    })
}

/// `<宽>x<高>` → 两个数。**只认这一种写法**（与 `px_pass::SizeRule::parse` 的定长那一档
/// 同一个口径）：虚拟影图的 atlas 尺寸上限必须是一个写死的数，否则"这一版最多用多大"
/// 就成了一句要靠执行器去猜的话。
fn parse_fixed_size(text: &str) -> Result<(u32, u32), String> {
    let (width, height) = text
        .split_once('x')
        .ok_or_else(|| format!("帧图资源尺寸 '{text}' 不是 '<宽>x<高>'"))?;
    let width = width
        .trim()
        .parse::<u32>()
        .map_err(|err| format!("尺寸 '{text}' 的宽读不出来：{err}"))?;
    let height = height
        .trim()
        .parse::<u32>()
        .map_err(|err| format!("尺寸 '{text}' 的高读不出来：{err}"))?;
    if width == 0 || height == 0 {
        return Err(format!("尺寸 '{text}' 里有 0"));
    }
    Ok((width, height))
}

/// **虚拟影图的分配**（§本轮）：从这一帧的物体 + 投影灯算出稀疏页表与 atlas 布局。
///
/// 两条前置都按"说得出理由的拒绝"处理，而不是猜一个：
/// - 要投影（`shadow_density > 0`）的物体必须有 `Geometry::bounding_radius` ——
///   没有它就算不出"这个物体在灯看来张开多大的角"（而按 0 算就是**静默没有影**）；
/// - 一盏投影灯一个正密度物体都没有 ⇒ 拒（那是"立了一盏空转的灯"，不是一种画法）。
///
/// ⚠ 返回 `None` = 这一帧没有投影灯（合法：整份资源与那些 pass 都不烘）。
fn shadow_allocation(
    lights: &[px_protocol::scene::Light],
    objects: &[Object],
) -> Result<Option<crate::vshadow::Allocation>, String> {
    let casting: Vec<&px_protocol::scene::Light> =
        lights.iter().filter(|light| light.shadows).collect();
    if casting.is_empty() {
        return Ok(None);
    }
    let mut per_light: Vec<Vec<crate::vshadow::Caster>> = Vec::with_capacity(casting.len());
    for light in &casting {
        let mut casters = Vec::new();
        for object in objects {
            if !object.cast_shadow || object.shadow_density <= 0.0 {
                continue;
            }
            let radius = object.geometry.bounding_radius().ok_or_else(|| {
                format!(
                    "物体 '{}' 要投影（shadow_density {}），而它的几何没有 `bounding_radius`：\
                     虚拟影图要按包围球分页，而没有半径就算不出它占多少页",
                    object.id, object.shadow_density
                )
            })?;
            casters.push(crate::vshadow::Caster {
                id: object.id.clone(),
                radius: radius * object.transform.scale[0].abs(),
                position: [
                    object.transform.translation[0] - light.position[0],
                    object.transform.translation[1] - light.position[1],
                    object.transform.translation[2] - light.position[2],
                ],
                density: object.shadow_density,
            });
        }
        per_light.push(casters);
    }
    let allocation =
        crate::vshadow::allocate(&per_light).map_err(|err| format!("虚拟影图分配不出来：{err}"))?;
    Ok(Some(allocation))
}

/// 一页在**面 NDC** 里的矩形：`(中心 x, 中心 y, 半宽, 半高)`，四个数都在 `[-1, 1]`。
///
/// 面 NDC ↔ 虚拟页格的关系是线性的（一面 `[-1,1]` 摊成 `pages_per_side²` 个页格）：
/// 第 `px` 列占 `[-1 + 2·px/n, -1 + 2·(px+1)/n]`。于是中心与半宽各一条算式 ——
/// 而**这一份算式与采样侧那份必须一致**（那边要从面的方向反算出虚拟 texel 坐标）。
fn page_rect_in_face(window: [u32; 4], pages_per_side: u32) -> [f32; 4] {
    let n = pages_per_side as f32;
    let span = 2.0 / n;
    let x0 = -1.0 + span * (window[0] as f32 / crate::vshadow::PAGE_SIZE as f32);
    let y0 = -1.0 + span * (window[1] as f32 / crate::vshadow::PAGE_SIZE as f32);
    [x0 + span * 0.5, y0 + span * 0.5, span * 0.5, span * 0.5]
}

/// 一页影子 pass 的标签：`<面 pass 的标签>_<页行>_<页列>`。
///
/// ⚠ 页**不是** `cube_face` 那一维的展开（`layer` 仍然是 `灯 × 6 + 面`）：
/// 一页一笔 draw 落在**同一层**里，靠 `PassPlan::viewport` 落在 atlas 的那一格上。
fn page_label(face_label: &str, page_y: u32, page_x: u32) -> String {
    format!("{face_label}_p{page_y}_{page_x}")
}

/// 一页要**清**（`depth=clear(0)`）的 pass：它不画几何，只把那一格压到最远。
///
/// 为什么需要它：atlas 是**共享**的（这一页的邻居会被别的物体写到），
/// 而 `depth=load` 会把上一帧 / 上一笔留在那一格里的深度接着用 —— 那是"影子凭空多一块"。
fn page_clear_pass(template: &PassSpec, window: [u32; 4], label: String) -> PassSpec {
    let mut pass = template.clone();
    pass.label = label;
    pass.draws.clear();
    pass.render = clear_state(&template.render);
    pass.viewport = Some([
        window[0] as f32,
        window[1] as f32,
        (window[2] - window[0]) as f32,
        (window[3] - window[1]) as f32,
    ]);
    pass
}

/// 一页要**清**（`depth=clear(0)`）的状态：把 `depth` 那一栏换成 `clear(0)`。
///
/// 为什么需要它：atlas 是**共享**的（这一页的邻居会被别的物体写到），
/// 而 `depth=load` 会把上一帧 / 上一笔留在那一格里的深度接着用 —— 那是"影子凭空多一块"。
///
/// ⚠ 只换 depth 那一栏：`color=none` / `depth_write` / `compare` / `winding` 照抄 ——
/// 换多了就是"渲染器的形状"在烘图侧被改了一遍。
fn clear_state(render: &str) -> String {
    swap_depth(render, "depth=clear(0)")
}

/// 一页的**后续几笔**：接着本页第一笔那一格的深度（`depth=load`）。
fn load_state(render: &str) -> String {
    swap_depth(render, "depth=load")
}

fn swap_depth(render: &str, want: &str) -> String {
    render
        .split('|')
        .map(|part| {
            if part.starts_with("depth=") {
                want.to_string()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// 烘帧材质时组装 WGSL 用的桩表：**宿主那一张**（`bevy_stub` + 宿主自己的 `view`）。
///
/// ⚠ 为什么不能只用 `bevy_stub`：帧材质是**宿主自有**的 WGSL，它的运行期兑现者就是裸 wgpu
/// 宿主 —— `art/frame/skybox.wgsl` 引的 `view.view_from_clip` 在 Bevy 那张**近似**表里
/// 没有（Bevy 真正的 `View` 有七十多个字段，那张表只有五格）。所以这一格必须换成宿主那一份，
/// 而它的文本**只有一处**（`px_shader::assemble::HOST_VIEW_STUB`，宿主与这里共用）。
///
/// ⚠ 内容 shader 的烘图侧走的是 Bevy 那张（`px_graph::shader_schema` 那段注释写了为什么）——
/// 两张表的区别正是"这份 WGSL 是谁的"：内容是 Bevy 宿主与 wgpu 宿主**都要**兑现的，
/// 帧自有材质只由 wgpu 宿主兑现。
fn frame_stubs(symbol: &str) -> Option<&'static str> {
    match symbol {
        "bevy_pbr::mesh_view_bindings::view" => Some(px_shader::assemble::HOST_VIEW_STUB),
        other => px_shader::assemble::bevy_stub(other),
    }
}

/// 一份帧配方材质 → 文档里的 `frame_materials` 一节。
///
/// 三档校验都在**烘图时**做完（三处都在下面点名）：
/// ① 来源名不认识；② WGSL 声明了参数而配方没给来源；③ 类型不符。
/// ④ 配方给了 WGSL 没声明的参数 —— 交给 `px_scene::contract::merge_named`（它会把两张表列出来）。
///
/// 为什么要反射而不是信任配方：参数的**类型**只有 WGSL 说了算（那是契约的真本，
/// 与材质、全屏 pass 走的是同一条路）。烘图时不问，就要等到装载/打包那一刻才报错，
/// 而那时报的是渲染器的错，不是作者写错了配方。
fn bake_material(
    material: &MaterialFile,
    sources: &Sources,
    modules: &px_shader::ModuleTable,
) -> Result<FrameMaterial, String> {
    let at = format!("帧材质 '{}'", material.name);
    let full = px_graph::workspace_root().join(&material.shader);
    let text = std::fs::read_to_string(&full)
        .map_err(|err| format!("{at} 读不了 {}：{err}", full.display()))?;
    let mut seen = Vec::new();
    let assembled = px_shader::assemble::render_source(&text, modules, frame_stubs, &mut seen);
    let layout = px_shader::reflect::reflect_assembled(&assembled, &at)
        .map_err(|err| format!("{at}（{}）反射不出参数块：{err}", material.shader))?;

    // ⑥ 配方给的 `entry` 必须真的在那份 WGSL 里。    //
    // ⚠ 这一条是 §136 补的，代价付过：这里原来写的是 `entry = "fs_main"`（全屏 pass 那条
    //    约定），而 `art/frame/skybox.wgsl` 里那个函数叫 `fragment`。名字指不到东西 ⇒
    //    那份产物**永远建不起来**（运行期 wgpu 报"找不到入口"），而报错离病因已经很远。
    //    反射只读参数块与贴图格，**看不见入口名** —— 所以它必须单独有这一条。
    let entries = px_shader::reflect::entry_points(&assembled, &at)?;
    if !entries
        .iter()
        .any(|(name, stage)| name == &material.entry && *stage == "fragment")
    {
        let list = |stage: &str| -> String {
            let found: Vec<&str> = entries
                .iter()
                .filter(|(_, kind)| *kind == stage)
                .map(|(name, _)| name.as_str())
                .collect();
            if found.is_empty() {
                "（一个都没有）".to_string()
            } else {
                found.join(" / ")
            }
        };
        return Err(format!(
            "{at} 要的片元入口 '{}' 在 {} 里不存在。\n  那份 WGSL 的**片元**入口：{}\n  \
             全部入口：{}",
            material.entry,
            material.shader,
            list("fragment"),
            if entries.is_empty() {
                "（一个都没有）".to_string()
            } else {
                entries
                    .iter()
                    .map(|(name, stage)| format!("{name}（{stage}）"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            }
        ));
    }

    let mut given: BTreeMap<String, toml::Value> = BTreeMap::new();
    // ④ 配方给了 WGSL 没声明的参数 ⇒ 拒，并把这份 shader 声明的参数列出来。
    //    ⚠ 这一条不能省给 `merge_named`：下面那张 `given` 是**按声明的参数**填的，
    //       多出来的名字根本走不到它那里 —— 于是"多给一个参数"会**静默消失**，
    //       而配方里那一行看着还在（§73 禁的那种绿灯）。
    for name in material.params.keys() {
        if layout.param(name).is_none() {
            return Err(format!(
                "{at} 不认识参数 '{name}'：\n  这份 WGSL 声明的参数：{}\n  \
                 ⇒ 要么名字拼错了，要么得先在 shader 的结构体里声明它\
                 （声明之后按名字透传，不用改 Rust）",
                layout.param_names()
            ));
        }
    }
    // 按**声明**的次序走：这样报错的第一句总是"哪个参数"，而不是"配方里哪一行"。
    for slot in &layout.params {
        let Some(value) = material.params.get(&slot.name) else {
            return Err(format!(
                "{at} 的 WGSL 声明了参数 '{}'（{}），而配方没给它**来源**。\n  \
                 认得的来源：{}\n  ⇒ 在 `[[materials]]` 的 params 里加一行 `{} = \"<来源>\"`",
                slot.name,
                slot.kind.name(),
                SOURCES.join(" / "),
                slot.name
            ));
        };
        let Some(source) = value.as_str() else {
            return Err(format!(
                "{at} 的参数 '{}' 给的是 {value}：参数要说**来源**，不是值 ——\n  \
                 帧图是六个场景共用的一份，写死一个数就等于把内容焊进帧策略\
                 （§133：六份场景的亮度今天恰好都是 900，而内容可以不是）。\n  \
                 认得的来源：{}",
                slot.name,
                SOURCES.join(" / ")
            ));
        };
        let Some((resolved, kind)) = source_of(source, sources) else {
            return Err(format!(
                "{at} 的参数 '{}' 说的来源是 '{source}'：**不认得这个来源**。\n  \
                 认得的来源：{}",
                slot.name,
                SOURCES.join(" / ")
            ));
        };
        if kind != slot.kind {
            return Err(format!(
                "{at} 的参数 '{}'：来源 '{source}' 是 {}，而 WGSL 里声明的是 {} —— 类型不符\n  \
                 ⇒ 要么改 WGSL 那一格，要么换一个类型对得上的来源（认得的：{}）",
                slot.name,
                kind.name(),
                slot.kind.name(),
                SOURCES.join(" / ")
            ));
        }
        given.insert(slot.name.clone(), toml_of(&resolved));
    }

    let params = crate::contract::merge_named(&at, &given, &[], &layout, BTreeMap::new())
        .map_err(|err| err.to_string())?;
    Ok(FrameMaterial {
        name: material.name.clone(),
        // ⚠ **原文**（组装前的那一份）落进文档：组装是宿主的事（它有自己的桩表），
        //    而"文档里这是什么"必须与"宿主会拿它做什么"分开。
        shader: text,
        entry: material.entry.clone(),
        params,
    })
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

/// 一张帧图**应当烘出哪些标签**（按 `shadow_lights` 把展开的那几条乘开）。
///
/// ⚠ 这个函数只用于**报错时列期望**（[`mismatch`]），所以对展开的那些条目它给的是一句
/// **形状说明**而不是逐条列举：虚拟影图之后展开的不是"灯 × 面"而是"灯 × 面 × 页 × 物体"，
/// 而页数由内容（包围球 / 密度 / 灯位）定 —— 在这里**猜**一个数出来只会给读者一个错的期望。
/// 真正的对账由 [`verify`] 按形状走（它一行一行地读产物里的标签）。
fn expanded_labels(frame: &FrameFile, shadow_lights: usize) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for entry in frame.before.iter().chain(frame.after.iter()) {
        match entry.shadow_faces {
            None => labels.push(entry.label.clone()),
            Some(_) => labels.push(format!(
                "{}_<灯>_<面>_p<页行>_<页列>（× {shadow_lights} 盏灯 × {CUBE_FACES} 面 × 该面分配到的页，\
                 每页一笔 `<...>_c<序号>` 是那一页后续的物体）",
                entry.label
            )),
        }
    }
    labels
}

/// 核对：这份产物是不是用这张帧图烘的。不是就**当场拒**，并说清期望什么、实际是什么。
///
/// ⚠ 展开的那些条目（影子）**不按写死的张数对**：虚拟影图之后它展开成
/// "灯 × 面 × 页 × 物体"，而**页数由内容定**（物体的包围球、`shadow_density`、灯位）。
/// 所以这一档按**形状**走：每一条影子的标签必须是
/// `<标签>_<灯>_<面>_p<页行>_<页列>`，可选后缀 `_c<序号>`（同一页里的后续物体），
/// 灯与面从 `0` / `+x` 起按次序排。
///
/// 为什么还是值得核：它拦的是"换成另一张帧图烘的产物"与"半截产物"这两件事 ——
/// 而这两件事在运行期的表现都是**画面上少东西**，没有任何别的门会响。
pub fn verify(spec: &SceneSpec, frame: &FrameFile, name: &str) -> Result<(), String> {
    let found: Vec<&str> = spec.passes.iter().map(|pass| pass.label.as_str()).collect();
    let mut at = 0_usize;
    for entry in frame.before.iter().chain(frame.after.iter()) {
        match entry.shadow_faces {
            None => {
                if found.get(at) != Some(&entry.label.as_str()) {
                    return Err(mismatch(name, frame, &found, at, &entry.label));
                }
                at += 1;
            }
            Some(faces) => {
                if faces != CUBE_FACES {
                    return Err(format!(
                        "帧图 '{name}' 的 '{}' 写了 shadow_faces = {faces}：cube 只有 {CUBE_FACES} 面",
                        entry.label
                    ));
                }
                // 展开：0 盏或若干盏灯；每盏灯六面；每面若干页；每页一至若干笔。
                let mut light = 0_u32;
                loop {
                    let mut matched_faces = 0_u32;
                    for face in 0..CUBE_FACES {
                        let prefix =
                            format!("{}_{}_{}", entry.label, light, FACE_NAMES[face as usize]);
                        let start = at;
                        while found
                            .get(at)
                            .is_some_and(|label| label.starts_with(&prefix))
                        {
                            at += 1;
                        }
                        if at > start {
                            matched_faces += 1;
                        }
                    }
                    if matched_faces == 0 {
                        break;
                    }
                    if matched_faces != CUBE_FACES {
                        return Err(format!(
                            "帧图 '{name}' 的 '{}' 展开到第 {light} 盏灯时只找到 {matched_faces} 面\
                             （应当是 {CUBE_FACES} 面）：要么产物是半截的，要么标签不是这一条烘的。\
                             实际标签：[{}]",
                            entry.label,
                            found.join(" / ")
                        ));
                    }
                    light += 1;
                }
            }
        }
    }
    if at != found.len() {
        return Err(mismatch(name, frame, &found, at, "（帧图的条目已经走完）"));
    }
    Ok(())
}
/// 对不上时那句话：说清期望什么、实际是什么、在第几个标签上分的岔。
fn mismatch(name: &str, frame: &FrameFile, found: &[&str], at: usize, want: &str) -> String {
    format!(
        "这份产物不是用帧图 '{name}' 烘的：第 {at} 个标签应当是 '{want}'，实际是 '{}'。\
         期望（配方里那些条目，`cube_faces` 的按灯×面展开）：[{}]；\
         实际：[{}]。要么改用烘它的那张帧图（--frame <名>），要么用 --no-frame-graph 走老形状",
        found.get(at).copied().unwrap_or("（没有更多了）"),
        expanded_labels(frame, 1).join(" / "),
        if found.is_empty() {
            "（空）".to_string()
        } else {
            found.join(" / ")
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::scene::{CullMode, Geometry, Material, Transform};

    /// **这几条测试要的 `shaders` 图**：帧图的片元成员按**名字**去那张图的清单里查键
    /// （`manifest_key_of("shaders", node)`），而清单与产物都是**盘上的**（`target/pcg/`）。
    /// 干净 checkout（`target/` 被 gitignore）上它们不存在 ⇒ 这几条当场红
    /// （实测三条：`the_legacy_switch_emits_nothing_at_all` / `the_baked_materials_…` /
    /// `verify_names_the_expected_frame_…`，报"图 'shaders' 的清单读不到"）。
    ///
    /// ⚠ 对策**不是跳过**（本仓那条不变式："任何「跳过」都是判据的敌人"）：缺了就**当场烘**。
    ///   `px_graph::bake_shader_graph()` 就是 `--bin shaders` 调的**同一个函数**，
    ///   键是内容的纯函数 ⇒ 重复烘只是重写同一批字节，而这几条判据在任何 checkout 上都跑得起来。
    fn ensure_shader_graph() {
        if shader_graph_is_ready() {
            return;
        }
        let baked = px_graph::bake_shader_graph().unwrap_or_else(|err| {
            panic!("盘上没有 `shaders` 图的产物，现烘又失败（帧图按名字查它的成员）：{err}")
        });
        assert!(
            shader_graph_is_ready(),
            "现烘了 {} 份入口 shader，清单/产物还是不全",
            baked.len()
        );
    }

    /// 清单**与它点名的每一份产物**都在盘上。
    ///
    /// ⚠ 只查清单文件是不够的：`target/pcg/ab/` 被清过而清单还在（内容寻址的两半是两件事）
    ///   —— 那时 `schema_of(member)` 会在"读不到产物"上红，而错误信息离现场很远。
    fn shader_graph_is_ready() -> bool {
        let root = px_graph::cache_root();
        let Ok(text) = std::fs::read_to_string(root.join("shaders").join("manifest.json")) else {
            return false;
        };
        let Ok(entries) = serde_json::from_str::<Vec<px_graph::ManifestEntry>>(&text) else {
            return false;
        };
        !entries.is_empty()
            && entries.iter().all(|entry| {
                px_protocol::scene::cas_path(&root, &entry.key)
                    .map(|path| path.is_file())
                    .unwrap_or(false)
            })
    }

    fn object(id: &str, alpha: AlphaMode) -> Object {
        let mut material = Material::new(Member::new("shaders", "surface", &"a".repeat(64)));
        material.alpha = alpha;
        material.cull = CullMode::Back;
        Object {
            id: id.to_string(),
            // ⚠ 半径与密度都要给：这一档要烘出**真的**影子 pass，而页是按
            //    "包围球多大、密度多少"分出来的（`shadow_allocation`）。
            //    少了任一个，分配那一步会当场拒（那正是它该做的）。
            geometry: Geometry::primitive(
                "icosphere",
                BTreeMap::from([("radius".to_string(), Value::Num(1.0))]),
            )
            .with_bounding_radius(1.0),
            material,
            transform: Transform::default(),
            cast_shadow: true,
            shadow_density: 256.0,
        }
    }

    /// `select` 按**材质自带的 alpha 档**挑物体 —— 那是 oracle 分相位的依据。
    ///
    /// ⚠ 透明那一档**故意钉住"反序"**：它不是实现细节，而是 oracle 的实测行为
    /// （见 `draws_of` 上那段注释）。哪天有人把 `.rev()` 删掉，这条会红。
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
            opaque
                .iter()
                .map(|d| d.geometry.as_str())
                .collect::<Vec<_>>(),
            vec!["planet"]
        );
        let transparent = draws_of(&objects, "transparent");
        assert_eq!(
            transparent
                .iter()
                .map(|d| d.geometry.as_str())
                .collect::<Vec<_>>(),
            // `objects[]` 是 [planet, atmosphere, clouds, rings]，实测 oracle 画的是它的反序。
            vec!["rings", "clouds", "atmosphere"]
        );
        // 一整笔的几何与材质用**同一个名字**（物体 id）：宿主按它查顶点数据与材质。
        assert_eq!(transparent[0].material, "rings");
        // 天空盒不是物体：程序化的三个顶点。
        let sky = draws_of(&objects, "skybox");
        assert_eq!(sky.len(), 1);
        assert_eq!(sky[0].geometry, "skybox");
        assert_eq!(draws_of(&objects, "none").len(), 0);
    }

    /// 透明次序的**主键是 `depth_bias`**，`objects[]` 反序只是**平局规则**。
    ///
    /// 与上一条分开写，因为它们是两件不同的事，而且**两档判据各钉一条**：
    /// · 只按 `objects[]` 反序 ⇒ `orbit-rings` 对、带雾壳的档错；
    /// · 只按 `depth_bias` ⇒ 带雾壳的档对、`orbit-rings` 错（它两笔 bias 都是 0）。
    /// ⚠ 这里的期望值是**锚宿主实测**出来的（`target/rings/` 的仪器），不是推的。
    #[test]
    fn a_transparent_pass_sorts_by_depth_bias_before_the_reversed_order() {
        // 雾壳带 -1.0：它必须**压过**反序 —— 反序本来会给出 [rings, clouds, atmosphere]。
        let mut clouds = object("clouds", AlphaMode::Premultiplied);
        clouds.material.depth_bias = -1.0;
        let objects = vec![
            object("planet", AlphaMode::Opaque),
            object("atmosphere", AlphaMode::Add),
            clouds,
            object("rings", AlphaMode::Blend),
        ];
        assert_eq!(
            draws_of(&objects, "transparent")
                .iter()
                .map(|d| d.geometry.as_str())
                .collect::<Vec<_>>(),
            vec!["clouds", "rings", "atmosphere"]
        );
    }

    /// 这一档的**内容那一边**的值（环境里那两个数）——「六个场景今天恰好都是 900」这件事
    /// 不影响这里：帧材质要的是"从环境取"，取到的值是多少是内容的事。
    fn sources() -> Sources {
        Sources {
            ambient: 80.0,
            skybox_brightness: 900.0,
            shadow_lights: 1,
        }
    }

    /// 这一档的灯表：一盏在 `(-4.2, 1.15, 2.35)` 的点光，**开影子**。
    ///
    /// ⚠ 它必须真的开影子：`sources().shadow_lights == 1`，而虚拟影图的分配是**烘图侧**
    /// 算的 —— 两处对不上（说了一盏投影灯、实际一盏都没有）就是当场拒。
    fn lights() -> Vec<px_protocol::scene::Light> {
        vec![
            px_protocol::scene::Light::point("sun", [-4.2, 1.15, 2.35], [1.0, 1.0, 1.0], 7.6e5)
                .with_shadows(true),
        ]
    }

    /// 兼容逃生门：不给帧图 ⇒ 三节都空（产物逐字节回到老形状）。
    #[test]
    fn the_legacy_switch_emits_nothing_at_all() {
        ensure_shader_graph();
        let frame = load(DEFAULT_FRAME).expect("默认帧图要能读");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), &lights(), false).expect("老形状");
        assert!(baked.resources.is_empty(), "老形状不许有 resources");
        assert!(baked.passes.is_empty(), "老形状不许有 passes");
        assert!(
            baked.materials.is_empty(),
            "老形状不许有 frame_materials（多一节就改产物字节）"
        );
        // 反过来：开着就得真的出东西。
        let baked = build(&frame, &objects, &sources(), &lights(), true).expect("帧图");
        assert!(!baked.resources.is_empty(), "帧图要声明中间目标");
        // ⚠ 标签序列照**形状**核（`verify`）：虚拟影图之后展开的是"灯 × 面 × 页 × 物体"，
        //    而页数由内容定 —— 逐条列举的期望只会变成一份会漂的手抄。
        //    这里把这一份烘出来的标签装进一份最小文档，直接问 `verify` 认不认。
        let mut document = SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: "判据".to_string(),
            environment: px_protocol::scene::Environment::default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            resources: baked.resources.clone(),
            passes: baked.passes.clone(),
            lights: lights(),
            objects: objects.clone(),
            frame_materials: Vec::new(),
            material_instances: Vec::new(),
        };
        document.resources = baked.resources.clone();
        document.passes = baked.passes.clone();
        verify(&document, &frame, DEFAULT_FRAME).expect("自己烘出来的标签自己要认");
        // 展开出来的那些必须**真的**带 cube_face，而且层号是 灯×6 + 面。
        let cubes: Vec<(u32, u32, u32)> = baked
            .passes
            .iter()
            .filter_map(|pass| pass.cube_face.map(|c| (c.light, c.face, c.layer)))
            .collect();
        assert!(
            !cubes.is_empty(),
            "一条 `shadow_faces = 6` 的条目要展开成若干页，每页都带 (灯, 面, 层)"
        );
        for (light, face, layer) in &cubes {
            assert_eq!(
                *layer,
                light * CUBE_FACES + face,
                "层号必须是 灯 × 6 + 面（烘图侧那一份算式）"
            );
        }
        // 六面都得有（页数可以不一样，但每一面至少一页）。
        let mut faces_seen: Vec<u32> = cubes.iter().map(|(_, face, _)| *face).collect();
        faces_seen.sort_unstable();
        faces_seen.dedup();
        assert_eq!(faces_seen.len(), CUBE_FACES as usize, "六面各要有页");
        // ⚠ 帧自有材质（§135）：配方里声明了几份，文档里就该有几份，而且**全文内联**。
        assert_eq!(baked.materials.len(), frame.materials.len());
        let skybox = baked
            .materials
            .iter()
            .find(|material| material.name == "skybox")
            .expect("默认帧图里那份天空盒");
        // ⚠ 入口名是 `fragment`（材质那条约定），不是全屏 pass 的 `fs_main`：
        //    这一格写错时，烘图侧现在会当场拒（见下一条测试）。
        assert_eq!(skybox.entry, "fragment");
        assert!(
            skybox.shader.contains("coords_to_ray_direction"),
            "内联的必须是**文件全文**（不是路径、也不是摘要）：{} 字节",
            skybox.shader.len()
        );
        assert!(
            skybox.shader.contains("#{MATERIAL_BIND_GROUP}"),
            "落进文档的是**组装前**的原文 —— 组装是宿主的事（它有自己的桩表与组号）"
        );
        // 参数是**值**：来源（`environment.skybox_brightness`）在烘图时已经换成了数。
        assert_eq!(
            skybox.params.get("brightness"),
            Some(&Value::Num(900.0)),
            "参数要按反射出来的结构体打包：{:?}",
            skybox.params
        );
    }

    /// 帧材质那三档拒法（§135）：来源不认识 / 声明了没给来源 / 类型不符。
    ///
    /// ⚠ 这三条都是**烘图时**的红：等到装载才发现，报的就是渲染器的错，
    /// 而真正该改的是配方。
    #[test]
    fn a_broken_frame_material_is_refused_at_bake_time() {
        let modules = px_shader::workspace_modules(&px_graph::workspace_root()).expect("模块表");
        let material = |params: &str| -> MaterialFile {
            toml::from_str(&format!(
                "name = \"skybox\"\nshader = \"art/frame/skybox.wgsl\"\nentry = \"fragment\"\nparams = {{ {params} }}\n"
            ))
            .expect("夹具")
        };

        // ① 来源不认识 ⇒ 拒，并把认得的来源列出来。
        let err = bake_material(
            &material("brightness = \"environment.skyboox_brightness\""),
            &sources(),
            &modules,
        )
        .expect_err("不认识的来源 ⇒ 拒");
        assert!(err.contains("environment.skyboox_brightness"), "{err}");
        assert!(
            err.contains("environment.skybox_brightness"),
            "要把认得的来源列出来：{err}"
        );

        // ② WGSL 声明了参数，配方没给来源 ⇒ 拒。
        let err = bake_material(&material(""), &sources(), &modules).expect_err("声明了没给 ⇒ 拒");
        assert!(err.contains("brightness"), "{err}");
        assert!(err.contains("来源"), "{err}");

        // ②′ 给的**是值不是来源** ⇒ 拒（这正是"参数要说来源"那条规矩的钉子）。
        let err = bake_material(&material("brightness = 900.0"), &sources(), &modules)
            .expect_err("给值 ⇒ 拒");
        assert!(err.contains("brightness"), "{err}");
        assert!(err.contains("来源"), "{err}");

        // ③ 类型不符：`brightness` 在 WGSL 里是 `f32`，而 `environment.ambient` 也是 f32
        //    ⇒ 这一档得换个法子造：把参数名换成一个不存在的（那就变成 ④ 了）。
        //    真正的类型不符要一份声明了别的类型的 WGSL —— 用一个临时夹具文本。
        let dir = px_graph::workspace_root()
            .join("target")
            .join("frame-material-fixture");
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        let fixture = dir.join("vec3_param.wgsl");
        std::fs::write(
            &fixture,
            // 与 `view` 无关的一份最小 WGSL：只需要一个 `vec3<f32>` 参数。
            "struct FixtureParams { brightness: vec3<f32> };\n\
             @group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: FixtureParams;\n\
             @fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(params.brightness, 1.0); }\n",
        )
        .expect("写夹具");
        let mut wrong_type = material("brightness = \"environment.skybox_brightness\"");
        wrong_type.shader = "target/frame-material-fixture/vec3_param.wgsl".to_string();
        // ⚠ 夹具那份 WGSL 的入口叫 `fs_main`（它是个**全屏 pass 风格**的最小件），
        //    所以这里要把 entry 一起换掉 —— 否则先撞上的是 ⑥ 那条"入口不存在"，
        //    而这一档要验的是**类型不符**。
        wrong_type.entry = "fs_main".to_string();
        let err = bake_material(&wrong_type, &sources(), &modules).expect_err("类型不符 ⇒ 拒");
        assert!(err.contains("类型不符"), "{err}");
        assert!(err.contains("vec3"), "要说清声明的是哪一档：{err}");

        // ⑥ 入口名指不到东西 ⇒ 拒，并把**实际的入口**列出来（§136 补的那条守卫）。
        //    这一格原来写的是 `fs_main`（全屏 pass 那条约定），而天空盒那份 WGSL 里
        //    那个函数叫 `fragment`：产物烘得出来，运行期 wgpu 才报"找不到入口"。
        let mut wrong_entry = material("brightness = \"environment.skybox_brightness\"");
        wrong_entry.entry = "fs_main".to_string();
        let err = bake_material(&wrong_entry, &sources(), &modules).expect_err("入口不存在 ⇒ 拒");
        assert!(err.contains("fs_main"), "要点名那个指不到的名字：{err}");
        assert!(
            err.contains("fragment"),
            "要把那份 WGSL 实际的入口列出来：{err}"
        );

        // ④ 给了 shader 没声明的参数 ⇒ 拒，并由 `merge_named` 把两张表列出来。
        //    （夹具的 WGSL 只有 `brightness` 一格。）
        let err = bake_material(
            &material("brightness = \"environment.ambient\", gain = \"environment.ambient\""),
            &sources(),
            &modules,
        )
        .expect_err("多给参数 ⇒ 拒");
        assert!(err.contains("gain"), "{err}");
        assert!(
            err.contains("brightness"),
            "要列出 shader 声明的参数：{err}"
        );

        // ⑤ 正面：来源取的就是**环境里的值**（换个亮度，文档里的数就跟着变）。
        let dim = Sources {
            ambient: 80.0,
            skybox_brightness: 1200.0,
            // 帧材质的参数只认环境那两个来源；这一格是给"资源层数与影子 pass 展开"用的，
            // 与这一档（参数取值）无关 —— 填一个说得通的值就行。
            shadow_lights: 1,
        };
        let baked = bake_material(
            &material("brightness = \"environment.skybox_brightness\""),
            &dim,
            &modules,
        )
        .expect("换一个亮度也要烘得出来");
        assert_eq!(baked.params["brightness"], Value::Num(1200.0));
    }

    /// 帧图的形状判据：乒乓对、select 词汇、顶点/片元各归其位。
    #[test]
    fn a_broken_frame_recipe_is_refused_by_name() {
        // ⚠ 取那一条要**按标签**，不按下标：帧图会长（§131 就往 prepass 后面插了 copy_depth），
        //    按下标写的判据会在别人加一条 pass 的那天悄悄指到另一条上去。
        fn entry_mut<'a>(frame: &'a mut FrameFile, label: &str) -> &'a mut EntryFile {
            frame
                .before
                .iter_mut()
                .chain(frame.after.iter_mut())
                .find(|entry| entry.label == label)
                .unwrap_or_else(|| panic!("默认帧图里没有 '{label}' 这一条"))
        }

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
        entry_mut(&mut frame, "prepass").select = Some("clouds".to_string());
        let err = frame.check().expect_err("不认识的 select ⇒ 拒");
        assert!(err.contains("clouds"), "{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        entry_mut(&mut frame, "opaque").fragment_shader = Some("px_grade".to_string());
        let err = frame.check().expect_err("几何 pass 给片元成员 ⇒ 拒");
        assert!(err.contains("属于材质"), "{err}");

        // ⚠ copy 那一档：两栏都必须是空的（拷贝不画东西）——
        //    给了任一条都是"说了没做"，而且要给得出**去哪一栏改**。
        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        entry_mut(&mut frame, "copy_depth").vertex_shader =
            Some("art/frame/vertex_mesh.wgsl".to_string());
        let err = frame.check().expect_err("copy 给顶点阶段 ⇒ 拒");
        assert!(err.contains("拷贝不画东西"), "{err}");
        assert!(err.contains("vertex_shader"), "要指出是哪一栏：{err}");

        let mut frame = load(DEFAULT_FRAME).expect("默认帧图");
        entry_mut(&mut frame, "copy_depth").fragment_shader = Some("blit".to_string());
        let err = frame.check().expect_err("copy 给片元成员 ⇒ 拒");
        assert!(err.contains("拷贝不画东西"), "{err}");
    }

    /// 核对基准产物：标签对不上就报出**期望什么**与**实际是什么**。
    #[test]
    fn verify_names_the_expected_frame_and_what_it_found() {
        ensure_shader_graph();
        let frame = load(DEFAULT_FRAME).expect("默认帧图");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), &lights(), true).expect("帧图");
        let spec = SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: "夹具".to_string(),
            environment: Default::default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            resources: baked.resources,
            passes: baked.passes,
            lights: Vec::new(),
            objects,
            frame_materials: baked.materials,
            material_instances: baked.material_instances,
        };
        verify(&spec, &frame, DEFAULT_FRAME).expect("自己烘的自己认");

        let mut stale = spec.clone();
        stale.passes.truncate(2);
        let err = verify(&stale, &frame, DEFAULT_FRAME).expect_err("少了两条 ⇒ 拒");
        assert!(err.contains(DEFAULT_FRAME), "要说清是哪张帧图：{err}");
        assert!(err.contains("prepass"), "要列出期望的标签：{err}");
    }

    /// 帧图烘出来的文档**自己说得通**：`SceneSpec::check` 那两条帧材质判据
    /// （不与物体撞名、声明了必须有人用）在真配方上必须是绿的 ——
    /// `sky` 那条 pass 的 draw 指的就是 `skybox`，而帧材质那一节正是它。
    ///
    /// ⚠ 这一条在 §135 之前是**红的**（`sky` 的材质名谁都不认识），
    /// 而那时它只在宿主装载时才炸。
    #[test]
    fn the_baked_materials_satisfy_the_document_checks() {
        ensure_shader_graph();
        let frame = load(DEFAULT_FRAME).expect("默认帧图");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), &lights(), true).expect("帧图");
        let spec = SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: "夹具".to_string(),
            environment: Default::default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            resources: baked.resources,
            passes: baked.passes,
            lights: Vec::new(),
            objects,
            frame_materials: baked.materials,
            material_instances: baked.material_instances,
        };
        spec.check()
            .unwrap_or_else(|err| panic!("帧图烘出来的文档要自洽：{err}"));
        // 反过来：把帧材质那一节拿掉，同一条 check 必须拒（证明那两条判据真的在管事）。
        let mut without = spec.clone();
        without.frame_materials.clear();
        let err = without
            .check()
            .expect_err("少了帧材质，`sky` 的 draw 就没人认领了 ⇒ 必须拒");
        assert!(err.contains("skybox"), "{err}");
    }
}
