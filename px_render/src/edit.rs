use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub type Recipe = String;

#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

impl Scalar {
    pub fn label(&self) -> String {
        match self {
            Scalar::Int(value) => value.to_string(),
            Scalar::Float(value) => format_float(*value),
            Scalar::Bool(value) => if *value { "on" } else { "off" }.to_string(),
            Scalar::Text(value) => value.clone(),
        }
    }

    fn to_toml(&self) -> toml_edit::Value {
        match self {
            Scalar::Int(value) => toml_edit::Value::from(*value),
            Scalar::Float(value) => toml_edit::Value::from(*value),
            Scalar::Bool(value) => toml_edit::Value::from(*value),
            Scalar::Text(value) => toml_edit::Value::from(value.clone()),
        }
    }
}

fn format_float(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{value:.1}");
    }
    let mut text = format!("{value}");
    if text.contains(['e', 'E']) {
        return text;
    }
    if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Number,
    Bool,
    Text,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub key: String,
    pub kind: Kind,
    pub original: Scalar,
    pub value: Scalar,
    pub range: Option<(f64, f64)>,
}

impl Field {
    pub fn dirty(&self) -> bool {
        self.value != self.original
    }
}

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

pub struct ParamStore {
    recipe: Recipe,
    root: PathBuf,
    store: PathBuf,
    graphs: Vec<String>,
    nodes: Vec<Node>,
}

impl ParamStore {
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

    pub fn store_root(&self) -> &Path {
        &self.store
    }

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

    pub fn dirty(&self) -> bool {
        self.nodes.iter().any(Node::dirty)
    }

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

    pub fn cook_order(&self) -> &[String] {
        &self.graphs
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub fn store_root(root: &Path) -> PathBuf {
    root.join("target").join("pcg").join("edit")
}

pub fn derive_recipe(root: &Path, pcg_root: &Path, scene: &Path) -> Option<String> {
    let absolute = if scene.is_absolute() {
        scene.to_path_buf()
    } else {
        root.join(scene)
    };
    if let Some(name) = manifest_recipe(pcg_root, &absolute) {
        return Some(name);
    }
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

fn manifest_recipe(pcg_root: &Path, scene: &Path) -> Option<String> {
    let text = std::fs::read_to_string(pcg_root.join("scene").join("manifest.json")).ok()?;
    let entries: Vec<serde_json::Value> = serde_json::from_str(&text).ok()?;
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
    if let Some(array) = item.as_array_mut() {
        let index = index.ok_or_else(|| format!("`{key}` 是一个数组，但没给下标"))?;
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
    let span = if magnitude > 1e-12 { magnitude } else { 1.0 };
    Some((number - span, number + span))
}

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

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不了 {}：{err}", path.display()))?;
    String::from_utf8(bytes).map_err(|err| format!("{} 不是 UTF-8：{err}", path.display()))
}

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

    #[test]
    fn a_float_stays_a_float() {
        assert_eq!(format_float(1.0), "1.0");
        assert_eq!(format_float(-2.0), "-2.0");
        assert_eq!(format_float(0.45), "0.45");
        assert_eq!(format_float(0.0), "0.0");
        let awkward = 0.119_999_997_317_790_99_f64;
        assert_eq!(format_float(awkward).parse::<f64>().unwrap(), awkward);
        let tiny = 1.0e-9;
        assert_eq!(format_float(tiny).parse::<f64>().unwrap(), tiny);
    }

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
        assert_eq!(graphs, ["nebulasky", "planet"]);
    }

    #[test]
    fn a_skybox_only_recipe_still_names_a_graph() {
        let dir = scratch("skybox-only");
        let path = dir.join("sky.toml");
        std::fs::write(&path, "name = \"x\"\nskybox = \"nebulasky::sky\"\n").unwrap();
        assert_eq!(graphs_of(&path).unwrap(), ["nebulasky"]);
    }

    #[test]
    fn a_scene_artifact_names_the_recipe_that_baked_it() {
        let root = scratch("derive-recipe");
        let pcg_root = root.join("target").join("pcg");
        std::fs::create_dir_all(pcg_root.join("scene")).unwrap();
        std::fs::create_dir_all(root.join("art").join("scene")).unwrap();

        let key = "f9029752d99725ac2197c4d2178823483be41bf6332bb14105f5737b1e2319de";
        let artifact = px_protocol::scene::cas_path(&pcg_root, key).unwrap();
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, b"x").unwrap();
        std::fs::write(
            pcg_root.join("scene").join("manifest.json"),
            format!("[{{\"node\":\"orbit-bare\",\"key\":\"{key}\"}}]"),
        )
        .unwrap();

        let mixed = PathBuf::from(artifact.display().to_string().replace('/', "\\"));
        assert_eq!(
            derive_recipe(&root, &pcg_root, &mixed).as_deref(),
            Some("orbit-bare")
        );
        let relative = artifact.strip_prefix(&root).unwrap();
        assert_eq!(
            derive_recipe(&root, &pcg_root, relative).as_deref(),
            Some("orbit-bare")
        );

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
        let nameless = pcg_root.join("deadbeef.pxart");
        std::fs::write(&nameless, b"x").unwrap();
        assert_eq!(derive_recipe(&root, &pcg_root, &nameless), None);
    }

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
