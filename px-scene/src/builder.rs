use std::collections::BTreeMap;
use std::path::Path;

use px_protocol::scene::{
    AlphaMode, CullMode, Environment, Geometry, Light, Material, Member, Object, SceneSpec,
    TextureRef, Transform, Value,
};

use crate::contract::{merge_named, schema_of};
use crate::frame::{self, Sources};
use crate::stage::{self, CheckStages, Content, Registration, Stage};

#[derive(Debug, Clone)]
pub struct SceneBuilder {
    name: String,
    environment: Environment,
    cameras: Vec<px_protocol::art::Camera>,
    expects: Vec<String>,
    lights: Vec<Light>,
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

    pub fn ambient(mut self, ambient: f32) -> Self {
        self.environment.ambient = ambient;
        self
    }

    pub fn skybox(mut self, member: Member, brightness: f32) -> Self {
        self.environment.skybox = Some(member);
        self.environment.skybox_brightness = brightness;
        self
    }

    pub fn cameras(mut self, cameras: Vec<px_protocol::art::Camera>) -> Self {
        self.cameras = cameras;
        self
    }

    pub fn camera(mut self, camera: px_protocol::art::Camera) -> Self {
        self.cameras.push(camera);
        self
    }

    pub fn expect(mut self, label: &str) -> Self {
        self.expects.push(label.to_string());
        self
    }

    pub fn add_light(mut self, light: Light) -> Self {
        self.lights.push(light);
        self
    }

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
            shadow_density: 0.0,
        };
        self.add(registration, object)
    }

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
            shadow: None,
            objects: self.objects,
            frame_materials: Vec::new(),
            material_instances: Vec::new(),
        }
    }

    pub fn build(self) -> Result<SceneSpec, String> {
        if self.objects.is_empty() {
            return Err(format!("场景 '{}' 一个物体都没有", self.name));
        }
        let document = self.document();
        document.check()?;
        Ok(document)
    }

    pub fn bake(self, frame: &frame::FrameFile, sources: &Sources) -> Result<SceneSpec, String> {
        let mut document = self.build()?;
        let baked = frame::build(frame, &document.objects, sources, &document.lights, true)?;
        document.resources = baked.resources;
        document.passes = baked.passes;
        document.frame_materials = baked.materials;
        document.material_instances = baked.material_instances;
        document.check()?;
        Ok(document)
    }

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

pub fn stage_of(material: &Material) -> &'static str {
    if material.alpha == AlphaMode::Opaque {
        stage::Opaque::STAGE_NAME
    } else {
        stage::Transparent::STAGE_NAME
    }
}

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

    pub fn num(mut self, name: &str, value: f64) -> Self {
        self.given
            .insert(name.to_string(), toml::Value::Float(value));
        self
    }

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

    pub fn build(self, root: &Path) -> Result<Material, String> {
        let layout = schema_of(&self.shader, root)?;
        let params = merge_named(
            &format!("材质 '{}'", self.shader.node),
            &self.given,
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

    #[test]
    fn an_empty_id_is_refused() {
        let err = SceneBuilder::new("夹具")
            .object("   ", sphere(), surface())
            .expect_err("空 id");
        assert!(err.contains("没给 id"), "{err}");
    }

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

    #[test]
    fn an_empty_scene_is_refused() {
        let err = SceneBuilder::new("夹具")
            .build()
            .expect_err("一个物体都没有");
        assert!(err.contains("一个物体都没有"), "{err}");
    }

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

    #[test]
    fn the_stage_of_an_object_follows_its_alpha() {
        let mut material = Material::new(Member::new("shaders", "surface", &"0".repeat(64)));
        assert_eq!(stage_of(&material), Opaque::STAGE_NAME);
        material.alpha = AlphaMode::Add;
        assert_eq!(stage_of(&material), Transparent::STAGE_NAME);
    }

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
        let registration = Registration::<Transparent, Atmosphere>::single(params(&[
            ("inner", Value::Num(1.0)),
            ("outer", Value::Num(1.14)),
            ("density", Value::Num(0.3)),
            ("softness", Value::Num(0.5)),
            ("tint", Value::Quad([0.0, 0.0, 0.0, 1.0])),
        ]))
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
