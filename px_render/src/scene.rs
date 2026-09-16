//! **通用装配**：一份渲染文档 → 一堆实体。
//!
//! 这一篇里没有「行星」「云」「大气」—— 只有物体（几何 + 材质 + 变换）、灯、环境。
//! 「渲的是什么」全部由产物说了算（§65）：
//!
//! - 几何：CAS 里的 `Mesh` 产物，或者一个内建图元（球/细分球）；
//! - 材质：一份 WGSL 产物 + 按名字给的参数（**按它自己声明的结构体**打包，`crate::reflect`）
//!   + 按绑定下标给的贴图 + 混合/剔除两条渲染状态；
//! - 灯：点 / 聚 / 平行，位置、色、强度、射程、开不开影全是产物里的数；
//! - 环境：环境光强度 + 天空盒（cube 贴图产物）。

use std::path::Path;

use bevy::light::{NotShadowCaster, Skybox};
use bevy::prelude::*;
use bevy::shader::Shader;

use px_protocol::art::Camera;
use px_protocol::scene::{
    AlphaMode as DocAlphaMode, Geometry, Light, LightKind, Member, Sampler, SceneSpec,
};

use crate::art_cache::ArtCache;
use crate::material::{BoundTexture, DocMaterial};
use crate::reflect::ParamKind;
use crate::{mesh, reflect, slots};

/// 材质里一个**可以被窗口现场改**的整数参数。它记的是"哪一份材质、哪个名字、打在参数块的第几个字节"。
///
/// 渲染器仍然不认识这个参数是什么意思（云的消融档？水面的波浪档？都行）：
/// 它只是给窗口一条"照名字改那一格"的路。真正懂它的是那份 WGSL 与写场景的人。
#[derive(Debug, Clone)]
pub struct Instrument {
    pub material: Handle<DocMaterial>,
    pub param: String,
    pub offset: u32,
}

/// 一份渲染文档搭出来的东西。`label` 进 `Response.scene`，`ambient` 由相机组件用，
/// `skybox` / `cameras` 由每一步的相机摆放用。
pub struct DocumentScene {
    pub label: String,
    pub ambient: f32,
    pub skybox: Option<Handle<Image>>,
    pub skybox_brightness: f32,
    pub cameras: Vec<Camera>,
    /// 这一步往槽里装了**新的一版** WGSL（要等管线重编，见 `drive`）。
    pub installed_shaders: bool,
    /// 内容声明的期望标签里有 `clouds`（报告里"该有云"的判据，`ShotReport.declared_clouds`）。
    pub declared_clouds: bool,
    /// 这一步看的资产（槽里装的 shader 句柄）：等"资产装完"看的就是它们。
    pub watched: Vec<Handle<Shader>>,
    /// 可以现场改的仪器参数（窗口那一套键）。
    pub instruments: Vec<Instrument>,
    /// 这一份文档声明的 pass 表（空 = 没有这一节，只有主 pass）。
    pub passes: Option<std::sync::Arc<px_pass::Plan>>,
}

pub fn read_document(path: &str) -> Result<SceneSpec, String> {
    let spec = px_protocol::scene::read_scene(Path::new(path))?;
    spec.check()?;
    Ok(spec)
}

/// 装载一份 shader 之前先对账：产物烘的时候那份 **include 闭包**，跟**现在盘上**的闭包
/// 是不是同一份（§52.3）。
///
/// 为什么必须要这一条：`#import planet_x::…` 的真本住在 `assets/shaders/*.wgsl`，由 naga_oil
/// 在运行期组装。改了库、没重烘 ⇒ 场景指的还是老产物、而组装用的是新库 —— 画出来的东西
/// 既不是老那一版、也不是新那一版，**而键 / 清单 / 场景键 / 槽版本全都没动**：所有门都是绿的。
/// 所以在这里当场拒，并给出重烘配方；返回闭包摘要给日志（对上了也要说清对的是哪一份）。
fn closure_check(
    member: &Member,
    entry: &crate::art_cache::ShaderEntry,
    modules: &px_shader::ModuleTable,
) -> Result<String, String> {
    const REBAKE: &str = "重烘配方：cargo run -p px_graphs --bin shaders；再逐个 cargo run -p px_graphs --bin scene <名>";
    let current = px_shader::closure(&entry.source, modules);
    let now = current.fingerprint();
    match entry.closure {
        Some(recorded) if recorded == now => Ok(current.summary()),
        Some(recorded) => Err(format!(
            "shader 成员 {}/{} 的 include 闭包对不上：\n  \
             产物记的 {recorded:016x}｜盘上现在的 {now:016x}\n  \
             ⇒ 这份产物是拿另一版 include 烘的：现在画出来的既不是老那一版、也不是新那一版，\n    \
             而键 / 场景键 / 槽版本全没动（§52.3）—— 所以在这里拒，不静默出图。\n  {REBAKE}",
            member.graph, member.node,
        )),
        None => Err(format!(
            "shader 成员 {}/{} 的产物没有 include 闭包指纹（`px_shader/v1` 时代烘的）：\n  \
             那一版的键里少了「include」这一维，认它等于认错东西。\n  {REBAKE}",
            member.graph, member.node,
        )),
    }
}

/// 把文档里每一份材质要的 WGSL 装进槽里。**装之前**先问一句「槽里是不是已经是这一份」：
/// 装 = 新资产 = 管线重编（§52.3 那个"云静默消失"的坑）。
pub fn preload_shaders(
    server: &AssetServer,
    cache: &mut ArtCache,
    pcg_root: &Path,
    document: &SceneSpec,
) -> Result<(bool, Vec<Handle<Shader>>), String> {
    let mut installed = false;
    let mut watched = Vec::new();
    // 库表一个请求读一次就够（几万字节）；对账必须用**装载这一份时**盘上的库，
    // 所以不缓存到进程级：改了库的下一刻就该拦得住。
    let modules = px_shader::module_sources(&crate::shaders::shader_roots())
        .map_err(|err| format!("读 shader 库失败：{err}"))?;
    for object in &document.objects {
        let member: &Member = &object.material.shader;
        let path = member.resolve(pcg_root)?;
        let entry = cache.shader(&path.display().to_string())?;
        let closure = closure_check(member, &entry.value, &modules)?;
        let version = slots::version_of(&member.key)?;
        let fresh = slots::activate(server, slots::MATERIAL, version, &entry.value.source);
        println!(
            "shader 成员 {}/{} → {}：{} 字节（{}）｜版本 {:016x}｜{}",
            member.graph,
            member.node,
            slots::version_file(slots::MATERIAL, version),
            entry.value.source.len(),
            if entry.hit { "缓存命中" } else { "现读产物" },
            version,
            if fresh {
                "第一次见这一版，装进槽（管线要现编）"
            } else {
                "这一版已经在养，零动作（不 reload ⇒ 管线不重编）"
            }
        );
        println!("  {closure}");
        installed |= fresh;
        if let Some(handle) = slots::version_handle(slots::MATERIAL, version) {
            watched.push(handle);
        }
    }
    Ok((installed, watched))
}

/// 贴图产物的采样器由 `art_cache::load_texture` 按产物声明建（`TextureRef.sampler`）。

fn geometry_mesh(
    cache: &mut ArtCache,
    meshes: &mut Assets<Mesh>,
    pcg_root: &Path,
    geometry: &Geometry,
    id: &str,
) -> Result<Handle<Mesh>, String> {
    match geometry {
        Geometry::Mesh { member } => {
            let path = member.resolve(pcg_root)?;
            let ready = cache.mesh(&format!("{id}/{member}"), &path.display().to_string(), meshes)?;
            crate::art_cache::replay(&ready.value.audit, ready.hit);
            Ok(ready.value.handle.clone())
        }
        Geometry::Primitive { name, params } => {
            let mesh = mesh::primitive(name, params).map_err(|err| format!("物体 '{id}'：{err}"))?;
            Ok(meshes.add(mesh))
        }
    }
}

fn spawn_light(commands: &mut Commands, light: &Light) -> Result<(), String> {
    let color = Color::linear_rgb(light.color[0], light.color[1], light.color[2]);
    let position = Vec3::from_array(light.position);
    let direction = Vec3::from_array(light.direction);
    let transform = match light.kind {
        LightKind::Point => Transform::from_translation(position),
        LightKind::Spot | LightKind::Directional => {
            if direction.length_squared() < 1e-12 {
                return Err(format!(
                    "灯 '{}' 是 {:?}，但方向是零向量：它没有朝向可用",
                    light.id, light.kind
                ));
            }
            let direction = direction.normalize();
            // 方向与 up 共线时 `looking_to` 会退化 ⇒ 换一个 up（同一件事在两极上踩过，§29）。
            let up = if direction.dot(Vec3::Y).abs() > 0.999 {
                Vec3::Z
            } else {
                Vec3::Y
            };
            Transform::from_translation(position).looking_to(direction, up)
        }
    };

    match light.kind {
        LightKind::Point => {
            commands.spawn((
                crate::ScenePart,
                PointLight {
                    color,
                    intensity: light.intensity,
                    range: light.range.unwrap_or_else(|| PointLight::default().range),
                    shadow_maps_enabled: light.shadows,
                    ..default()
                },
                transform,
            ));
        }
        LightKind::Spot => {
            commands.spawn((
                crate::ScenePart,
                SpotLight {
                    color,
                    intensity: light.intensity,
                    range: light.range.unwrap_or_else(|| SpotLight::default().range),
                    inner_angle: light.inner_angle,
                    outer_angle: light.outer_angle,
                    shadow_maps_enabled: light.shadows,
                    ..default()
                },
                transform,
            ));
        }
        LightKind::Directional => {
            commands.spawn((
                crate::ScenePart,
                DirectionalLight {
                    color,
                    illuminance: light.intensity,
                    shadow_maps_enabled: light.shadows,
                    ..default()
                },
                transform,
            ));
        }
    }
    Ok(())
}

/// 一份渲染文档 → 搭好的场景。次序不能换：材质建起来的时候槽里就得是真本，
/// 否则这一帧画的是占位（洋红 = 没装上）。
#[allow(clippy::too_many_arguments)]
pub fn spawn_document(
    cache: &mut ArtCache,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<DocMaterial>,
    server: &AssetServer,
    pcg_root: &Path,
    scene_path: &str,
) -> Result<DocumentScene, String> {
    let document = read_document(scene_path)?;
    let (installed_shaders, watched) = preload_shaders(server, cache, pcg_root, &document)?;
    let (passes, pass_audit) = crate::passes::resolve(&document, cache, pcg_root)?;

    let skybox = match &document.environment.skybox {
        Some(member) => {
            let path = member.resolve(pcg_root)?;
            let ready = cache.texture(
                &format!("skybox/{member}"),
                &path.display().to_string(),
                &Sampler::clamped(),
                images,
            )?;
            crate::art_cache::replay(&ready.value.audit, ready.hit);
            Some(ready.value.image.clone())
        }
        None => None,
    };

    let mut instruments = Vec::new();
    let mut lines = vec![format!(
        "渲染文档 {}（v{}）｜物体 {} 个｜灯 {} 盏｜环境光 {}",
        document.name,
        document.schema,
        document.objects.len(),
        document.lights.len(),
        document.environment.ambient,
    )];
    lines.push(pass_audit);

    for light in &document.lights {
        spawn_light(commands, light)?;
        lines.push(format!(
            "  灯 {}：{:?} 位置 ({:.2},{:.2},{:.2}) 强度 {:.3e} 色 ({:.2},{:.2},{:.2}){}",
            light.id,
            light.kind,
            light.position[0],
            light.position[1],
            light.position[2],
            light.intensity,
            light.color[0],
            light.color[1],
            light.color[2],
            if light.shadows { "｜阴影贴图" } else { "" },
        ));
    }

    for object in &document.objects {
        let id = object.id.as_str();
        let handle = geometry_mesh(cache, meshes, pcg_root, &object.geometry, id)?;

        // 材质：先反射出这份 WGSL 的契约，再按它打包参数、按它校验贴图格。
        let shader_member = &object.material.shader;
        let shader_path = shader_member.resolve(pcg_root)?;
        let shader_entry = cache.shader(&shader_path.display().to_string())?;
        let version = slots::version_of(&shader_member.key)?;
        let layout = reflect::layout_of(version, &shader_member.node, &shader_entry.value.source)?;
        let params = layout
            .pack(&object.material.params)
            .map_err(|err| format!("物体 '{id}' 的材质：{err}"))?;

        let mut textures: Vec<BoundTexture> = Vec::new();
        for (role, texture) in &object.material.textures {
            let Some(slot) = layout.texture(texture.binding) else {
                return Err(format!(
                    "物体 '{id}' 的贴图 '{role}' 要绑在第 {} 格，但这份 shader 没在那里声明贴图；\
                     它声明了：{}",
                    texture.binding,
                    if layout.textures.is_empty() {
                        "（一张都没有）".to_string()
                    } else {
                        layout
                            .textures
                            .iter()
                            .map(|slot| {
                                format!("{} 格 {}", slot.binding, slot.dimension.name())
                            })
                            .collect::<Vec<_>>()
                            .join(" / ")
                    }
                ));
            };
            let path = texture.member.resolve(pcg_root)?;
            let ready = cache.texture(
                &format!("{id}/{role}"),
                &path.display().to_string(),
                &texture.sampler,
                images,
            )?;
            crate::art_cache::replay(&ready.value.audit, ready.hit);
            if ready.value.layers != slot.dimension.layers() {
                return Err(format!(
                    "物体 '{id}' 的贴图 '{role}' 是 {} 层，而 shader 第 {} 格声明的是 {}（{} 层）",
                    ready.value.layers,
                    texture.binding,
                    slot.dimension.name(),
                    slot.dimension.layers()
                ));
            }
            textures.push(BoundTexture {
                binding: texture.binding,
                image: ready.value.image.clone(),
            });
        }
        for slot in &layout.textures {
            if !textures.iter().any(|bound| bound.binding == slot.binding) {
                println!(
                    "⚠ 物体 '{id}'：shader 第 {} 格声明了 {}，产物没给那张贴图 ⇒ 这一格绑兜底图\
                     （shader 里若真去采它，采到的是纯白）",
                    slot.binding,
                    slot.dimension.name()
                );
            }
        }

        let material = materials.add(DocMaterial {
            params,
            textures,
            alpha: match object.material.alpha {
                DocAlphaMode::Opaque => AlphaMode::Opaque,
                DocAlphaMode::Premultiplied => AlphaMode::Premultiplied,
                DocAlphaMode::Blend => AlphaMode::Blend,
                DocAlphaMode::Add => AlphaMode::Add,
            },
            cull: object.material.cull,
            depth_bias: object.material.depth_bias,
            shader: version,
        });

        let transform = Transform {
            translation: Vec3::from_array(object.transform.translation),
            // ⚠ 只在**明显**不是单位四元数时才归一化：`normalize()` 对"本来就是单位"的四元数
            // 也会动最后一位（`sqrt(1.0)` 未必正好是 1.0），而那一位之差会被矩阵合成放大成
            // 亚像素抖动 —— 出图对账时表现为"个别像素差 1~2"，看起来像渲染变了。
            // 产物里写的是单位四元数（烘图侧算的），所以正常路径一个字节都不动。
            rotation: unit_or_fix(Quat::from_array(object.transform.rotation)),
            scale: Vec3::from_array(object.transform.scale),
        };
        let entity = commands
            .spawn((
                crate::ScenePart,
                Mesh3d(handle.clone()),
                MeshMaterial3d(material.clone()),
                transform,
            ))
            .id();
        if !object.cast_shadow {
            commands.entity(entity).insert(NotShadowCaster);
        }

        for slot in &layout.params {
            if matches!(slot.kind, ParamKind::U32 | ParamKind::I32) {
                instruments.push(Instrument {
                    material: material.clone(),
                    param: slot.name.clone(),
                    offset: slot.offset,
                });
            }
        }
        lines.push(format!(
            "  物体 {id}：{}｜shader {}/{}（版本 {:016x}）｜参数 {} 个｜贴图 {} 张",
            match &object.geometry {
                Geometry::Mesh { member } => format!("网格 {member}"),
                Geometry::Primitive { name, .. } => format!("图元 {name}"),
            },
            shader_member.graph,
            shader_member.node,
            version,
            object.material.params.len(),
            object.material.textures.len(),
        ));
    }

    let assets = match &document.environment.skybox {
        Some(member) => member.to_string(),
        None => "无".to_string(),
    };
    lines.push(format!(
        "  环境：环境光 {}｜天空盒 {assets}（亮度 {}）",
        document.environment.ambient, document.environment.skybox_brightness
    ));

    Ok(DocumentScene {
        label: lines.join("\n"),
        ambient: document.environment.ambient,
        skybox,
        skybox_brightness: document.environment.skybox_brightness,
        cameras: document.cameras.clone(),
        installed_shaders,
        declared_clouds: document.expects.iter().any(|tag| tag == "clouds"),
        watched,
        instruments,
        passes,
    })
}

/// 见 `spawn_document` 里那条注释：够接近单位就不动它，差太多才修（产物写坏了要看得出来）。
fn unit_or_fix(rotation: Quat) -> Quat {
    let length = rotation.length();
    if (length - 1.0).abs() <= 1e-4 {
        rotation
    } else {
        println!("⚠ 产物里的旋转四元数长度是 {length}（应当接近 1）⇒ 归一化它");
        rotation.normalize()
    }
}

/// 相机：产物给的是**世界系**里的方向与距离（以行星半径 1 为单位）。
/// 渲染器只负责把它摆成"看着原点"的 `Transform`，不认识任何倾斜常数 —— 倾斜是内容
/// （烘图侧把 `SYSTEM_TILT` 乘进方向里），渲染器里再留一份就是第二个会漂开的默认值。
///
/// ⚠️ 方向与 `Vec3::Y` 共线时 `looking_at` 的 up 会退化 —— `px_ops::cameras::review()`
/// 因此把两极视角停在 82° 而不是 90°。
pub fn camera_for(camera: &Camera) -> Transform {
    let direction = Vec3::from_array(camera.direction);
    let direction = if direction.length_squared() > 1e-12 {
        direction.normalize()
    } else {
        Vec3::Z
    };
    Transform::from_translation(direction * camera.distance.max(1e-3)).looking_at(Vec3::ZERO, Vec3::Y)
}

/// 天空盒组件（相机上）。没有天空盒产物就不挂它 —— 背景就是清屏色。
pub fn skybox_of(scene: &DocumentScene) -> Option<Skybox> {
    scene.skybox.as_ref().map(|image| Skybox {
        image: Some(image.clone()),
        brightness: scene.skybox_brightness,
        rotation: Quat::IDENTITY,
    })
}

/// `--cam yaw,pitch,dist`：**世界系**的方位角 / 仰角 / 距离。没给就是那条固定视角。
/// 与相机的产物表无关（那是"这一步从哪儿看"，不是内容）。
pub fn probe_camera(cam: Option<[f32; 3]>) -> Transform {
    let Some([yaw, pitch, distance]) = cam else {
        return Transform::from_xyz(0.0, 0.55, 3.15).looking_at(Vec3::ZERO, Vec3::Y);
    };
    let yaw = yaw.to_radians();
    let pitch = pitch.clamp(-89.5, 89.5).to_radians();
    let direction = Vec3::new(
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        pitch.cos() * yaw.cos(),
    );
    Transform::from_translation(direction * distance).looking_at(Vec3::ZERO, Vec3::Y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art_cache::ShaderEntry;

    /// 一份入口 + 它 import 的那个模块。入口文本一个字没动也能测出三种情形。
    const ENTRY: &str = "#import planet_x::noise::fbm_3\nfn f() -> f32 { fbm_3() }\n";
    const LIBRARY: &str = "#define_import_path planet_x::noise\nfn fbm_3() -> f32 { 1.0 }\n";

    fn modules(source: &str) -> px_shader::ModuleTable {
        [("planet_x::noise".to_string(), source.to_string())]
            .into_iter()
            .collect()
    }

    fn member() -> Member {
        Member::new("shaders", "surface", &"0".repeat(64))
    }

    fn entry(closure: Option<u64>) -> ShaderEntry {
        ShaderEntry {
            source: ENTRY.to_string(),
            closure,
        }
    }

    #[test]
    fn the_gate_passes_when_the_artifact_was_baked_with_this_library() {
        let table = modules(LIBRARY);
        let recorded = px_shader::closure(ENTRY, &table).fingerprint();
        let summary = closure_check(&member(), &entry(Some(recorded)), &table)
            .expect("同一份闭包应当放行");
        assert!(
            summary.contains("include 闭包"),
            "放行时也要说清对的是哪一份：{summary}"
        );
    }

    #[test]
    fn the_gate_refuses_an_artifact_baked_with_another_library() {
        let table = modules(LIBRARY);
        let recorded = px_shader::closure(ENTRY, &modules("别的库")).fingerprint();
        let refusal = closure_check(&member(), &entry(Some(recorded)), &table)
            .expect_err("另一版 include 烘的产物必须当场拒");
        assert!(refusal.contains("shaders/surface"), "要点名是谁：{refusal}");
        assert!(refusal.contains("include 闭包对不上"), "{refusal}");
        assert!(refusal.contains("重烘配方"), "拒了要给重烘配方：{refusal}");
    }

    #[test]
    fn the_gate_refuses_an_artifact_that_never_recorded_a_closure() {
        let table = modules(LIBRARY);
        let refusal = closure_check(&member(), &entry(None), &table)
            .expect_err("px_shader/v1 时代的产物必须当场拒");
        assert!(refusal.contains("没有 include 闭包指纹"), "{refusal}");
        assert!(refusal.contains("重烘配方"), "{refusal}");
    }
}
