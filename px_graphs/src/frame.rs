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
    px_ops::workspace_root()
        .join("art")
        .join("frame")
        .join(format!("{name}.toml"))
}

/// `select` 认的取值。**这是烘图侧的词汇**，不是渲染器的：它按物体自己带着的
/// 材质 alpha 档（或者"投不投影"那一格）分组，而那正是 oracle 分相位的依据。
const SELECTS: [&str; 5] = [
    "opaque",
    "transparent",
    "shadow_casters",
    "skybox",
    "none",
];

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
    /// 帧图里那一份 cube 影图的层数与"展开几条影子 pass"都由它算出来：
    /// 每盏投影的点光一个 cube（六层），而**一盏都没有时整份资源与那些 pass 都不烘**
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
/// 为什么绕一圈：参数的校验与打包只有一份实现（`px_graphs::params::merge_named` +
/// `MaterialLayout::pack`，材质与全屏 pass 都走它），而它的入口是 toml 值
/// （配方那一侧本来就是 toml）。在烘图侧再写一份"按名字打包"就是第二个真相。
fn toml_of(value: &Value) -> toml::Value {
    match value {
        Value::Num(number) => toml::Value::Float(*number),
        Value::Text(text) => toml::Value::String(text.clone()),
        Value::Triple(items) => {
            toml::Value::Array(items.iter().map(|v| toml::Value::Float(f64::from(*v))).collect())
        }
        Value::Quad(items) => {
            toml::Value::Array(items.iter().map(|v| toml::Value::Float(f64::from(*v))).collect())
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceFile {
    pub name: String,
    pub format: String,
    pub size: String,
    /// 层数：**来源**（不是数）。今天只认一个来源 `"shadow_cubes"` ——
    /// "每盏投影的点光一个 cube"，由 [`Sources::shadow_lights`] 解出来（§133 同一条口径：
    /// 配方里不写内容值，文档里落**解出来的整数**）。
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
    #[serde(default)]
    pub cube_faces: Option<u32>,
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
                    ))
                }
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
        // ---- 帧自有材质（§135）：**配方形状**那一半 ----------------------------
        //
        // 参数里的**来源对不对**（认不认识、类型对不对）要反射 WGSL 才知道，那一步在
        // `bake_material` 里；这里只判"名字/入口/文件这些一眼能看出来的"。
        let mut names: Vec<&str> = Vec::new();
        for material in &self.materials {
            let at = format!("帧图材质 '{}'", material.name);
            if material.name.trim().is_empty() {
                return Err("帧图有一份 `[[materials]]` 没给 name：draws 是按名字引用它的".to_string());
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
            let full = px_ops::workspace_root().join(&material.shader);
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
            // 每盏投影的点光一个 cube（六层）。
            "shadow_cubes" => sources.shadow_lights * CUBE_FACES as usize,
            other => {
                return Err(format!(
                    "帧图资源 '{}' 的 layers 来源是 '{other}'：这一版只认 'shadow_cubes'\
                     （每盏投影的点光一个 cube）",
                    resource.name
                ))
            }
        };
        if layers == 0 {
            dropped.push(resource.name.clone());
            continue;
        }
        resources.push(PassResource {
            name: resource.name.clone(),
            format: resource.format.clone(),
            size: resource.size.clone(),
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
        let modules = px_shader::workspace_modules(&px_ops::workspace_root())?;
        for material in &frame.materials {
            materials.push(bake_material(material, sources, &modules)?);
        }
    }

    let mut passes: Vec<PassSpec> = Vec::new();
    let mut material_instances: Vec<MaterialInstance> = Vec::new();
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
        let draws = if entry.kind == "geometry" {
            draws_of(objects, entry.select())
        } else {
            Vec::new()
        };
        // ---- 展开：`cube_faces = 6` 的那一条 → **每盏投影的灯 × 六面**各一条 ----
        //
        // ⚠ 只有点光的影子走这条路（§109：`light_id` 是 cube 的下标，层号 = light*6+face）。
        //    第 m 盏投影点光的 cube 下标恒为 m —— 因为宿主那一步排序（开影子的在前、
        //    同档稳定）把投影的那些灯**原样**排在最前面，不依赖那个不可实测的 entity 次序。
        let Some(faces) = entry.cube_faces else {
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
            });
            continue;
        };
        if faces != CUBE_FACES {
            return Err(format!(
                "{at} 的 cube_faces 是 {faces}：cube 只有 {CUBE_FACES} 面\
                 （§109.1：Bevy 就是 6 个单层 pass）"
            ));
        }
        for light in 0..sources.shadow_lights as u32 {
            for face in 0..CUBE_FACES {
                // 这一面的名字：投影的每一笔 draw（哪个物体投影，由 `select` 挑）
                // 在这里**各起一个名字**（`<物体 id>@shadow_<灯>_<面名>`）。
                //
                // ⚠ 名字是**生成**的：人写的那一层（帧配方）仍然只有一份意图 ——
                //    "这一条 pass 画投影的那些物体" + "影子要六面"。手写层长出 N 份材质
                //    才是这个形状要避免的事；生成层里"每个 (pass, 灯, 面) 一个名字"
                //    正是 `.pxart` 作为一种指令流该有的样子（§139 的用户裁决）。
                let face_name = FACE_NAMES[face as usize];
                let face_draws: Vec<DrawSpec> = draws
                    .iter()
                    .map(|draw| DrawSpec {
                        geometry: draw.geometry.clone(),
                        material: instance_name(&draw.material, light, face_name),
                    })
                    .collect();
                // 每一笔改名后的 draw 都对应一份实例：名字是新的，材质照原来那一份。
                for (original, renamed) in draws.iter().zip(&face_draws) {
                    material_instances.push(MaterialInstance {
                        name: renamed.material.clone(),
                        base: original.material.clone(),
                    });
                }
                passes.push(PassSpec {
                    kind: entry.kind.clone(),
                    shader: shader.clone(),
                    // 标签照 oracle 那一条路的形状：`shadow_point_light_{灯}_{面}`
                    // （`light.rs:2107-2111`），面名与 `face_index_to_name` 同一套。
                    label: format!("{}_{}_{}", entry.label, light, face_name),
                    entry: entry.entry.clone(),
                    reads: entry.reads.clone(),
                    writes: entry.writes.clone(),
                    params: params.clone(),
                    draws: face_draws,
                    vertex_shader: vertex_shader.clone(),
                    vertex_entry: vertex_entry.clone(),
                    render: entry.render.clone(),
                    depth_target: entry.depth_target.clone(),
                    cube_face: Some(PassCubeFace {
                        light,
                        face,
                        // ⚠ 算式在这里、只有这里（`light.rs:2075`）；宿主拿到的是数并**对账**。
                        layer: light * CUBE_FACES + face,
                    }),
                });
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

/// 生成出来的材质实例名：`<物体 id>@shadow_<灯>_<面名>`。
///
/// ⚠ 它只是一根**给绑定状态起的名字**（§139）：宿主不认识这个格式，它只按
/// `material_instances` 那张表查"这个名字照的是哪一份材质"，再按用到它的那条 pass
/// 的 `cube_face` 决定「哪一面的 `PassView`」（§148）。所以这个格式**不进任何契约** ——
/// 改它一个字都不会动画面（改的是文档里的字符串，两边一起改）。
fn instance_name(base: &str, light: u32, face_name: &str) -> String {
    format!("{base}@shadow_{light}_{face_name}")
}

/// 烘帧材质时组装 WGSL 用的桩表：**宿主那一张**（`bevy_stub` + 宿主自己的 `view`）。
///
/// ⚠ 为什么不能只用 `bevy_stub`：帧材质是**宿主自有**的 WGSL，它的运行期兑现者就是裸 wgpu
/// 宿主 —— `art/frame/skybox.wgsl` 引的 `view.view_from_clip` 在 Bevy 那张**近似**表里
/// 没有（Bevy 真正的 `View` 有七十多个字段，那张表只有五格）。所以这一格必须换成宿主那一份，
/// 而它的文本**只有一处**（`px_shader::assemble::HOST_VIEW_STUB`，宿主与这里共用）。
///
/// ⚠ 内容 shader 的烘图侧走的是 Bevy 那张（`px_ops::shader_schema` 那段注释写了为什么）——
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
/// ④ 配方给了 WGSL 没声明的参数 —— 交给 `px_graphs::params::merge_named`（它会把两张表列出来）。
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
    let full = px_ops::workspace_root().join(&material.shader);
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

    let params = crate::params::merge_named(&at, &given, &[], &layout, BTreeMap::new())
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

/// 一张帧图**应当烘出哪些标签**（按 `shadow_lights` 把 `cube_faces` 那几条乘开）。
///
/// ⚠ 这一步是"配方 → 标签"的唯一一份算式（`build` 与 `verify` 共用）：两处各写一遍，
/// 漂开的那天就变成"自己烘的自己不认"。
fn expanded_labels(frame: &FrameFile, shadow_lights: usize) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for entry in frame.before.iter().chain(frame.after.iter()) {
        match entry.cube_faces {
            None => labels.push(entry.label.clone()),
            Some(_) => {
                for light in 0..shadow_lights as u32 {
                    for face in 0..CUBE_FACES {
                        labels.push(format!(
                            "{}_{}_{}",
                            entry.label, light, FACE_NAMES[face as usize]
                        ));
                    }
                }
            }
        }
    }
    labels
}

/// 核对：这份产物是不是用这张帧图烘的。不是就**当场拒**，并说清期望什么、实际是什么。
///
/// ⚠ 带 `cube_faces` 的那一条会按"投影的灯数 × 6"展开，而**展开几条由内容定**
/// （一盏投影的灯都没有 ⇒ 一条都不展开，见 `build` 里那段）。所以这一档不能拿一张
/// 写死的标签表去比 —— 它按 `cube_faces` 的形状走：非展开条目**逐个**对上，
/// 展开条目允许出现 **0 或若干个完整的 cube**，但每一组必须是 `<标签>_<灯>_<面>`
/// 且从 `0_+x` 起、按灯与面的次序排。
pub fn verify(spec: &SceneSpec, frame: &FrameFile, name: &str) -> Result<(), String> {
    let found: Vec<&str> = spec.passes.iter().map(|pass| pass.label.as_str()).collect();
    let mut at = 0_usize;
    for entry in frame.before.iter().chain(frame.after.iter()) {
        match entry.cube_faces {
            None => {
                if found.get(at) != Some(&entry.label.as_str()) {
                    return Err(mismatch(name, frame, &found, at, &entry.label));
                }
                at += 1;
            }
            Some(faces) => {
                if faces != CUBE_FACES {
                    return Err(format!(
                        "帧图 '{name}' 的 '{}' 写了 cube_faces = {faces}：cube 只有 {CUBE_FACES} 面",
                        entry.label
                    ));
                }
                // 展开：0 个或若干个 cube，每个 cube 六面、次序照 `CUBE_MAP_FACES`。
                let mut light = 0_u32;
                loop {
                    let mut matched = 0_u32;
                    for face in 0..CUBE_FACES {
                        let want = format!("{}_{}_{}", entry.label, light, FACE_NAMES[face as usize]);
                        if found.get(at) == Some(&want.as_str()) {
                            at += 1;
                            matched += 1;
                        } else {
                            break;
                        }
                    }
                    if matched == 0 {
                        break;
                    }
                    if matched != CUBE_FACES {
                        return Err(format!(
                            "帧图 '{name}' 的 '{}' 展开到第 {light} 盏灯时只找到 {matched} 面\
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
            opaque.iter().map(|d| d.geometry.as_str()).collect::<Vec<_>>(),
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

    /// 兼容逃生门：不给帧图 ⇒ 三节都空（产物逐字节回到老形状）。
    #[test]
    fn the_legacy_switch_emits_nothing_at_all() {
        begin();
        let frame = load(DEFAULT_FRAME).expect("默认帧图要能读");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), false).expect("老形状");
        assert!(baked.resources.is_empty(), "老形状不许有 resources");
        assert!(baked.passes.is_empty(), "老形状不许有 passes");
        assert!(
            baked.materials.is_empty(),
            "老形状不许有 frame_materials（多一节就改产物字节）"
        );
        // 反过来：开着就得真的出东西。
        let baked = build(&frame, &objects, &sources(), true).expect("帧图");
        assert!(!baked.resources.is_empty(), "帧图要声明中间目标");
        // ⚠ 标签序列是"配方条目按灯×面展开"之后的（§139）：`sources()` 给 1 盏投影的灯
        //    ⇒ 那条 `point_shadow` 变成六个标签。算式只有一份（`expanded_labels`），
        //    `verify` 用的也是它 —— 这里再抄一遍就等于给自己留一个"烘的与认的不是一套"。
        assert_eq!(
            baked
                .passes
                .iter()
                .map(|p| p.label.as_str())
                .collect::<Vec<_>>(),
            expanded_labels(&frame, sources().shadow_lights)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        // 展开出来的那六条必须**真的**带 cube_face，而且层号是 灯×6 + 面。
        let cubes: Vec<(u32, u32, u32)> = baked
            .passes
            .iter()
            .filter_map(|pass| pass.cube_face.map(|c| (c.light, c.face, c.layer)))
            .collect();
        assert_eq!(
            cubes,
            (0..CUBE_FACES).map(|face| (0, face, face)).collect::<Vec<_>>(),
            "一条 `cube_faces = 6` 的条目要展开成六个 (灯, 面, 层)"
        );
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
        begin();
        let modules = px_shader::workspace_modules(&px_ops::workspace_root()).expect("模块表");
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
        let err =
            bake_material(&material("brightness = 900.0"), &sources(), &modules).expect_err("给值 ⇒ 拒");
        assert!(err.contains("brightness"), "{err}");
        assert!(err.contains("来源"), "{err}");

        // ③ 类型不符：`brightness` 在 WGSL 里是 `f32`，而 `environment.ambient` 也是 f32
        //    ⇒ 这一档得换个法子造：把参数名换成一个不存在的（那就变成 ④ 了）。
        //    真正的类型不符要一份声明了别的类型的 WGSL —— 用一个临时夹具文本。
        let dir = px_ops::workspace_root().join("target").join("frame-material-fixture");
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
        assert!(err.contains("brightness"), "要列出 shader 声明的参数：{err}");

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
        begin();
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
        begin();
        let frame = load(DEFAULT_FRAME).expect("默认帧图");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), true).expect("帧图");
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
        begin();
        let frame = load(DEFAULT_FRAME).expect("默认帧图");
        let objects = vec![object("planet", AlphaMode::Opaque)];
        let baked = build(&frame, &objects, &sources(), true).expect("帧图");
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
