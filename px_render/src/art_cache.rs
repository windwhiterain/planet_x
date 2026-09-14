//! 产物级 / 派生级资源缓存（`.agents/notes/art-framework.md` §49.5 P1）。
//!
//! 服务端原来是「每请求从零造一遍」：读+解码场、建 46 万条边的 HashMap 审计、
//! 逐 texel 上色 + 整条 mip 链、f16 立方图。这些活的输入只有两样 ——
//! **产物内容**和**渲染参数** —— 所以缓存键就是这两样的拼接，别的什么都不看。
//!
//! 三条纪律：
//!
//! 1. **产物级**的键 = 路径 + 清单里的载荷指纹（§48 的 `AssetManifest.fingerprint`）。
//!    指纹为 0（§48 之前烘的旧产物）⇒ **一律不缓存**，每次现造：键里少了「内容」
//!    这一维时，命中就等于认错了东西。
//! 2. **派生级**的键必须把渲染参数拼进去：`surface_textures` 吃 palette / sea_level，
//!    程序化球面还吃 displace / radius（`flat_sea` 由 palette 决定）。
//!    漏一个就会出现「同一个键、不同内容」——§25.2 那个「画布尺寸没进键」的前科。
//! 3. 命中时把造它那一次的审计文本原样重放。审计（§12.2 的定性手段）是仪器，
//!    不能因为走了缓存就哑掉。
//!
//! 淘汰不是 LRU 而是**按冷落次数**：服务是长期进程（§13），但相邻请求常在 A/B 之间
//! 来回（两个色板、两个消光档），所以一条缓存被连续忽略 `MISS_LIMIT` 次才丢。

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::prelude::*;
use px_protocol::art::{self, ArtBundle};

use crate::planet::{self, Field, Palette, PlanetSpec};

/// 一条缓存最多被连续忽略几次请求就丢掉。至少要是 1，否则 A/B 交替会一直打空。
pub const MISS_LIMIT: u32 = 3;

/// 一次请求里每个输入**躺的位置**。
///
/// diff 靠它把「这次的 height」和「上次的 height」对上 —— 重烘必换 CAS 路径，
/// 只有角色是稳定的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Height,
    Surface,
    Coverage,
    SlopeX,
    SlopeY,
    SlopeZ,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Height => "height",
            Self::Surface => "surface",
            Self::Coverage => "coverage",
            Self::SlopeX => "slope-x",
            Self::SlopeY => "slope-y",
            Self::SlopeZ => "slope-z",
        }
    }
}

/// 产物身份 = 路径 + 载荷指纹。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtKey {
    pub path: PathBuf,
    pub fingerprint: u64,
}

/// 程序化球面的派生键。`palette` 决定 `flat_sea`，`sea_level`/`displace`/`radius`
/// 都直接乘进顶点位置 ⇒ 一个都不能漏。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SurfaceKey {
    pub field: ArtKey,
    pub palette: Palette,
    pub sea_level: u32,
    pub displace: u32,
    pub radius: u32,
}

/// 着色贴图的派生键。**只有这两样**进得去：`displace` / `radius` 只动顶点，不动贴图，
/// 拼进来只会白白多存几份一样的东西。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureKey {
    pub field: ArtKey,
    pub palette: Palette,
    pub sea_level: u32,
}

/// 云覆盖度立方图的派生键：四份产物（覆盖度 + 三个梯度）一起决定那 24 字节×texel。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CoverageKey {
    pub coverage: ArtKey,
    pub slopes: [ArtKey; 3],
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

impl CacheKey for SurfaceKey {
    fn cacheable(&self) -> bool {
        self.field.cacheable()
    }
}

impl CacheKey for TextureKey {
    fn cacheable(&self) -> bool {
        self.field.cacheable()
    }
}

impl CacheKey for CoverageKey {
    fn cacheable(&self) -> bool {
        self.coverage.cacheable() && self.slopes.iter().all(CacheKey::cacheable)
    }
}

/// 查缓存的答案。`hit` 只是给日志用的，不影响值本身。
pub struct Cached<T> {
    pub value: T,
    pub hit: bool,
}

/// 解码好的场 + 它的身份（派生键要拿身份去拼）。
#[derive(Clone)]
pub struct FieldEntry {
    pub key: ArtKey,
    pub field: Arc<Field>,
}

/// 从 `.pxart` 载入并焊好法线的网格。
#[derive(Clone)]
pub struct ReadyMesh {
    pub handle: Handle<Mesh>,
    pub audit: String,
}

/// 程序化球面的网格（位移 + 焊法线）。贴图是另一份条目 —— 它不吃位移与半径。
#[derive(Clone)]
pub struct ReadySphere {
    pub mesh: Handle<Mesh>,
    /// 进 `Response.scene` 的那一段（位移、半径范围、最远顶点）。
    pub displacement: String,
    pub audit: String,
}

/// 逐 texel 上色 + mip 链的着色贴图（熔岩色板还带一张自发光图）。
#[derive(Clone)]
pub struct ReadyTextures {
    pub color: Handle<Image>,
    pub glow: Option<Handle<Image>>,
    pub audit: String,
}

/// 云覆盖度立方图。
#[derive(Clone)]
pub struct ReadyCoverage {
    pub image: Handle<Image>,
    /// 每个面的边长（立方图的 6 个面沿行堆叠）。
    pub face: u32,
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
    fields: Slots<ArtKey, Arc<Field>>,
    meshes: Slots<ArtKey, ReadyMesh>,
    spheres: Slots<SurfaceKey, ReadySphere>,
    textures: Slots<TextureKey, ReadyTextures>,
    coverages: Slots<CoverageKey, ReadyCoverage>,
    /// 本次请求里每个角色拿到的清单（用于 diff）。
    seen: HashMap<Role, ArtBundle>,
    /// 上一批请求的清单。
    last: HashMap<Role, ArtBundle>,
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
        let evicted = self.fields.sweep()
            + self.meshes.sweep()
            + self.spheres.sweep()
            + self.textures.sweep()
            + self.coverages.sweep();
        self.last = std::mem::take(&mut self.seen);
        if evicted > 0 {
            println!("[缓存] 丢掉 {evicted} 条冷落超过 {MISS_LIMIT} 次请求的条目");
        }
        report
    }

    /// 「这一批产物跟上一批比，到底哪个节点变了」。
    fn diff_report(&self) -> String {
        let mut changed: Vec<String> = Vec::new();
        let mut same = 0_usize;
        for (role, bundle) in &self.seen {
            match self.last.get(role) {
                Some(previous) => {
                    let report = art::diff(previous, bundle);
                    if report.is_identical() {
                        same += 1;
                    } else {
                        changed.push(format!(
                            "{}（{}）",
                            role.name(),
                            report
                                .touched()
                                .iter()
                                .map(|id| id.to_string())
                                .collect::<Vec<_>>()
                                .join(",")
                        ));
                    }
                }
                // 第一次见这个角色，或者它换了位置 —— 都算「换了新东西」。
                None => changed.push(format!("{}（新）", role.name())),
            }
        }
        let gone = self
            .last
            .keys()
            .filter(|role| !self.seen.contains_key(role))
            .count();
        if changed.is_empty() {
            return if gone == 0 {
                format!("[产物] 零变化（{same} 份逐项相同）")
            } else {
                // 这一批请求只用到了部分角色（比如不带云）—— 剩下那些没动，不算「变了」。
                format!("[产物] {same} 份零变化；另有 {gone} 个角色本次没用到")
            };
        }
        format!(
            "[产物] {} 份变了：{}；{same} 份逐项相同{}",
            changed.len(),
            changed.join(" / "),
            if gone > 0 {
                format!("；另有 {gone} 个角色本次没用到")
            } else {
                String::new()
            }
        )
    }

    /// 一行缓存台账，给长时间跑的服务盯着用。
    pub fn stats(&self) -> String {
        let total = self.hits + self.misses;
        format!(
            "[缓存] 命中 {}/{}（在册 field {} / mesh {} / sphere {} / texture {} / coverage {}，冷落上限 {MISS_LIMIT}）",
            self.hits,
            total,
            self.fields.len(),
            self.meshes.len(),
            self.spheres.len(),
            self.textures.len(),
            self.coverages.len(),
        )
    }

    /// 解码好的场。键 = 路径 + 载荷指纹。
    pub fn field(&mut self, role: Role, path: &str) -> Result<Cached<FieldEntry>, String> {
        let key = self.identity(role, path)?;
        let Some(key) = key else {
            self.misses += 1;
            return Ok(Cached {
                value: FieldEntry {
                    key: ArtKey {
                        path: PathBuf::from(path),
                        fingerprint: 0,
                    },
                    field: Arc::new(planet::load_field(path)?),
                },
                hit: false,
            });
        };
        if let Some(field) = self.fields.get(&key) {
            self.hits += 1;
            return Ok(Cached {
                value: FieldEntry {
                    key: key.clone(),
                    field,
                },
                hit: true,
            });
        }
        self.misses += 1;
        let field = Arc::new(planet::load_field(path)?);
        self.fields.put(key.clone(), field.clone());
        Ok(Cached {
            value: FieldEntry { key, field },
            hit: false,
        })
    }

    /// 从 `.pxart` 载入的网格（`--mesh` 那条路：载入 + 焊法线 + 审计）。
    pub fn artifact_mesh(
        &mut self,
        role: Role,
        path: &str,
        meshes: &mut Assets<Mesh>,
    ) -> Result<Cached<ReadyMesh>, String> {
        let key = self.identity(role, path)?;
        let Some(key) = key else {
            self.misses += 1;
            return Ok(Cached {
                value: planet::build_artifact_mesh(path, meshes)?,
                hit: false,
            });
        };
        if let Some(ready) = self.meshes.get(&key) {
            self.hits += 1;
            return Ok(Cached { value: ready, hit: true });
        }
        self.misses += 1;
        let ready = planet::build_artifact_mesh(path, meshes)?;
        self.meshes.put(key, ready.clone());
        Ok(Cached {
            value: ready,
            hit: false,
        })
    }

    /// 程序化球面的网格（位移 + 焊法线）。键含全部会进顶点位置的渲染参数。
    pub fn sphere(
        &mut self,
        field: &FieldEntry,
        spec: &PlanetSpec,
        meshes: &mut Assets<Mesh>,
    ) -> Result<Cached<ReadySphere>, String> {
        let key = SurfaceKey {
            field: field.key.clone(),
            palette: spec.palette,
            sea_level: spec.sea_level.to_bits(),
            displace: spec.displace.to_bits(),
            radius: spec.radius.to_bits(),
        };
        if let Some(ready) = self.spheres.get(&key) {
            self.hits += 1;
            return Ok(Cached { value: ready, hit: true });
        }
        self.misses += 1;
        let ready = planet::build_sphere_mesh(meshes, &field.field, spec)?;
        self.spheres.put(key, ready.clone());
        Ok(Cached {
            value: ready,
            hit: false,
        })
    }

    /// 着色贴图（逐 texel 上色 + 整条 mip 链）。两条路（PCG 网格 / 载入网格）共用。
    pub fn textures(
        &mut self,
        field: &FieldEntry,
        spec: &PlanetSpec,
        images: &mut Assets<Image>,
    ) -> Result<Cached<ReadyTextures>, String> {
        let key = TextureKey {
            field: field.key.clone(),
            palette: spec.palette,
            sea_level: spec.sea_level.to_bits(),
        };
        if let Some(ready) = self.textures.get(&key) {
            self.hits += 1;
            return Ok(Cached { value: ready, hit: true });
        }
        self.misses += 1;
        let (color, glow, audit) =
            planet::surface_textures(images, &field.field, spec.palette, spec.sea_level);
        let ready = ReadyTextures { color, glow, audit };
        self.textures.put(key, ready.clone());
        Ok(Cached {
            value: ready,
            hit: false,
        })
    }

    /// 云覆盖度立方图。键 = 覆盖度指纹 + 三个梯度指纹（渲染参数都不进这张图）。
    pub fn coverage(
        &mut self,
        spec: &PlanetSpec,
        images: &mut Assets<Image>,
    ) -> Result<Cached<ReadyCoverage>, String> {
        let coverage_path = spec
            .clouds
            .as_deref()
            .ok_or_else(|| "没有给云覆盖度".to_string())?;
        let slope_paths = spec
            .slope
            .as_ref()
            .ok_or_else(|| "没有给云的梯度场（--cloud-slope x,y,z）".to_string())?;
        let coverage = self.field(Role::Coverage, coverage_path)?;
        let slopes = [
            self.field(Role::SlopeX, &slope_paths[0])?,
            self.field(Role::SlopeY, &slope_paths[1])?,
            self.field(Role::SlopeZ, &slope_paths[2])?,
        ];
        let key = CoverageKey {
            coverage: coverage.value.key.clone(),
            slopes: [
                slopes[0].value.key.clone(),
                slopes[1].value.key.clone(),
                slopes[2].value.key.clone(),
            ],
        };
        if let Some(ready) = self.coverages.get(&key) {
            self.hits += 1;
            return Ok(Cached { value: ready, hit: true });
        }
        self.misses += 1;
        let image = crate::clouds::coverage_image(
            &coverage.value.field,
            &[
                slopes[0].value.field.as_ref(),
                slopes[1].value.field.as_ref(),
                slopes[2].value.field.as_ref(),
            ],
        )?;
        let ready = ReadyCoverage {
            face: coverage.value.field.width.max(1),
            image: images.add(image),
        };
        self.coverages.put(key, ready.clone());
        Ok(Cached {
            value: ready,
            hit: false,
        })
    }

    /// 读清单、记进 `seen`，返回产物身份。指纹为 0 的旧产物返回 `None`（不缓存）。
    fn identity(&mut self, role: Role, path: &str) -> Result<Option<ArtKey>, String> {
        let bundle = art::read_manifest(Path::new(path))?;
        let fingerprint = bundle
            .assets
            .first()
            .map(|asset| asset.fingerprint)
            .unwrap_or(0);
        self.seen.insert(role, bundle);
        if fingerprint == 0 {
            if self.warned.insert(path.to_string()) {
                println!("⚠ 指纹为 0（§48 之前烘的产物），这份不缓存，每次现造：{path}");
            }
            return Ok(None);
        }
        Ok(Some(ArtKey {
            path: canonical(path),
            fingerprint,
        }))
    }
}

/// 只问「这个路径现在的载荷指纹是多少」，不碰缓存状态（viewer 热重载用它代替 mtime）。
pub fn fingerprint_of(path: &str) -> Result<u64, String> {
    let bundle = art::read_manifest(Path::new(path))?;
    Ok(bundle.assets.first().map(|a| a.fingerprint).unwrap_or(0))
}

fn canonical(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(fingerprint: u64) -> ArtKey {
        ArtKey {
            path: PathBuf::from("target/x.pxart"),
            fingerprint,
        }
    }

    #[test]
    fn a_zero_fingerprint_never_enters_the_cache() {
        let mut slots: Slots<ArtKey, u32> = Slots::default();
        slots.put(key(0), 7);
        assert_eq!(slots.len(), 0, "指纹为 0 就是身份不完整，不许进册");
        assert_eq!(slots.get(&key(0)), None);
    }

    #[test]
    fn an_entry_is_dropped_only_after_it_has_been_ignored_miss_limit_times() {
        let mut slots: Slots<ArtKey, u32> = Slots::default();
        slots.put(key(1), 7);
        // sweep 就是「这一轮没被 get 过」；刚放进去的那条也记一次，所以总数是 MISS_LIMIT + 1。
        for round in 1..=MISS_LIMIT {
            assert_eq!(slots.sweep(), 0, "第 {round} 次冷落还不该丢");
        }
        assert_eq!(slots.len(), 1);
        assert_eq!(slots.sweep(), 1, "超过上限就该丢");
        assert_eq!(slots.len(), 0);
    }

    #[test]
    fn being_used_resets_the_cold_counter() {
        let mut slots: Slots<ArtKey, u32> = Slots::default();
        slots.put(key(1), 7);
        slots.sweep();
        slots.sweep();
        assert_eq!(slots.get(&key(1)), Some(7));
        for _ in 0..MISS_LIMIT {
            slots.sweep();
            assert_eq!(slots.get(&key(1)), Some(7), "刚被用过就该重新计数");
        }
    }

    #[test]
    fn a_derived_key_is_only_cacheable_when_every_input_is() {
        let surface = SurfaceKey {
            field: key(0),
            palette: Palette::Rocky,
            sea_level: 0,
            displace: 0,
            radius: 0,
        };
        assert!(!surface.cacheable(), "场没有指纹 ⇒ 派生资源也没有身份");
        let texture = TextureKey {
            field: key(0),
            palette: Palette::Rocky,
            sea_level: 0,
        };
        assert!(!texture.cacheable());
        let coverage = CoverageKey {
            coverage: key(5),
            slopes: [key(1), key(2), key(0)],
        };
        assert!(!coverage.cacheable(), "三个梯度里有一个没指纹就不缓存");
    }

    #[test]
    fn the_same_content_at_two_paths_is_two_entries() {
        // 这是故意的：缓存键含路径，因为「同一个指纹、两个文件」在磁盘上是两份东西，
        // 而真正让同名覆盖失效的是指纹那一半（§48.2）。
        let mut slots: Slots<ArtKey, u32> = Slots::default();
        let mut other = key(9);
        other.path = PathBuf::from("target/y.pxart");
        slots.put(key(9), 1);
        slots.put(other, 2);
        assert_eq!(slots.len(), 2);
    }

    /// 造一份真的 `.pxart`（排布跟 `px_ops::write_artifact` 一样：清单帧在最前）。
    fn write_field_artifact(path: &Path, id: &str, width: u32, height: u32, seed: f32) {
        use px_protocol::art::{AssetKind, AssetManifest};
        use px_protocol::wire::Blob;
        let values: Vec<f32> = (0..width * height)
            .map(|index| ((index as f32 * 0.017 + seed).sin() * 0.5 + 0.5).clamp(0.0, 1.0))
            .collect();
        let blob = Blob::from_f32(vec![height, width], &values);
        // 载荷指纹：拿内容算，别用常量 —— 「同名覆盖」那一条要真的换指纹。
        let fingerprint = values.iter().fold(7_u64, |hash, value| {
            hash.wrapping_mul(31).wrapping_add(value.to_bits() as u64)
        }) | 1;
        let bundle = ArtBundle {
            assets: vec![AssetManifest {
                id: id.to_string(),
                kind: AssetKind::OctahedralField,
                params: Default::default(),
                blobs: vec![blob.header.clone()],
                fingerprint,
                cameras: Vec::new(),
            }],
        };
        let mut bytes = Vec::new();
        px_protocol::stream::write_stream(
            &mut bytes,
            &[
                px_protocol::stream::Frame::Art(bundle),
                px_protocol::stream::Frame::Blob(blob),
            ],
        )
        .unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &bytes).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/art-cache-test");
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(name)
    }

    fn spec(field: &str) -> PlanetSpec {
        PlanetSpec {
            field: field.to_string(),
            mesh: None,
            clouds: None,
            slope: None,
            palette: Palette::Rocky,
            displace: 0.0,
            sea_level: 0.5,
            radius: 1.0,
            spin: 0.0,
            rings: 0.0,
            atmo: 1.0,
            ablate: crate::clouds::Ablate::None,
        }
    }

    /// 这一条是 P1 的核心判据，也是「`cargo check` 过了」替代不了的那半：
    /// 同一个场、同一组渲染参数，**第二次必须全部命中**；参数一改必须重新造。
    #[test]
    fn the_second_request_reuses_everything_and_a_param_change_does_not() {
        let path = scratch("field.pxart");
        write_field_artifact(&path, "height", 64, 64, 0.0);
        let path = path.to_string_lossy().to_string();

        let mut cache = ArtCache::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut images = Assets::<Image>::default();
        let spec = spec(&path);

        cache.begin();
        let first_field = cache.field(Role::Height, &path).unwrap();
        let first_sphere = cache
            .sphere(&first_field.value, &spec, &mut meshes)
            .unwrap();
        let first_textures = cache
            .textures(&first_field.value, &spec, &mut images)
            .unwrap();
        assert!(!first_field.hit && !first_sphere.hit && !first_textures.hit);
        assert!(
            cache.sweep().contains("height（新）"),
            "第一次见这个角色，diff 要说「新」"
        );

        cache.begin();
        let again_field = cache.field(Role::Height, &path).unwrap();
        let again_sphere = cache.sphere(&again_field.value, &spec, &mut meshes).unwrap();
        let again_textures = cache
            .textures(&again_field.value, &spec, &mut images)
            .unwrap();
        assert!(again_field.hit, "同一个场不该解码第二遍");
        assert!(again_sphere.hit, "同一组渲染参数不该重造球面");
        assert!(again_textures.hit, "同一组贴图参数不该重上色");
        assert_eq!(
            again_sphere.value.mesh, first_sphere.value.mesh,
            "命中要给回同一份资产，不是等价的一份"
        );
        assert_eq!(again_textures.value.color, first_textures.value.color);

        // 渲染参数一变，派生资源必须重造 —— 而场还是那份场。
        let mut deeper = spec;
        deeper.sea_level = 0.62;
        cache.begin();
        let field_again = cache.field(Role::Height, &path).unwrap();
        assert!(field_again.hit, "换海平面不该让场重新解码");
        let sphere = cache.sphere(&field_again.value, &deeper, &mut meshes).unwrap();
        let textures = cache
            .textures(&field_again.value, &deeper, &mut images)
            .unwrap();
        assert!(!sphere.hit, "sea_level 进网格键（它决定海平面压平）");
        assert!(!textures.hit, "sea_level 进贴图键");
        assert_ne!(textures.value.color, first_textures.value.color);

        // 场换了内容 ⇒ 场自己重解码，派生资源也跟着换。
        write_field_artifact(Path::new(&path), "height", 64, 64, 2.5);
        cache.begin();
        let reloaded = cache.field(Role::Height, &path).unwrap();
        assert!(!reloaded.hit, "同名覆盖、指纹变了 ⇒ 必须重解码（§48.2 那一档）");
    }
}
