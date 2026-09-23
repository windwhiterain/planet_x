//! **通用场景装配**：一个物体 = 几何 + 材质（可以按 stage 各注册一份）+ 变换。
//!
//! ⚠ 这一层**不认识任何内容**：不认识行星、不认识云、不认识 `art/` 下的路径。
//! 它只知道两件通用的事：
//!
//! 1. **物体表**（[`Object`]）：几何（CAS 网格成员 或 内建图元）+ 材质 + 世界系变换 + 投不投影；
//! 2. **灯 / 相机 / 环境**：文档里那三节。
//!
//! 「行星 / 云 / 大气」那些语义住在 [`crate::recipe`]（配方的适配器）与图程序里 ——
//! 把它们塞进这一层，就等于让每个新场景都要改库（§65 那条"渲染器不再认识行星"的同一条道理，
//! 只是这次守的是**装配这一层**）。
//!
//! ## 两条会咬人的规矩（照旧，不是新发明的）
//!
//! - **物体的插入次序就是文档的次序**，而透明物体的绘制次序**由它算出来**
//!   （`frame::draws_of`：`depth_bias` 升序、平局用反序）⇒ 这一层**不排序**，
//!   顺序是作者说的。乱序不会报错，只会让画面悄悄变。
//! - **id 唯一**，而且重建场景时按 id 配对（宿主那一侧）：重名在 [`SceneBuilder::build`]
//!   就拒，不留到装载。
//!
//! ⚠ 它**不搬** `px_protocol::scene::SceneSpec::check` 那份判据：那份是产物那一层的闸门
//! （schema / 贴图格 / 四元数长度），而这里是作者那一层。两层都拦是**故意的** ——
//! 作者这一层要的是"说清是哪一步写错了"，产物那一层要的是"这份产物自洽吗"。

use std::collections::BTreeMap;
use std::path::Path;

use px_protocol::scene::{
    AlphaMode, CullMode, Environment, Geometry, Light, Material, Member, Object, SceneSpec,
    TextureRef, Transform, Value,
};

use crate::contract::{merge_named, schema_of};
use crate::frame::{self, Sources};
use crate::stage::{self, CheckStages, Content, Registration, Stage};

/// 一份**通用**场景的装配器。
///
/// ```ignore
/// let scene = SceneBuilder::new("orbit")
///     .ambient(80.0)
///     .skybox(stars_member, 900.0)
///     .add(Registration::<Opaque, Surface>::single(surface_params), planet_object)?
///     .add_light(sun)
///     .cameras(px_graph::cameras::review())
///     .expect("clouds")
///     .build()?;
/// ```
///
/// ⚠ 它**不保存** stage 登记：登记在 [`SceneBuilder::object`] / [`SceneBuilder::add`] 那一刻
/// **先对账、再解算**成产物那一份材质（`Registration::freeze`）。于是这一层不用把类型参数
/// 塞进 `Vec`（那正是擦除的入口），而"这一档的参数给全了吗"仍然在**加上去的那一刻**就红。
#[derive(Debug, Clone)]
pub struct SceneBuilder {
    name: String,
    environment: Environment,
    cameras: Vec<px_protocol::art::Camera>,
    expects: Vec<String>,
    lights: Vec<Light>,
    /// ⚠ 次序 = 文档次序 = 透明物体的绘制次序（见本模块文档）。
    objects: Vec<Object>,
}

impl SceneBuilder {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            environment: Environment::default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            lights: Vec::new(),
            objects: Vec::new(),
        }
    }

    /// 环境光强度（文档里的 `environment.ambient`）。
    pub fn ambient(mut self, ambient: f32) -> Self {
        self.environment.ambient = ambient;
        self
    }

    /// 天空盒：一份 cube 贴图成员 + 它的亮度倍率。
    pub fn skybox(mut self, member: Member, brightness: f32) -> Self {
        self.environment.skybox = Some(member);
        self.environment.skybox_brightness = brightness;
        self
    }

    /// 评审相机表（`--sheet` 用）。
    pub fn cameras(mut self, cameras: Vec<px_protocol::art::Camera>) -> Self {
        self.cameras = cameras;
        self
    }

    pub fn camera(mut self, camera: px_protocol::art::Camera) -> Self {
        self.cameras.push(camera);
        self
    }

    /// 内容自己声明的**期望标签**（渲染器不认识它们，只逐字交给报告）。
    pub fn expect(mut self, label: &str) -> Self {
        self.expects.push(label.to_string());
        self
    }

    pub fn add_light(mut self, light: Light) -> Self {
        self.lights.push(light);
        self
    }

    /// 登记一个物体：**先逐档对账，再解算成产物那一份材质**。
    ///
    /// 三样都在类型里：`S` = 这个物体被哪一档画（不透明 / 透明），`M` = 哪份内容。
    /// 对账走 `M::Stages` 那张类型级清单（漏写某一档的 `StageParams` 是编译错误）。
    pub fn add<S, M>(
        mut self,
        registration: Registration<S, M>,
        mut object: Object,
    ) -> Result<Self, String>
    where
        S: Stage,
        M: Content,
        Registration<S, M>: CheckStages,
    {
        self.check_id(&object.id)?;
        registration
            .check_all()
            .map_err(|err| format!("场景 '{}' 的物体 '{}'：{err}", self.name, object.id))?;
        object.material.params = registration.freeze(stage_of(&object.material));
        self.objects.push(object);
        Ok(self)
    }

    /// 寻常那一路：几何 + **一份登记的材质**。
    pub fn object<S, M>(
        self,
        id: &str,
        geometry: Geometry,
        registration: Registration<S, M>,
    ) -> Result<Self, String>
    where
        S: Stage,
        M: Content,
        Registration<S, M>: CheckStages,
    {
        let params = registration
            .params_of(S::STAGE_NAME)
            .cloned()
            .unwrap_or_default();
        let object = Object {
            id: id.to_string(),
            geometry,
            material: Material {
                params,
                ..Material::new(Member::new("shaders", M::name(), &"0".repeat(64)))
            },
            transform: Transform::default(),
            cast_shadow: true,
        };
        self.add(registration, object)
    }

    /// 一个内建图元 + 一份登记的材质（"寻常引擎"最短的那条路）。
    pub fn primitive<S, M>(
        self,
        id: &str,
        name: &str,
        params: BTreeMap<String, Value>,
        registration: Registration<S, M>,
    ) -> Result<Self, String>
    where
        S: Stage,
        M: Content,
        Registration<S, M>: CheckStages,
    {
        self.object(id, Geometry::primitive(name, params), registration)
    }

    /// 一份 CAS 网格成员 + 一份登记的材质。
    pub fn mesh<S, M>(
        self,
        id: &str,
        member: Member,
        registration: Registration<S, M>,
    ) -> Result<Self, String>
    where
        S: Stage,
        M: Content,
        Registration<S, M>: CheckStages,
    {
        self.object(id, Geometry::mesh(member), registration)
    }

    /// **产物那一份文档**：不跑帧图（等于 `--no-frame-graph` 那条老形状）。
    pub fn document(self) -> SceneSpec {
        SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: self.name,
            environment: self.environment,
            cameras: self.cameras,
            expects: self.expects,
            resources: Vec::new(),
            passes: Vec::new(),
            lights: self.lights,
            objects: self.objects,
            frame_materials: Vec::new(),
            material_instances: Vec::new(),
        }
    }

    /// 装配 + 自检 ⇒ 一份**通用渲染文档**。
    ///
    /// 自检在产物那一层是 `SceneSpec::check`（`schema` / 贴图格 / 四元数长度）；这里只做
    /// 作者这一层能说得更清楚的那两条（**id 唯一**、逐 stage 的 per-pass 对账），
    /// 最后仍然过一遍 `check` —— 两道闸门都在，不是二选一。
    pub fn build(self) -> Result<SceneSpec, String> {
        if self.objects.is_empty() {
            return Err(format!("场景 '{}' 一个物体都没有", self.name));
        }
        let document = self.document();
        document.check()?;
        Ok(document)
    }

    /// 装配 + **烘帧图**：文档的 `resources` / `passes` / `frame_materials` /
    /// `material_instances` 四节由 [`frame::build`] 算出来。
    ///
    /// `sources` 是**内容**那一边的值（帧配方里写的是来源，值在这里给）。
    pub fn bake(self, frame: &frame::FrameFile, sources: &Sources) -> Result<SceneSpec, String> {
        let mut document = self.build()?;
        let baked = frame::build(frame, &document.objects, sources, true)?;
        document.resources = baked.resources;
        document.passes = baked.passes;
        document.frame_materials = baked.materials;
        document.material_instances = baked.material_instances;
        document.check()?;
        Ok(document)
    }

    /// 这一层自己能说得更清楚的那条判据：**id 唯一且非空**。
    fn check_id(&self, id: &str) -> Result<(), String> {
        if id.trim().is_empty() {
            return Err("有个物体没给 id：重建场景时按它配对，空 id 配不上任何东西".to_string());
        }
        if self.objects.iter().any(|object| object.id == id) {
            return Err(format!(
                "物体 id 重了：'{id}'（重建场景时按 id 配对 ⇒ 重名会让宿主认错物体）"
            ));
        }
        Ok(())
    }
}

/// 文档里这个物体被**哪一档**读材质参数 —— 与 `frame::draws_of` 的 `opaque` / `transparent`
/// 两个 `select` **同一套谓词**（不透明走主 pass 的不透明那一档，其余走透明那一档）。
///
/// ⚠ 深度-only 的那两档（预通道 / 影子）不读材质参数，所以不在这个映射里。
pub fn stage_of(material: &Material) -> &'static str {
    if material.alpha == AlphaMode::Opaque {
        stage::Opaque::STAGE_NAME
    } else {
        stage::Transparent::STAGE_NAME
    }
}

/// 一份材质的**作者这一层**的组装：按名字给值，并**当场**按这份 shader 的契约校验。
///
/// 为什么要有它：`Material` 本身是一袋 `BTreeMap<String, Value>` —— 手写它意味着
/// "名字拼错"要到烘图时才红，而这里在**作者写下的那一刻**就红，且报错里带名字与候选表。
///
/// ⚠ 参数的名字 ↔ 字节仍然只有一份真源（shader 的反射，[`crate::contract`]）：
/// 这一层只是把"给值"这件事变得有类型可查，**不**另造一份契约。
pub struct MaterialBuilder {
    shader: Member,
    given: BTreeMap<String, toml::Value>,
    textures: BTreeMap<String, TextureRef>,
    alpha: AlphaMode,
    cull: CullMode,
    depth_bias: f32,
}

impl MaterialBuilder {
    pub fn new(shader: Member) -> Self {
        Self {
            shader,
            given: BTreeMap::new(),
            textures: BTreeMap::new(),
            alpha: AlphaMode::Opaque,
            cull: CullMode::Back,
            depth_bias: 0.0,
        }
    }

    /// 一个数（f32 / u32 / i32 都走它 —— 打出哪一档由契约说）。
    pub fn num(mut self, name: &str, value: f64) -> Self {
        self.given
            .insert(name.to_string(), toml::Value::Float(value));
        self
    }

    /// 一个整数（契约里是 `u32` / `i32` 的那些格）。
    pub fn int(mut self, name: &str, value: i64) -> Self {
        self.given
            .insert(name.to_string(), toml::Value::Integer(value));
        self
    }

    pub fn vec3(mut self, name: &str, value: [f32; 3]) -> Self {
        self.given.insert(
            name.to_string(),
            toml::Value::Array(
                value
                    .iter()
                    .map(|x| toml::Value::Float(f64::from(*x)))
                    .collect(),
            ),
        );
        self
    }

    pub fn vec4(mut self, name: &str, value: [f32; 4]) -> Self {
        self.given.insert(
            name.to_string(),
            toml::Value::Array(
                value
                    .iter()
                    .map(|x| toml::Value::Float(f64::from(*x)))
                    .collect(),
            ),
        );
        self
    }

    pub fn texture(mut self, role: &str, texture: TextureRef) -> Self {
        self.textures.insert(role.to_string(), texture);
        self
    }

    pub fn alpha(mut self, alpha: AlphaMode) -> Self {
        self.alpha = alpha;
        self
    }

    pub fn cull(mut self, cull: CullMode) -> Self {
        self.cull = cull;
        self
    }

    pub fn depth_bias(mut self, depth_bias: f32) -> Self {
        self.depth_bias = depth_bias;
        self
    }

    /// **按这份 shader 的契约**校验并打包 ⇒ 一份产物里的材质。
    ///
    /// 三档都在这里红：名字不认识（`merge_named` 把两张表列出来）、声明了没人给
    /// （`layout.pack`）、类型不符（`coerce_value`）。`root` 是 CAS 根。
    pub fn build(self, root: &Path) -> Result<Material, String> {
        let layout = schema_of(&self.shader, root)?;
        let params = merge_named(
            &format!("材质 '{}'", self.shader.node),
            &self.given,
            // ⚠ **结构键一张都不跳**：材质的参数表里只有这份 shader 声明过的名字，
            //    而"半径 / 色板 / 灯"那些是**场景**的结构键，根本不该出现在这里。
            &[],
            &layout,
            BTreeMap::new(),
        )
        .map_err(|err| format!("{err}\n  shader 成员：{}", self.shader.node))?;
        let mut material = Material::new(self.shader).with_params(params);
        material.textures = self.textures;
        material.alpha = self.alpha;
        material.cull = self.cull;
        material.depth_bias = self.depth_bias;
        Ok(material)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::{Atmosphere, Content, Opaque, Ring, Surface, Transparent};

    fn params(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect()
    }

    /// 一份给全了的表面参数（`opaque` 那一档要 8 格）。
    fn surface_params() -> BTreeMap<String, Value> {
        params(&[
            ("orientation", Value::Quad([0.0, 0.0, 0.0, 1.0])),
            ("emissive", Value::Quad([0.0, 0.0, 0.0, 0.0])),
            ("inner", Value::Num(1.01)),
            ("outer", Value::Num(1.06)),
            ("coverage", Value::Num(0.35)),
            ("shadow", Value::Num(0.0)),
            ("height", Value::Num(0.5)),
            ("gain", Value::Num(2.0)),
        ])
    }

    fn surface() -> Registration<Opaque, Surface> {
        Registration::single(surface_params())
    }

    fn sphere() -> Geometry {
        Geometry::primitive("icosphere", BTreeMap::new())
    }

    /// **id 唯一**：重名在作者这一层就拒（不留到装载，也不让宿主认错物体）。
    #[test]
    fn a_duplicate_id_is_refused_at_build_time() {
        let err = SceneBuilder::new("夹具")
            .object("body", sphere(), surface())
            .unwrap()
            .object("body", sphere(), surface())
            .expect_err("同一个 id 加两次");
        assert!(err.contains("重了"), "{err}");
        assert!(err.contains("body"), "{err}");
    }

    /// 空 id 也拒（重建场景时按 id 配对，空 id 配不上任何东西）。
    #[test]
    fn an_empty_id_is_refused() {
        let err = SceneBuilder::new("夹具")
            .object("   ", sphere(), surface())
            .expect_err("空 id");
        assert!(err.contains("没给 id"), "{err}");
    }

    /// **次序就是文档次序** —— 这一层不许排序（透明物体的绘制次序由它算）。
    #[test]
    fn insertion_order_is_the_document_order() {
        let document = SceneBuilder::new("夹具")
            .object("one", sphere(), surface())
            .unwrap()
            .object("two", sphere(), surface())
            .unwrap()
            .object("three", sphere(), surface())
            .unwrap()
            .build()
            .expect("三份都合法");
        let ids: Vec<&str> = document
            .objects
            .iter()
            .map(|object| object.id.as_str())
            .collect();
        assert_eq!(ids, vec!["one", "two", "three"]);
    }

    /// 一个空场景不是"能烘的场景"：当场拒，而不是烘出一份没有物体的文档。
    #[test]
    fn an_empty_scene_is_refused() {
        let err = SceneBuilder::new("夹具")
            .build()
            .expect_err("一个物体都没有");
        assert!(err.contains("一个物体都没有"), "{err}");
    }

    /// **逐档对账真的会红**，而且是在**加进去的那一刻**（不是 `build()`）：
    /// surface 那一档要 8 格，这里只给 1 格。
    #[test]
    fn an_incomplete_material_is_refused_when_it_is_added() {
        let thin = Registration::<Opaque, Surface>::single(params(&[(
            "orientation",
            Value::Quad([0.0, 0.0, 0.0, 1.0]),
        )]));
        let err = SceneBuilder::new("夹具")
            .object("body", sphere(), thin)
            .expect_err("少七格");
        assert!(err.contains("body"), "{err}");
        assert!(err.contains("emissive") || err.contains("gain"), "{err}");
        assert!(err.contains("surface"), "要点名是哪份内容：{err}");
    }

    /// **分档登记**：某一档另有参数 ⇒ 产物里落的是**那一档**那一份。
    #[test]
    fn a_stage_override_decides_what_lands_in_the_document() {
        let mut variant = surface_params();
        variant.insert("coverage".to_string(), Value::Num(0.9));
        let registration = surface().stage::<Opaque>(variant);
        let document = SceneBuilder::new("夹具")
            .object("body", sphere(), registration)
            .unwrap()
            .build()
            .expect("两档都给全了");
        assert_eq!(
            document.objects[0].material.params["coverage"],
            Value::Num(0.9),
            "不透明那一档登记的是 variant ⇒ 落盘的是它"
        );
    }

    /// **内容的身份是类型**：产物里那份材质的 shader 节点名来自 `M::name()`，
    /// 而不是某个 `dyn` 句柄上的字符串字段。
    #[test]
    fn the_document_material_identity_comes_from_the_content_type() {
        let document = SceneBuilder::new("夹具")
            .object("body", sphere(), surface())
            .unwrap()
            .build()
            .expect("合法");
        assert_eq!(document.objects[0].material.shader.node, Surface::name());
        assert_eq!(Surface::name(), "surface");
    }

    /// 不透明的物体走**不透明**那一档；透明走**透明**那一档 ——
    /// 判据与 `frame::draws_of` 的 `select` 同一套谓词。
    #[test]
    fn the_stage_of_an_object_follows_its_alpha() {
        let mut material = Material::new(Member::new("shaders", "surface", &"0".repeat(64)));
        assert_eq!(stage_of(&material), Opaque::STAGE_NAME);
        material.alpha = AlphaMode::Add;
        assert_eq!(stage_of(&material), Transparent::STAGE_NAME);
    }

    /// 一个物体可以注册**多档**：登记里两档都在，而产物只落被画的那一档。
    #[test]
    fn one_object_can_register_several_stages() {
        let mut atmosphere = params(&[
            ("inner", Value::Num(1.0)),
            ("outer", Value::Num(1.14)),
            ("density", Value::Num(0.3)),
            ("softness", Value::Num(0.5)),
            ("tint", Value::Quad([0.0, 0.0, 0.0, 1.0])),
        ]);
        atmosphere.insert("density".to_string(), Value::Num(0.7));
        let registration = Registration::<Transparent, Atmosphere>::single(
            // 先给一份合法的，再给透明那一档一个变体。
            params(&[
                ("inner", Value::Num(1.0)),
                ("outer", Value::Num(1.14)),
                ("density", Value::Num(0.3)),
                ("softness", Value::Num(0.5)),
                ("tint", Value::Quad([0.0, 0.0, 0.0, 1.0])),
            ]),
        )
        .stage::<Transparent>(atmosphere);
        assert_eq!(
            registration.stages(),
            vec![Transparent::STAGE_NAME],
            "只点名了一档"
        );
        assert_eq!(
            registration.freeze(Transparent::STAGE_NAME)["density"],
            Value::Num(0.7)
        );
    }

    /// 环那一份内容只要一个 `tint` —— 表是按**内容类型**给的。
    #[test]
    fn a_minimal_content_needs_only_its_one_field() {
        let ring = Registration::<Transparent, Ring>::single(params(&[(
            "tint",
            Value::Quad([1.0, 1.0, 1.0, 1.0]),
        )]));
        SceneBuilder::new("夹具")
            .object("rings", sphere(), ring)
            .expect("环只要 tint");
    }
}
