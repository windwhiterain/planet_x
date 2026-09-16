//! 产物级资源缓存（`.agents/notes/art/08-renderer.md` §50）。
//!
//! 服务端原来每请求从零造一遍：解场、逐 texel 上色、整条 mip 链、f16 立方图……
//! 通用渲染之后**造**这件事整个搬去了烘图侧（§65）—— 渲染器只剩下三件"读"的活：
//! 网格（载入 + 焊法线 + 缠绕审计）、贴图（产物字节 → `Image`）、shader（产物里的 WGSL 文本）。
//! 这三样的输入只有**产物内容**，所以键就是路径 + 清单里的载荷指纹。
//!
//! 三条纪律：
//!
//! 1. 键 = 路径 + 清单里的载荷指纹（§48 的 `AssetManifest.fingerprint`）。
//!    指纹为 0（§48 之前烘的旧产物）⇒ **一律不缓存**，每次现读：键里少了「内容」
//!    这一维时，命中就等于认错了东西。
//! 2. 贴图还吃**采样器**（同一张图按不同地址模式采是两份不同的 GPU 资源）⇒ 它进键。
//! 3. 命中时把造它那一次的审计文本原样重放。审计（§12.2 的定性手段）是仪器，
//!    不能因为走了缓存就哑掉。
//!
//! 淘汰不是 LRU 而是**按冷落次数**：服务是长期进程（§13），但相邻请求常在 A/B 之间
//! 来回（两个色板、两个消光档），所以一条缓存被连续忽略 `MISS_LIMIT` 次才丢。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;
use std::path::{Path, PathBuf};

use bevy::asset::RenderAssetUsages;
use bevy::image::{
    Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor,
};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension as GpuDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use px_protocol::art::{self, ArtBundle, AssetKind, TextureFormat as ArtFormat, TextureShape};
use px_protocol::scene::Sampler;
use px_protocol::wire::DType;

use crate::mesh;

/// 一条缓存最多被连续忽略几次请求就丢掉。至少要是 1，否则 A/B 交替会一直打空。
pub const MISS_LIMIT: u32 = 3;

/// 产物身份 = 路径 + 载荷指纹。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtKey {
    pub path: PathBuf,
    pub fingerprint: u64,
}

/// 贴图的键 = 产物身份 + 采样器。同一张图两种地址模式就是两份 GPU 资源。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureKey {
    pub art: ArtKey,
    pub sampler: Sampler,
}

/// 「这个键能不能进缓存」。
trait CacheKey {
    fn cacheable(&self) -> bool;
}

impl CacheKey for ArtKey {
    fn cacheable(&self) -> bool {
        self.fingerprint != 0
    }
}

impl CacheKey for TextureKey {
    fn cacheable(&self) -> bool {
        self.art.cacheable()
    }
}

/// 查缓存的答案。`hit` 只是给日志用的，不影响值本身。
pub struct Cached<T> {
    pub value: T,
    pub hit: bool,
}

/// 从 `.pxart` 载入并焊好法线的网格。
#[derive(Clone)]
pub struct ReadyMesh {
    pub handle: Handle<Mesh>,
    pub audit: String,
}

/// 从 `.pxart` 载入的贴图（像素 + mip 链 + 采样器）。
#[derive(Clone)]
pub struct ReadyTexture {
    pub image: Handle<Image>,
    /// 层数：1 = 2D、6 = cube。场景装配拿它跟 shader 声明的维度对账。
    pub layers: u32,
    pub audit: String,
}

/// CAS 里的 WGSL 真本（`kind = Shader` 的 U8 blob）。键 = 路径 + 载荷指纹，与网格同规矩。
///
/// `closure` = 产物烘的时候，它那份 WGSL 的 **include 闭包指纹**（`px_shader`，§52.3）。
/// `None` = 老产物没记过（`px_shader/v1` 时代）⇒ 调用方该当场拒：那一版的键里少了
/// 「include」这一维，认它等于认错东西。
#[derive(Clone)]
pub struct ShaderEntry {
    pub source: String,
    pub closure: Option<u64>,
}

struct Slot<V> {
    value: V,
    /// 连续多少次请求没用上它。
    misses: u32,
}

struct Slots<K, V> {
    entries: HashMap<K, Slot<V>>,
}

// 手写而不是 derive：derive 会给 K/V 也加 Default 约束，而键和值都不需要 Default。
impl<K, V> Default for Slots<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<K: CacheKey + Eq + Hash, V: Clone> Slots<K, V> {
    fn get(&mut self, key: &K) -> Option<V> {
        if !key.cacheable() {
            return None;
        }
        let slot = self.entries.get_mut(key)?;
        slot.misses = 0;
        Some(slot.value.clone())
    }

    fn put(&mut self, key: K, value: V) {
        if !key.cacheable() {
            return;
        }
        self.entries.insert(key, Slot { value, misses: 0 });
    }

    /// 这一轮没被碰过的 +1，越过上限的丢掉。返回丢掉几条。
    fn sweep(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, slot| {
            slot.misses += 1;
            slot.misses <= MISS_LIMIT
        });
        before - self.entries.len()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Resource, Default)]
pub struct ArtCache {
    meshes: Slots<ArtKey, ReadyMesh>,
    textures: Slots<TextureKey, ReadyTexture>,
    shaders: Slots<ArtKey, ShaderEntry>,
    /// 本次请求里每个成员（按"图/节点"的名字）拿到的清单（用于 diff）。
    seen: HashMap<String, ArtBundle>,
    /// 上一批请求的清单。
    last: HashMap<String, ArtBundle>,
    /// 已经提醒过的「指纹为 0」路径，避免每次请求刷屏。
    warned: HashSet<String>,
    hits: u32,
    misses: u32,
}

impl ArtCache {
    /// 每个请求开头叫一次：清掉上一轮的 diff 底稿与计数器。
    pub fn begin(&mut self) {
        self.seen.clear();
        self.hits = 0;
        self.misses = 0;
    }

    /// 一次请求收尾：给出 diff 摘要，顺便按冷落次数淘汰。
    pub fn sweep(&mut self) -> String {
        let report = self.diff_report();
        let evicted = self.meshes.sweep() + self.textures.sweep() + self.shaders.sweep();
        self.last = std::mem::take(&mut self.seen);
        if evicted > 0 {
            println!("[缓存] 丢掉 {evicted} 条冷落超过 {MISS_LIMIT} 次请求的条目");
        }
        report
    }

    /// 「这一批产物跟上一批比，到底哪个成员变了」。按成员名（`图/节点`）配对。
    fn diff_report(&self) -> String {
        let mut changed: Vec<String> = Vec::new();
        let mut same = 0_usize;
        for (label, bundle) in &self.seen {
            match self.last.get(label) {
                Some(previous) => {
                    let report = art::diff(previous, bundle);
                    if report.is_identical() {
                        same += 1;
                    } else {
                        changed.push(format!(
                            "{}（{}）",
                            label,
                            report
                                .touched()
                                .iter()
                                .map(|id| id.to_string())
                                .collect::<Vec<_>>()
                                .join(",")
                        ));
                    }
                }
                None => changed.push(format!("{label}（新）")),
            }
        }
        let gone = self
            .last
            .keys()
            .filter(|label| !self.seen.contains_key(*label))
            .count();
        if changed.is_empty() {
            return if gone == 0 {
                format!("[产物] 零变化（{same} 份逐项相同）")
            } else {
                // 这一批请求只用到了部分成员（比如不带云）—— 剩下那些没动，不算「变了」。
                format!("[产物] {same} 份零变化；另有 {gone} 个成员本次没用到")
            };
        }
        format!(
            "[产物] {} 份变了：{}；{same} 份逐项相同{}",
            changed.len(),
            changed.join(" / "),
            if gone > 0 {
                format!("；另有 {gone} 个成员本次没用到")
            } else {
                String::new()
            }
        )
    }

    /// 一行缓存台账，给长时间跑的服务盯着用。
    pub fn stats(&self) -> String {
        let total = self.hits + self.misses;
        format!(
            "[缓存] 命中 {}/{}（在册 mesh {} / texture {} / shader {}，冷落上限 {MISS_LIMIT}）",
            self.hits,
            total,
            self.meshes.len(),
            self.textures.len(),
            self.shaders.len(),
        )
    }

    /// 载入网格产物（焊法线 + 缠绕审计）。
    pub fn mesh(
        &mut self,
        label: &str,
        path: &str,
        meshes: &mut Assets<Mesh>,
    ) -> Result<Cached<ReadyMesh>, String> {
        let key = self.identity(label, path)?.map(|(key, _)| key);
        let Some(key) = key else {
            self.misses += 1;
            return Ok(Cached {
                value: mesh::build_artifact_mesh(path, meshes)?,
                hit: false,
            });
        };
        if let Some(ready) = self.meshes.get(&key) {
            self.hits += 1;
            return Ok(Cached { value: ready, hit: true });
        }
        self.misses += 1;
        let ready = mesh::build_artifact_mesh(path, meshes)?;
        self.meshes.put(key, ready.clone());
        Ok(Cached {
            value: ready,
            hit: false,
        })
    }

    /// 载入贴图产物（像素 + mip 链 + 采样器 → `Image`）。
    pub fn texture(
        &mut self,
        label: &str,
        path: &str,
        sampler: &Sampler,
        images: &mut Assets<Image>,
    ) -> Result<Cached<ReadyTexture>, String> {
        let key = self.identity(label, path)?.map(|(art, _)| TextureKey {
            art,
            sampler: *sampler,
        });
        let Some(key) = key else {
            self.misses += 1;
            let (image, layers, audit) = load_texture(path, sampler)?;
            return Ok(Cached {
                value: ReadyTexture {
                    image: images.add(image),
                    layers,
                    audit,
                },
                hit: false,
            });
        };
        if let Some(ready) = self.textures.get(&key) {
            self.hits += 1;
            return Ok(Cached { value: ready, hit: true });
        }
        self.misses += 1;
        let (image, layers, audit) = load_texture(path, sampler)?;
        let ready = ReadyTexture {
            image: images.add(image),
            layers,
            audit,
        };
        self.textures.put(key, ready.clone());
        Ok(Cached {
            value: ready,
            hit: false,
        })
    }

    /// shader 的 WGSL 文本。每次请求都从 `.pxart` 里抠一遍那个 U8 blob 没有意义 ——
    /// 而这门缓存与其他几门同规矩：键 = 路径 + 载荷指纹，指纹为 0 就不进册。
    /// 顺带把清单里的**闭包指纹**带出来：装载时要拿它跟盘上现在的闭包对账（§52.3），
    /// 对不上就是"这份产物是拿另一版 include 烘的"。
    pub fn shader(&mut self, path: &str) -> Result<Cached<ShaderEntry>, String> {
        let identity = self.identity(path, path)?;
        // 指纹为 0 的旧产物走到下面那条 `None` 支路（它多半也没记过闭包 ⇒ `closure` 是 None）。
        let closure = identity
            .as_ref()
            .and_then(|(_, params)| px_shader::closure_from_params(params));
        let Some((key, _params)) = identity else {
            self.misses += 1;
            return Ok(Cached {
                value: ShaderEntry {
                    source: art::read_shader(Path::new(path))?,
                    closure,
                },
                hit: false,
            });
        };
        if let Some(entry) = self.shaders.get(&key) {
            self.hits += 1;
            return Ok(Cached {
                value: entry,
                hit: true,
            });
        }
        self.misses += 1;
        let entry = ShaderEntry {
            source: art::read_shader(Path::new(path))?,
            closure,
        };
        self.shaders.put(key, entry.clone());
        Ok(Cached {
            value: entry,
            hit: false,
        })
    }

    /// 读清单、记进 `seen`，返回产物身份 + 第一份资产的**清单参数**（闭包指纹住在那里）。
    /// 指纹为 0 的旧产物返回 `None`（不缓存）。
    fn identity(
        &mut self,
        label: &str,
        path: &str,
    ) -> Result<Option<(ArtKey, BTreeMap<String, f64>)>, String> {
        let bundle = art::read_manifest(Path::new(path))?;
        let first = bundle.assets.first();
        let fingerprint = first.map(|asset| asset.fingerprint).unwrap_or(0);
        let params = first.map(|asset| asset.params.clone()).unwrap_or_default();
        self.seen.insert(label.to_string(), bundle);
        if fingerprint == 0 {
            if self.warned.insert(path.to_string()) {
                println!("⚠ 指纹为 0（§48 之前烘的产物），这份不缓存，每次现读：{path}");
            }
            return Ok(None);
        }
        Ok(Some((
            ArtKey {
                path: canonical(path),
                fingerprint,
            },
            params,
        )))
    }
}

/// 贴图产物 → `Image`。返回 `(图, 层数, 审计)`。
///
/// 产物里的形状（宽高 / 层数 / mip 级数 / 格式）**只信清单参数**，并且逐项与载荷字节数对账：
/// 对不上就是产物坏了，当场报错，不许"按能读的读一部分"。
pub fn load_texture(path: &str, sampler: &Sampler) -> Result<(Image, u32, String), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let frames = px_protocol::stream::read_stream(&mut bytes.as_slice())
        .map_err(|err| format!("{path} 不是产物流：{err}"))?;
    let manifest = frames
        .iter()
        .find_map(|frame| match frame {
            px_protocol::stream::Frame::Art(bundle) => bundle.assets.first(),
            _ => None,
        })
        .ok_or_else(|| format!("{path} 里没有清单帧"))?;
    if manifest.kind != AssetKind::Texture {
        return Err(format!("{path} 不是 Texture 产物：{:?}", manifest.kind));
    }
    let shape = TextureShape::from_params(&manifest.params)
        .map_err(|err| format!("{path}：{err}"))?;
    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            px_protocol::stream::Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .ok_or_else(|| format!("{path} 里没有载荷块"))?;
    let expected = match shape.format {
        ArtFormat::Rgba8Srgb => DType::U8,
        ArtFormat::Rgba16Float => DType::U16,
    };
    if blob.header.dtype != expected {
        return Err(format!(
            "{path}：格式是 {} 但载荷位深是 {:?}（应当是 {expected:?}）",
            shape.format.name(),
            blob.header.dtype
        ));
    }
    if blob.bytes.len() != shape.chain_bytes() {
        return Err(format!(
            "{path}：{}×{}×{} 层 {} 级 mip 应当是 {} 字节，实际 {} 字节",
            shape.width,
            shape.height,
            shape.layers,
            shape.levels,
            shape.chain_bytes(),
            blob.bytes.len()
        ));
    }

    let format = match shape.format {
        ArtFormat::Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
        ArtFormat::Rgba16Float => TextureFormat::Rgba16Float,
    };
    // `new_fill` 的填充数据必须是**整数个像素**（Rgba16Float 是 8 字节一像素）——
    // 给 4 字节的填充会在 debug 下当场炸（`bevy_image` 的断言）。
    let fill: &[u8] = match shape.format {
        ArtFormat::Rgba8Srgb => &[0, 0, 0, 0],
        ArtFormat::Rgba16Float => &[0; 8],
    };
    let mut image = Image::new_fill(
        Extent3d {
            width: shape.width,
            height: shape.height,
            depth_or_array_layers: shape.layers,
        },
        GpuDimension::D2,
        fill,
        format,
        RenderAssetUsages::default(),
    );
    image.data = Some(blob.bytes.clone());
    image.texture_descriptor.mip_level_count = shape.levels;
    if shape.layers == px_protocol::art::CUBE_FACES {
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        });
    }
    image.sampler = ImageSampler::Descriptor(sampler_descriptor(sampler));

    let audit = format!(
        "         贴图 {}：{}×{}×{} 层｜{}｜mip {}｜{} 字节",
        Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default(),
        shape.width,
        shape.height,
        shape.layers,
        shape.format.name(),
        shape.levels,
        blob.bytes.len(),
    );
    Ok((image, shape.layers, audit))
}

fn address_mode(mode: px_protocol::scene::Address) -> ImageAddressMode {
    match mode {
        px_protocol::scene::Address::Repeat => ImageAddressMode::Repeat,
        px_protocol::scene::Address::ClampToEdge => ImageAddressMode::ClampToEdge,
        px_protocol::scene::Address::MirrorRepeat => ImageAddressMode::MirrorRepeat,
    }
}

fn filter_mode(mode: px_protocol::scene::Filter) -> ImageFilterMode {
    match mode {
        px_protocol::scene::Filter::Linear => ImageFilterMode::Linear,
        px_protocol::scene::Filter::Nearest => ImageFilterMode::Nearest,
    }
}

fn sampler_descriptor(sampler: &Sampler) -> ImageSamplerDescriptor {
    ImageSamplerDescriptor {
        address_mode_u: address_mode(sampler.address_u),
        address_mode_v: address_mode(sampler.address_v),
        address_mode_w: address_mode(sampler.address_v),
        mag_filter: filter_mode(sampler.filter),
        min_filter: filter_mode(sampler.filter),
        mipmap_filter: filter_mode(sampler.filter),
        anisotropy_clamp: sampler.anisotropy.clamp(1, 16) as u16,
        ..default()
    }
}

/// 造它那一次的审计文本照打一遍（缓存命中时打的是当时那份）。
///
/// 审计是定性手段（§12.2）：缓存能让它别重算，但**不能让仪器哑掉** ——
/// 「这张图为什么是空的」永远得有人能回答。
pub fn replay(audit: &str, hit: bool) {
    if audit.trim().is_empty() {
        return;
    }
    if hit {
        println!("（缓存命中，下面是造它那一次记下的审计）");
    }
    println!("{}", audit.trim_end());
}

/// 只问「这个路径现在的载荷指纹是多少」，不碰缓存状态（viewer 热重载用它代替 mtime）。
pub fn fingerprint_of(path: &str) -> Result<u64, String> {
    let bundle = art::read_manifest(Path::new(path))?;
    Ok(bundle.assets.first().map(|a| a.fingerprint).unwrap_or(0))
}

fn canonical(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}
