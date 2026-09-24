//! **调参面板的那一半**（S9）：一份场景 → 它能改哪些数 → 改完怎么烘回来。
//!
//! 它一个 GPU 类型都不碰，所以能单独读、单独验；`panel.rs` 只管把这里的状态画成控件。
//!
//! ## 它为什么是这个形状
//!
//! 三条硬的：
//!
//! 1. **参数的真源是文件**：每个图节点的超参数住在 `art/<图>/<节点>.toml`，
//!    而 `px_cook::cached` 把那份参数的规范 JSON 折进 CAS 的键 ⇒ **改一个数只重算
//!    那个节点与它的下游**，别的节点命中。这是本仓"日常美术迭代 0 编译"的全部机制，
//!    面板不许在它旁边另造一条路（§73 那条裁决的落点）。
//! 2. **烘图的实现不在本进程**：`px_render` 只读产物（`art.rs` 顶上那条）。
//!    算节点的是图程序（`target/<profile>/<图>.exe`），由 `px run <图>` 领着跑。
//!    ⇒ 面板按 [`Cook`] 起**子进程**，不把 `px_*` 拖进来。
//! 3. **工作树里的 `art/` 不许被面板改脏**：它是作品的源码，"我到底改没改"得有一个
//!    能一眼看出来的答案。⇒ 编辑落在**会话副本** `target/pcg/edit/<图>/` 上，
//!    由 `PX_ART`（图侧的 `--store`）换指；`art/` 只在人显式按了 [Save] 之后才动。
//!
//! ## 一条场景是怎么变成"能改哪些数"的
//!
//! ```text
//! art/scene/<配方>.toml        ← 场景配方（名字就是 --edit 那一格）
//!   parts[].graph = "planet"   ┐
//!   members[].height = "height" ├→ 图名（前一段）
//!   skybox = "nebulasky::sky"  ┘
//!        ↓ 每个图名 → art/<图>/*.toml（一个文件 = 一个节点）
//!   每个 TOML 的**每一个标量叶子** = 一个控件（形状由叶子自己的类型定）
//!        ↓ 编辑
//! target/pcg/edit/<图>/<节点>.toml   ← 会话副本（唯一的编辑落点）
//!        ↓ px run <图> --store <副本>
//! target/pcg/<图>/manifest.json + CAS
//! ```
//!
//! ## 两条"改了但看不出来"的路，各堵一次
//!
//! * **改错了文件**（改了一个不是那个节点的 `art/<图>/<节点>.toml`）：`cached` 读的是
//!   **节点名**，面板列的是目录里**每一个** `*.toml` —— 目录里躺着一个没接进管线的
//!   文件（`art/nebula/density.toml` 就是一个）时，它会照列、照存、照烘，而画面**不动**。
//!   那不是缺陷（图脚本本来就可以不接某个文件），但不能**看着像**"改了没反应"：
//!   `px run` 收尾那行会报"本次没有任何节点重算" ⇒ 面板把它原样透给用户（见 `cook.rs`）。
//! * **值一样但字节变了**：编辑口是**保格式**的（`toml_edit`），改一个数不动别的行；
//!   判据在 `mod tests` 里钉着（编辑之后解出来的值必须逐字段相同）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 场景配方名（`art/scene/<名字>.toml`）。
pub type Recipe = String;

/// 一个标量叶子：面板能改的最小单位。
///
/// ⚠ 只有**标量**（整数 / 浮点 / 布尔 / 字符串）。数组按元素拆成一个个标量
/// （`light = [0.25, 0.35, 0.90]` → 三格），树的表和表数组则**不展开** ——
/// 今天 `art/` 下没有那种形状，而为一个不存在的形状造控件是纯负担。
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

impl Scalar {
    /// 面板上那一行显示什么（**不是** TOML 的写法：字符串不带引号，好读）。
    pub fn label(&self) -> String {
        match self {
            Scalar::Int(value) => value.to_string(),
            Scalar::Float(value) => format_float(*value),
            Scalar::Bool(value) => if *value { "on" } else { "off" }.to_string(),
            Scalar::Text(value) => value.clone(),
        }
    }

    /// 它在 TOML 里写成一个什么值（**照叶子的类型写**：整数格不许写成 `12.0`）。
    fn to_toml(&self) -> toml_edit::Value {
        match self {
            Scalar::Int(value) => toml_edit::Value::from(*value),
            Scalar::Float(value) => toml_edit::Value::from(*value),
            Scalar::Bool(value) => toml_edit::Value::from(*value),
            Scalar::Text(value) => toml_edit::Value::from(value.clone()),
        }
    }
}

/// 浮点写成最短的**往返**写法（`1.0` 而不是 `1`、`0.11999999731779099` 一位不丢）。
///
/// ⚠ 这一条是承重的：参数文本进 CAS 的键，而 `serde_json` 那一侧开了 `float_roundtrip`
///   （`Cargo.toml` 里那段注释）。这里写出去的文本要能**原样读回来**，否则
///   "编辑一次没改值"会变成"换了一个键 ⇒ 白烘一遍"。
fn format_float(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{value:.1}");
    }
    let mut text = format!("{value}");
    if text.contains(['e', 'E']) {
        // TOML 也认 `1e-3`，但 `1.0e-3` 更好读，且与 Rust 的 Display 一致。
        return text;
    }
    if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

/// 一个叶子的种类（决定面板给它什么控件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 整数 / 浮点：滑条 + 可编辑的数字。
    Number,
    /// 布尔：勾选框。
    Bool,
    /// 字符串：单行编辑框。
    Text,
}

/// 一个可编辑的格子。
#[derive(Debug, Clone)]
pub struct Field {
    /// 原文件里 `key = …` 的那个 key（数组元素用 `光[0]` 这种下标写法）。
    pub key: String,
    /// 文件里那个 key 的**原文**（`key` 去掉下标装饰之后就是它的一部分）。
    pub kind: Kind,
    /// 编辑前的值（`art/` 里那一份 —— 保存、复位、判"改没改"都用它）。
    pub original: Scalar,
    /// 现在的值（会话副本里的）。
    pub value: Scalar,
    /// 值域提示（只有数字有）。
    pub range: Option<(f64, f64)>,
}

impl Field {
    /// 这个格子在**这一次会话里**被改过吗。
    pub fn dirty(&self) -> bool {
        self.value != self.original
    }
}

/// 一个节点（`art/<图>/<节点>.toml`）与它里面那些格子。
///
/// ⚠ `path` 是**会话副本**里那一份（编辑的落点）；`origin` 是 `art/` 里那一份
///   （保存到哪、复位从哪读）。两份都在手上，面板才不会把"盘上现在是什么"猜错。
#[derive(Debug, Clone)]
pub struct Node {
    pub graph: String,
    pub node: String,
    pub path: PathBuf,
    pub origin: PathBuf,
    pub fields: Vec<Field>,
}

impl Node {
    pub fn dirty(&self) -> bool {
        self.fields.iter().any(Field::dirty)
    }
}

/// **会话参数副本**：一份场景引用到的那些图的 `art/*.toml`，加编辑与落盘。
pub struct ParamStore {
    recipe: Recipe,
    /// 工作区根（`art/` 与 `target/` 都在它下面）。
    root: PathBuf,
    /// 会话副本的根（`target/pcg/edit/`）—— **唯一的编辑落点**。
    store: PathBuf,
    /// 编辑的是哪几张图（顺序 = 烘的顺序，见 [`ParamStore::cook_order`]）。
    graphs: Vec<String>,
    nodes: Vec<Node>,
}

impl ParamStore {
    /// 开一份会话副本并读进来。
    ///
    /// 已经存在的副本**不覆盖**：一个窗口重开时，上一轮还没保存的编辑还在那儿 ——
    /// 那正是"会话"该有的意思（`art/` 才是被保护的那一份）。
    pub fn open(root: &Path, recipe: &str) -> Result<ParamStore, String> {
        let recipe_file = root
            .join("art")
            .join("scene")
            .join(format!("{recipe}.toml"));
        let graphs = graphs_of(&recipe_file)?;
        let store = root.join("target").join("pcg").join("edit");

        let mut nodes = Vec::new();
        for graph in &graphs {
            let art_dir = root.join("art").join(graph);
            if !art_dir.is_dir() {
                return Err(format!(
                    "场景 `{recipe}` 引用了图 `{graph}`，但 {} 不在盘上",
                    art_dir.display()
                ));
            }
            let store_dir = store.join(graph);
            let mut files: Vec<PathBuf> = std::fs::read_dir(&art_dir)
                .map_err(|err| format!("读不了 {}：{err}", art_dir.display()))?
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
                .collect();
            // ⚠ 排序：面板次序与"哪个文件先写"都必须是确定的（不依赖目录遍历顺序）。
            files.sort();
            for origin in files {
                let name = origin
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .ok_or_else(|| format!("{} 没有文件名", origin.display()))?
                    .to_string();
                let path = store_dir.join(format!("{name}.toml"));
                if !path.is_file() {
                    std::fs::create_dir_all(&store_dir)
                        .map_err(|err| format!("建不了 {}：{err}", store_dir.display()))?;
                    let bytes = std::fs::read(&origin)
                        .map_err(|err| format!("读不了 {}：{err}", origin.display()))?;
                    std::fs::write(&path, bytes)
                        .map_err(|err| format!("写不了 {}：{err}", path.display()))?;
                }
                let text = read_text(&path)?;
                let document: toml_edit::DocumentMut = text
                    .parse()
                    .map_err(|err| format!("{} 解不开：{err}", path.display()))?;
                let mut fields = Vec::new();
                collect_fields("", document.as_table(), &mut fields)?;
                nodes.push(Node {
                    graph: graph.clone(),
                    node: name,
                    path,
                    origin,
                    fields,
                });
            }
        }
        Ok(ParamStore {
            recipe: recipe.to_string(),
            root: root.to_path_buf(),
            store,
            graphs,
            nodes,
        })
    }

    pub fn recipe(&self) -> &str {
        &self.recipe
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// 会话副本的根（面板把它显示出来：**改在哪儿**要看得见）。
    pub fn store_root(&self) -> &Path {
        &self.store
    }

    /// 改一个格子。**当场写盘**（写的是会话副本）：`px run` 是另一个进程，
    /// 参数不到位它就读不到 —— 攒在内存里等"提交"多一条会漂的真相。
    pub fn set(&mut self, node: usize, field: usize, value: Scalar) -> Result<(), String> {
        let Some(entry) = self.nodes.get_mut(node) else {
            return Err(format!("没有第 {node} 个节点"));
        };
        let Some(slot) = entry.fields.get_mut(field) else {
            return Err(format!("节点 {} 没有第 {field} 个格子", entry.node));
        };
        if slot.kind != kind_of(&value) {
            return Err(format!(
                "`{}` 是 {:?}，不接受 {:?}",
                slot.key,
                slot.kind,
                kind_of(&value)
            ));
        }
        let text = read_text(&entry.path)?;
        let mut document: toml_edit::DocumentMut = text
            .parse()
            .map_err(|err| format!("{} 解不开：{err}", entry.path.display()))?;
        set_leaf(&mut document, &slot.key, &value.to_toml())?;
        std::fs::write(&entry.path, document.to_string())
            .map_err(|err| format!("写不了 {}：{err}", entry.path.display()))?;
        slot.value = value;
        Ok(())
    }

    /// 把某个节点复位成 `art/` 里那一份（连同盘上的会话副本一起）。
    pub fn reset(&mut self, node: usize) -> Result<(), String> {
        let Some(entry) = self.nodes.get_mut(node) else {
            return Err(format!("没有第 {node} 个节点"));
        };
        let bytes = std::fs::read(&entry.origin)
            .map_err(|err| format!("读不了 {}：{err}", entry.origin.display()))?;
        std::fs::write(&entry.path, &bytes)
            .map_err(|err| format!("写不了 {}：{err}", entry.path.display()))?;
        let text = read_text(&entry.path)?;
        let document: toml_edit::DocumentMut = text
            .parse()
            .map_err(|err| format!("{} 解不开：{err}", entry.path.display()))?;
        let mut fields = Vec::new();
        collect_fields("", document.as_table(), &mut fields)?;
        entry.fields = fields;
        Ok(())
    }

    /// 会话里有改动吗（面板拿它决定 Save 亮不亮）。
    pub fn dirty(&self) -> bool {
        self.nodes.iter().any(Node::dirty)
    }

    /// **写回 `art/`**：把改过的那些节点整个文件复制回去。
    ///
    /// ⚠ 只覆盖**改过**的：没改的文件连一次写盘都不该有（mtime 也是判据的一部分 ——
    ///   "我没动它"要能一眼看出来）。
    pub fn save_to_art(&self) -> Result<Vec<PathBuf>, String> {
        let mut written = Vec::new();
        for entry in self.nodes.iter().filter(|entry| entry.dirty()) {
            let bytes = std::fs::read(&entry.path)
                .map_err(|err| format!("读不了 {}：{err}", entry.path.display()))?;
            std::fs::write(&entry.origin, bytes)
                .map_err(|err| format!("写不了 {}：{err}", entry.origin.display()))?;
            written.push(entry.origin.clone());
        }
        Ok(written)
    }

    /// 烘的顺序：**场景配方里那几张图各烘一次**，被引用的在前。
    ///
    /// ⚠ 顺序不是风格问题：`nebulasky` 吃 `nebula` 的 `emission`，而 `scene` 吃两者的
    ///   清单 —— 反过来烘会拿到上一轮的产物（不报错，画面停在上一版）。
    ///   `graphs_of` 已经把 `skybox` 排在前、`parts` 排在后；这里只保持那一份次序。
    pub fn cook_order(&self) -> &[String] {
        &self.graphs
    }

    /// 工作区根（`cook.rs` 要按它找 `px.exe`）。
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// 会话副本的根（`target/pcg/edit/`）：**面板全部编辑的落点**，与有没有开面板无关。
///
/// ⚠ 单独一个自由函数（不是 `ParamStore` 的方法）：窗口起来的时候还没有 `ParamStore`
///   （`--edit` 没给、或者那份配方开不起来），而那一行日志照样要说清"会话副本在哪"。
pub fn store_root(root: &Path) -> PathBuf {
    root.join("target").join("pcg").join("edit")
}

/// **产物 → 它是哪份配方烘的**：给窗口正在显示的那份产物推一个配方名出来。
///
/// ⚠⚠ **不能拿产物路径的文件名当配方名**：CAS 那份文件叫 `<内容键>.pxart`
///   （`target/pcg/ab/f9/f9029752…pxart`），键与配方名一个字符都不相干。实测踩到过：
///   推出来的"配方名"是一串十六进制，然后面板报"读不到那个 .toml" —— 一句**指错方向**的话。
///
/// 所以只有一条路：**去问写它的那个人**。两条按可靠度排：
///
/// 1. **清单**（`<CAS>/scene/manifest.json`）：它是 `scene` 图收尾时写的，里面有
///    配方名与内容键 ⇒ 拿键拼出 CAS 路径，与手上的产物**逐字比**，命中就交回那个名字。
///    这是**定义上**正确的答案（键是内容的纯函数，而路径是 `cas_path` 拼的）。
/// 2. **扫配方**（`art/scene/*.toml` 的 `name` 栏）：清单读不到（还没烘过场景图）时的兜底。
///    今天 41 份配方的文件名与 `name` 栏逐份核过是一致的，所以这一步也稳。
///
/// 两条都不中 ⇒ `None`：面板会说"给 `--edit` 哪个名"，而不是拿一个猜出来的名字去开。
pub fn derive_recipe(root: &Path, pcg_root: &Path, scene: &Path) -> Option<String> {
    let absolute = if scene.is_absolute() {
        scene.to_path_buf()
    } else {
        root.join(scene)
    };
    // ① 清单：按内容键拼出路径来比（不是按文件名比）。
    if let Some(name) = manifest_recipe(pcg_root, &absolute) {
        return Some(name);
    }
    // ② 兜底：产物名 = 场景 `name` 栏 ⇒ 扫配方把那一份找出来（名字**从配方里读**，
    //    不从产物名反推 `art/scene/<那个名字>.toml` —— 文件名与 `name` 栏是两栏）。
    let stem = absolute.file_stem()?.to_str()?;
    let dir = root.join("art").join("scene");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect();
    files.sort();
    for path in files {
        let text = std::fs::read_to_string(&path).ok()?;
        let document: toml::Value = toml::from_str(&text).ok()?;
        if document.get("name").and_then(toml::Value::as_str) == Some(stem) {
            return path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string);
        }
    }
    None
}

/// 清单里"这个 CAS 路径对应哪份配方"。
///
/// ⚠ 按**算出来的路径**比，不按文件名比：`cas_path` 是拼路径的**唯一**一处，
///   再手写一遍前两位目录的规则就是第二个实现（而它会漂）。
fn manifest_recipe(pcg_root: &Path, scene: &Path) -> Option<String> {
    let text = std::fs::read_to_string(pcg_root.join("scene").join("manifest.json")).ok()?;
    let entries: Vec<serde_json::Value> = serde_json::from_str(&text).ok()?;
    // ⚠ 比的是**同一个文件**，所以两边都过一遍 `canonicalize`：盘上那一条路径可以由
    //   调用方写成相对 / 绝对、`\` 或 `/`、带不带 `..`，而清单里拼出来的是另一种写法。
    //   按字符串比会在"看着像同一个文件"的地方判不中（实测踩到过：`--show` 推过来的
    //   那一条与清单拼出来的逐字不同，于是推不出配方名、面板空着）。
    let wanted = std::fs::canonicalize(scene).unwrap_or_else(|_| scene.to_path_buf());
    for entry in entries {
        let (Some(node), Some(key)) = (
            entry.get("node").and_then(serde_json::Value::as_str),
            entry.get("key").and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        let Ok(path) = px_protocol::scene::cas_path(pcg_root, key) else {
            continue;
        };
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if path == wanted {
            return Some(node.to_string());
        }
    }
    None
}

/// 改一个叶子的值（`set` 那一半；单独拆出来是为了判据能在临时文件上验它）。
fn set_leaf(
    document: &mut toml_edit::DocumentMut,
    key: &str,
    value: &toml_edit::Value,
) -> Result<(), String> {
    let (path, index) = split_key(key);
    let mut item: &mut toml_edit::Item = document.as_item_mut();
    for step in &path {
        item = item
            .get_mut(step)
            .ok_or_else(|| format!("文件里没有 `{}`（走到 `{step}` 就断了）", key))?;
    }
    // ⚠⚠ **数组要先问**：`Item::is_value()` 对数组也回真（数组是一种 `Value`）⇒
    //   先问它的话，`light[1]` 会把**整个数组**换成一个标量（实测：`light = 0.5`，
    //   而没有任何一行报错）。判据 `array_elements_are_individual_fields` 钉的就是这一条。
    if let Some(array) = item.as_array_mut() {
        let index = index.ok_or_else(|| format!("`{key}` 是一个数组，但没给下标"))?;
        // ⚠ 先把长度取出来再可变借：`get_mut` 要 `&mut`，而错误文案里那个 `array.len()`
        //   是 `&` —— 两句写在同一个表达式里就是"同时可变借与不可变借"。
        let length = array.len();
        let slot = array
            .get_mut(index)
            .ok_or_else(|| format!("`{key}` 的第 {index} 格不在（只有 {length} 格）"))?;
        *slot = value.clone();
        return Ok(());
    }
    if item.is_array_of_tables() {
        return Err(format!("`{key}` 是表数组，面板不改它"));
    }
    if item.is_value() {
        *item = toml_edit::Item::Value(value.clone());
        return Ok(());
    }
    Err(format!("`{key}` 既不是值也不是数组（是表？）"))
}

/// `light[1]` → `(["light"], Some(1))`；`shape.width` → `(["shape", "width"], None)`。
///
/// ⚠ 表那一层用 `.` 分开，而数组下标**只出现在最后一段**：`art/` 下今天没有
///   "表数组里的表"那种形状，而为一个不存在的形状写一个通用解析器是本末倒置。
fn split_key(key: &str) -> (Vec<String>, Option<usize>) {
    let (head, index) = match key.rfind('[') {
        Some(at) if key.ends_with(']') => {
            let index = key[at + 1..key.len() - 1].parse::<usize>().ok();
            (&key[..at], index)
        }
        _ => (key, None),
    };
    let path = head.split('.').map(str::to_string).collect();
    (path, index)
}

/// 把一个 TOML 表里**每一个标量叶子**收成 [`Field`]。
fn collect_fields(
    prefix: &str,
    table: &toml_edit::Table,
    out: &mut Vec<Field>,
) -> Result<(), String> {
    for (name, item) in table.iter() {
        let key = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        if item.is_table() {
            collect_fields(&key, item.as_table().expect("上面刚问过"), out)?;
            continue;
        }
        if let Some(array) = item.as_array() {
            if array
                .iter()
                .any(|value| value.is_inline_table() || value.is_array())
            {
                // 表数组 / 嵌套数组：不展开（见 `Scalar` 那条注）。
                continue;
            }
            for (index, value) in array.iter().enumerate() {
                if let Some(scalar) = scalar_of(value) {
                    out.push(field_of(format!("{key}[{index}]"), scalar));
                }
            }
            continue;
        }
        if let Some(scalar) = item.as_value().and_then(scalar_of) {
            out.push(field_of(key, scalar));
        }
    }
    Ok(())
}

/// 一个 TOML 值 → 标量（**日期、表、数组都不算**：面板给不了它们一个有意义的控件）。
fn scalar_of(value: &toml_edit::Value) -> Option<Scalar> {
    match value {
        toml_edit::Value::Integer(value) => Some(Scalar::Int(*value.value())),
        toml_edit::Value::Float(value) => Some(Scalar::Float(*value.value())),
        toml_edit::Value::Boolean(value) => Some(Scalar::Bool(*value.value())),
        toml_edit::Value::String(value) => Some(Scalar::Text(value.value().clone())),
        _ => None,
    }
}

fn kind_of(value: &Scalar) -> Kind {
    match value {
        Scalar::Int(_) | Scalar::Float(_) => Kind::Number,
        Scalar::Bool(_) => Kind::Bool,
        Scalar::Text(_) => Kind::Text,
    }
}

fn field_of(key: String, value: Scalar) -> Field {
    Field {
        kind: kind_of(&value),
        range: range_of(&value),
        key,
        original: value.clone(),
        value,
    }
}

/// 值域提示：按**量级**取一条"两头都动得了"的区间。
///
/// ⚠ 这不是"这个参数允许取什么"（那只有算子自己知道），所以它**不夹取**：
///   面板给的是滑条的一条方便区间 + 一个能写任何数的编辑框。把它当约束用，
///   就等于在面板里编了一份**参数表** —— 而真源在算子那一侧（§73 那族）。
fn range_of(value: &Scalar) -> Option<(f64, f64)> {
    let number = match value {
        Scalar::Int(value) => *value as f64,
        Scalar::Float(value) => *value,
        _ => return None,
    };
    if !number.is_finite() {
        return None;
    }
    let magnitude = number.abs();
    // 0 附近也要有一条能用的区间（`warped.toml` 的 `strength = 0.0` 就是 0）。
    let span = if magnitude > 1e-12 { magnitude } else { 1.0 };
    Some((number - span, number + span))
}

/// 一份场景配方引用了哪些图（**烘的顺序**：天空盒 → parts → 成员）。
///
/// 三种引用都要认，缺一种的症状是"那张图的参数在面板里根本看不见"：
/// * `skybox = "nebulasky::sky"`；
/// * `parts[].graph = "planet"`；
/// * `parts[].members = { height = "planet::height" }` —— ⚠ 那一格是
///   **「本图里的节点名 = 产物成员」**：`height` 是 `planet` 图里的节点名，
///   而 `members` 这一栏**不写图名**（它默认是本 part 的 `graph`）。
///   所以这一条只用来兜底：既没 `graph` 也没 `skybox` 时才拿它当提示。
///   写成 `"planet::height"` 时取前一段，写成裸名时归到当前 part 的 `graph`。
fn graphs_of(recipe_file: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(recipe_file)
        .map_err(|err| format!("读不到场景配方 {}：{err}", recipe_file.display()))?;
    let document: toml::Value = toml::from_str(&text)
        .map_err(|err| format!("场景配方 {} 解不开：{err}", recipe_file.display()))?;

    let mut graphs: Vec<String> = Vec::new();
    let mut push = |graph: &str| {
        let graph = graph.trim();
        if graph.is_empty() || graphs.iter().any(|known| known == graph) {
            return;
        }
        graphs.push(graph.to_string());
    };

    if let Some(skybox) = document.get("skybox").and_then(toml::Value::as_str) {
        if let Some((graph, _)) = skybox.split_once("::") {
            push(graph);
        }
    }
    if let Some(parts) = document.get("parts").and_then(toml::Value::as_array) {
        for part in parts {
            let graph = part.get("graph").and_then(toml::Value::as_str);
            if let Some(graph) = graph {
                push(graph);
            }
            if let Some(members) = part.get("members").and_then(toml::Value::as_table) {
                for value in members.values() {
                    let Some(text) = value.as_str() else { continue };
                    match text.split_once("::") {
                        Some((graph, _)) => push(graph),
                        // 裸名：本图里的节点名 ⇒ 归当前 part 的 graph（它得先有）。
                        None => {
                            if let Some(graph) = graph {
                                push(graph);
                            }
                        }
                    }
                }
            }
        }
    }
    if graphs.is_empty() {
        return Err(format!(
            "{} 里一个图都没引用（skybox / parts 两栏都空）⇒ 没有可调的东西",
            recipe_file.display()
        ));
    }
    Ok(graphs)
}

/// 参数文件按**字节**读、按**字符串**解（`toml_edit` 只认 `&str`）。
///
/// ⚠ 读不出来**不当成"空文件"**：`art/` 下每一个文件都是 UTF-8，读不动就是出事了
///   （安静地当成空 ⇒ 面板显示成"这个节点什么参数都没有"）。
fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不了 {}：{err}", path.display()))?;
    String::from_utf8(bytes).map_err(|err| format!("{} 不是 UTF-8：{err}", path.display()))
}

/// 一个节点里"哪些格子被改过"的摘要（日志与判据用）。
pub fn dirty_summary(nodes: &[Node]) -> BTreeMap<String, Vec<String>> {
    let mut summary = BTreeMap::new();
    for node in nodes.iter().filter(|node| node.dirty()) {
        let changed = node
            .fields
            .iter()
            .filter(|field| field.dirty())
            .map(|field| format!("{}={}", field.key, field.value.label()))
            .collect();
        summary.insert(format!("{}::{}", node.graph, node.node), changed);
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **编辑一个数不许动别的字节。**
    ///
    /// 这是"面板改一个数 ⇒ 只重算那个节点"那条性质的地基：值一样而字节变了
    /// （注释被抹掉、`1.0` 写成 `1`、行尾变了）不会报错，只会让判据与人对不上。
    /// 判据取**解出来的值**：`art/` 里那些注释进不了键，进键的是 `canonical_params` 的值。
    #[test]
    fn editing_one_scalar_leaves_every_other_value_alone() {
        let dir = scratch("one-scalar");
        let path = dir.join("blobs.toml");
        let original = "# 上面这一段注释是参数表的一部分\nfrequency = 1.4\noctaves = 3\ngain = 0.45\nseed = 4107\nzonal = 0.8\n";
        std::fs::write(&path, original).expect("写不进临时文件");

        let text = read_text(&path).unwrap();
        let mut document: toml_edit::DocumentMut = text.parse().unwrap();
        set_leaf(&mut document, "frequency", &Scalar::Float(2.5).to_toml()).unwrap();
        std::fs::write(&path, document.to_string()).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("上面这一段注释是参数表的一部分"),
            "注释被抹掉了：\n{after}"
        );
        let before: toml::Value = toml::from_str(original).unwrap();
        let now: toml::Value = toml::from_str(&after).unwrap();
        assert_eq!(now["frequency"].as_float(), Some(2.5));
        for key in ["octaves", "gain", "seed", "zonal"] {
            assert_eq!(now[key], before[key], "`{key}` 被动了");
        }
    }

    /// **`1.0` 不许写成 `1`**：值的类型是键的一部分（整数格与浮点格是两个键）。
    #[test]
    fn a_float_stays_a_float() {
        assert_eq!(format_float(1.0), "1.0");
        assert_eq!(format_float(-2.0), "-2.0");
        assert_eq!(format_float(0.45), "0.45");
        assert_eq!(format_float(0.0), "0.0");
        // 往返：面板写出去的文本读回来必须是同一个数（一位都不许丢）。
        let awkward = 0.119_999_997_317_790_99_f64;
        assert_eq!(format_float(awkward).parse::<f64>().unwrap(), awkward);
        let tiny = 1.0e-9;
        assert_eq!(format_float(tiny).parse::<f64>().unwrap(), tiny);
    }

    /// 数组按元素拆开，而且**下标要能写回去**（`light[1]` 落回数组的第二格）。
    #[test]
    fn array_elements_are_individual_fields() {
        let text = "light = [0.25, 0.35, 0.90]\nextinction = [20.0, 20.0, 20.0]\n";
        let document: toml_edit::DocumentMut = text.parse().unwrap();
        let mut fields = Vec::new();
        collect_fields("", document.as_table(), &mut fields).unwrap();
        let keys: Vec<&str> = fields.iter().map(|field| field.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "light[0]",
                "light[1]",
                "light[2]",
                "extinction[0]",
                "extinction[1]",
                "extinction[2]"
            ]
        );

        let mut document: toml_edit::DocumentMut = text.parse().unwrap();
        set_leaf(&mut document, "light[1]", &Scalar::Float(0.5).to_toml()).unwrap();
        let after: toml::Value = toml::from_str(&document.to_string()).unwrap();
        let light = after["light"].as_array().unwrap();
        assert_eq!(light[0].as_float(), Some(0.25));
        assert_eq!(light[1].as_float(), Some(0.5));
        assert_eq!(light[2].as_float(), Some(0.9));
    }

    /// 一行一个格子的两种写法都要认：`--store D` 与 `--store=D` 是**同一个开关**。
    /// （这一条钉的是 `px_graph::driver::split_store_args` 的口径在面板这一侧的镜像：
    /// 面板写出去的永远是 `--store D` 那一种。）
    #[test]
    fn a_scene_recipe_names_its_graphs_in_cooking_order() {
        let dir = scratch("recipe");
        let path = dir.join("nebula.toml");
        std::fs::write(
            &path,
            "name = \"nebula\"\nskybox = \"nebulasky::sky\"\n\n[[parts]]\nid = \"occluder\"\nkind = \"planet\"\ngraph = \"planet\"\nmembers = { height = \"height\" }\n",
        )
        .unwrap();
        let graphs = graphs_of(&path).unwrap();
        // 天空盒在前（它吃 nebula 的产物），然后是本 part 的图。
        assert_eq!(graphs, ["nebulasky", "planet"]);
    }

    /// 一个只写 `skybox` 的配方（`art/scene/nebula.toml` 就是这一档）也要认得出来。
    #[test]
    fn a_skybox_only_recipe_still_names_a_graph() {
        let dir = scratch("skybox-only");
        let path = dir.join("sky.toml");
        std::fs::write(&path, "name = \"x\"\nskybox = \"nebulasky::sky\"\n").unwrap();
        assert_eq!(graphs_of(&path).unwrap(), ["nebulasky"]);
    }

    /// **产物 → 配方名**：清单里那一条（`node` + `key`）就是答案，而且**不靠文件名**
    /// ——CAS 那份文件叫 `<内容键>.pxart`，与配方名一个字符都不相干。
    ///
    /// 两条都钉：① 命中清单；② 清单不在时按 `name` 栏扫配方（兜底）。
    /// ⚠ 判据里的路径故意写成**另一种写法**（相对 + `/`），因为盘上那一条可以由调用方
    ///   随便写，而"比的是不是同一个文件"这件事不该由写法决定。
    #[test]
    fn a_scene_artifact_names_the_recipe_that_baked_it() {
        let root = scratch("derive-recipe");
        let pcg_root = root.join("target").join("pcg");
        std::fs::create_dir_all(pcg_root.join("scene")).unwrap();
        std::fs::create_dir_all(root.join("art").join("scene")).unwrap();

        // 一份假的场景产物：内容不重要，**文件名必须是内容键**（这才是真实形状）。
        let key = "f9029752d99725ac2197c4d2178823483be41bf6332bb14105f5737b1e2319de";
        let artifact = px_protocol::scene::cas_path(&pcg_root, key).unwrap();
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, b"x").unwrap();
        std::fs::write(
            pcg_root.join("scene").join("manifest.json"),
            format!("[{{\"node\":\"orbit-bare\",\"key\":\"{key}\"}}]"),
        )
        .unwrap();

        // ① 给**绝对**路径（`\` 与 `/` 混着写也要认）。
        let mixed = PathBuf::from(artifact.display().to_string().replace('/', "\\"));
        assert_eq!(
            derive_recipe(&root, &pcg_root, &mixed).as_deref(),
            Some("orbit-bare")
        );
        // ② 给**相对**路径（`--view --scene target\pcg\...` 那一档）。
        let relative = artifact.strip_prefix(&root).unwrap();
        assert_eq!(
            derive_recipe(&root, &pcg_root, relative).as_deref(),
            Some("orbit-bare")
        );

        // ③ 清单没了（还没烘过 `scene` 图）：按**产物名 = 场景 `name` 栏**扫配方。
        //    ⚠ 这一档只在"产物名就是场景名"时成立 —— 所以另造一份**那样命名**的产物，
        //    而不是拿上面那份内容键命名的（那种名字扫什么配方都对不上，是对的）。
        std::fs::remove_file(pcg_root.join("scene").join("manifest.json")).unwrap();
        std::fs::write(
            root.join("art/scene/soft-e300.toml"),
            "name = \"orbit-soft\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("art/scene/orbit-bare.toml"),
            "name = \"orbit-bare\"\n",
        )
        .unwrap();
        let named = pcg_root.join("orbit-bare.pxart");
        std::fs::write(&named, b"x").unwrap();
        assert_eq!(
            derive_recipe(&root, &pcg_root, &named).as_deref(),
            Some("orbit-bare")
        );
        // ④ 两条都不中 ⇒ `None`（面板会说"给 --edit 哪个名"，不拿猜出来的名字去开）。
        let nameless = pcg_root.join("deadbeef.pxart");
        std::fs::write(&nameless, b"x").unwrap();
        assert_eq!(derive_recipe(&root, &pcg_root, &nameless), None);
    }

    /// 判据用的临时目录：**在 `target/` 里面**（本仓不许往项目外面写东西）。
    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("px_render 住在 workspace 下")
            .join("target")
            .join("edit-tests")
            .join(name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).expect("清不掉上一轮的临时目录");
        }
        std::fs::create_dir_all(&dir).expect("建不了临时目录");
        dir
    }
}
