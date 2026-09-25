use bytemuck::{Pod, Zeroable};
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::mat4::{Mat4, Vec4};

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ViewUniform {
    pub world_position: [f32; 3],
    pub exposure: f32,
    pub view_from_world: [[f32; 4]; 4],
    pub clip_from_view: [[f32; 4]; 4],
    pub viewport: [f32; 4],
    pub view_from_clip: [[f32; 4]; 4],
    pub world_from_view: [[f32; 4]; 4],
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LightsUniform {
    pub ambient_color: [f32; 4],
    pub n_point_lights: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GlobalsUniform {
    pub time: f32,
    pub delta_time: f32,
    pub frame_count: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ClusteredLight {
    pub light_custom_data: [f32; 4],
    pub color_inverse_square_range: [f32; 4],
    pub position_radius: [f32; 4],
    pub flags: u32,
    pub shadow_depth_bias: f32,
    pub shadow_normal_bias: f32,
    pub spot_light_tan_angle: f32,
    pub soft_shadow_size: f32,
    pub shadow_map_near_z: f32,
    pub decal_index: u32,
    pub range: f32,
}

pub const VIEW_BINDING: (u32, u32) = (0, 0);
pub const LIGHTS_BINDING: (u32, u32) = (0, 1);
pub const CLUSTERED_LIGHTS_BINDING: (u32, u32) = (0, 8);
pub const GLOBALS_BINDING: (u32, u32) = (0, 11);
pub const DEPTH_PREPASS_BINDING: (u32, u32) = (0, 20);

pub const MESH_INSTANCES_BINDING: (u32, u32) = (0, 21);

pub const SHADOW_PAGE_OFFSETS_BINDING: (u32, u32) = (0, 5);

pub const SHADOW_FACES_BINDING: (u32, u32) = (0, 6);

pub fn exposure() -> f32 {
    f32::exp2(-9.7) / 1.2
}

pub fn to_bytes<T: Pod>(value: &T) -> Vec<u8> {
    bytemuck::bytes_of(value).to_vec()
}

pub fn to_uniform_bytes<T: Pod>(value: &T) -> Vec<u8> {
    let mut bytes = to_bytes(value);
    bytes.resize(bytes.len().div_ceil(16) * 16, 0);
    bytes
}

impl ViewUniform {
    pub fn new(
        world_position: [f32; 3],
        view_from_world: [[f32; 4]; 4],
        clip_from_view: [[f32; 4]; 4],
        viewport: [f32; 4],
        view_from_clip: [[f32; 4]; 4],
        world_from_view: [[f32; 4]; 4],
    ) -> ViewUniform {
        ViewUniform {
            world_position,
            exposure: exposure(),
            view_from_world,
            clip_from_view,
            viewport,
            view_from_clip,
            world_from_view,
        }
    }

    pub fn from_camera(camera: &crate::camera::Camera, viewport: [f32; 4]) -> ViewUniform {
        ViewUniform::new(
            [camera.position.x, camera.position.y, camera.position.z],
            columns(&camera.view_from_world),
            columns(&camera.clip_from_view),
            viewport,
            columns(&camera.view_from_clip),
            columns(&camera.world_from_view),
        )
    }
}

fn columns(matrix: &Mat4) -> [[f32; 4]; 4] {
    let axis = |v: Vec4| [v.x, v.y, v.z, v.w];
    [
        axis(matrix.x_axis),
        axis(matrix.y_axis),
        axis(matrix.z_axis),
        axis(matrix.w_axis),
    ]
}

impl LightsUniform {
    pub fn ambient(brightness: f32) -> LightsUniform {
        LightsUniform {
            ambient_color: [brightness; 4],
            n_point_lights: [0; 4],
        }
    }

    pub fn with_light_count(mut self, count: u32) -> LightsUniform {
        self.n_point_lights[0] = count;
        self
    }
}

impl ClusteredLight {
    pub fn absent() -> ClusteredLight {
        ClusteredLight::zeroed()
    }
}

pub const POINT_LIGHT_DEFAULT_RANGE: f32 = 20.0;

pub const POINT_LIGHT_RADIUS: f32 = 0.0;

pub const POINT_LIGHT_SHADOW_DEPTH_BIAS: f32 = 0.08;

pub const POINT_LIGHT_SHADOW_NORMAL_BIAS: f32 = 0.6;

pub const POINT_LIGHT_SHADOW_MAP_NEAR_Z: f32 = 0.1;

pub const POINT_LIGHT_SHADOW_MAP_SIZE: u32 = 1024;

pub const POINT_LIGHT_FLAGS_SHADOWS_ENABLED: u32 = 1 << 0;

pub const POINT_LIGHT_FLAGS_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE: u32 = 1 << 3;

pub fn shadow_normal_bias() -> f32 {
    POINT_LIGHT_SHADOW_NORMAL_BIAS * core::f32::consts::SQRT_2
}

pub fn light_of(light: &px_protocol::scene::Light) -> Result<ClusteredLight, String> {
    if light.kind != px_protocol::scene::LightKind::Point {
        return Err(format!(
            "灯 '{}' 是 {:?}，而这一档只兑现**点光源**：聚光要 direction / 内外角/\
             `spot_light_tan_angle`，平行光在 oracle 里根本不住 `clustered_lights`\
             （它住 `Lights` 那份 uniform）—— 塞进这一格就是画出来不对而没人报错",
            light.id, light.kind
        ));
    }
    let range = light.range.unwrap_or(POINT_LIGHT_DEFAULT_RANGE);
    let intensity = light.intensity / (4.0 * core::f32::consts::PI);
    let face = Mat4::perspective_infinite_reverse_rh(
        core::f32::consts::FRAC_PI_2,
        1.0,
        POINT_LIGHT_SHADOW_MAP_NEAR_Z,
    );
    Ok(ClusteredLight {
        light_custom_data: [face.z_axis.z, face.z_axis.w, face.w_axis.z, face.w_axis.w],
        color_inverse_square_range: [
            light.color[0] * intensity,
            light.color[1] * intensity,
            light.color[2] * intensity,
            1.0 / (range * range),
        ],
        position_radius: [
            light.position[0],
            light.position[1],
            light.position[2],
            POINT_LIGHT_RADIUS,
        ],
        flags: POINT_LIGHT_FLAGS_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE
            | if light.shadows {
                POINT_LIGHT_FLAGS_SHADOWS_ENABLED
            } else {
                0
            },
        shadow_depth_bias: if std::env::var_os("PX_NO_SHADOW_BIAS").is_some() {
            0.0
        } else {
            POINT_LIGHT_SHADOW_DEPTH_BIAS
        },
        shadow_normal_bias: if std::env::var_os("PX_NO_SHADOW_BIAS").is_some() {
            0.0
        } else {
            shadow_normal_bias()
        },
        spot_light_tan_angle: 0.0,
        soft_shadow_size: 0.0,
        shadow_map_near_z: POINT_LIGHT_SHADOW_MAP_NEAR_Z,
        decal_index: u32::MAX,
        range,
    })
}

pub fn lights_of(lights: &[px_protocol::scene::Light]) -> Result<Vec<ClusteredLight>, String> {
    let mut packed: Vec<ClusteredLight> = Vec::with_capacity(lights.len());
    for light in lights {
        packed.push(light_of(light)?);
    }
    packed.sort_by_key(|light| {
        if light.flags & POINT_LIGHT_FLAGS_SHADOWS_ENABLED != 0 {
            0
        } else {
            1
        }
    });
    Ok(packed)
}

pub fn globals_zero() -> GlobalsUniform {
    GlobalsUniform::default()
}

pub const POINT_SHADOW_TEXTURES_BINDING: (u32, u32) = (0, 2);
pub const POINT_SHADOW_TEXTURES_L_BINDINGS: [(u32, u32); 3] = [(0, 22), (0, 23), (0, 24)];

pub const POINT_SHADOW_SAMPLER_BINDING: (u32, u32) = (0, 3);

pub const SHADOW_PAGE_TABLE_BINDING: (u32, u32) = (0, 4);

pub const SHADOW_CUBE_FACES: u32 = 6;

pub fn fallback_cube(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("组 0：兜底 atlas（1×1×1，全 0）"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("组 0：兜底 atlas（D2Array/DepthOnly）"),
        format: None,
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: None,
        base_array_layer: 0,
        array_layer_count: Some(1),
    });
    (texture, view)
}

pub fn point_shadow_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("组 0：点光影子比较采样器"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        compare: Some(wgpu::CompareFunction::GreaterEqual),
        ..Default::default()
    })
}

pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let buffer = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("组 0（七格超集）"),
        entries: &[
            buffer(VIEW_BINDING.1, wgpu::BufferBindingType::Uniform),
            buffer(LIGHTS_BINDING.1, wgpu::BufferBindingType::Uniform),
            wgpu::BindGroupLayoutEntry {
                binding: POINT_SHADOW_TEXTURES_BINDING.1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: POINT_SHADOW_SAMPLER_BINDING.1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: POINT_SHADOW_TEXTURES_L_BINDINGS[0].1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: POINT_SHADOW_TEXTURES_L_BINDINGS[1].1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: POINT_SHADOW_TEXTURES_L_BINDINGS[2].1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            buffer(
                SHADOW_PAGE_TABLE_BINDING.1,
                wgpu::BufferBindingType::Storage { read_only: true },
            ),
            buffer(
                MESH_INSTANCES_BINDING.1,
                wgpu::BufferBindingType::Storage { read_only: true },
            ),
            buffer(
                SHADOW_PAGE_OFFSETS_BINDING.1,
                wgpu::BufferBindingType::Storage { read_only: true },
            ),
            buffer(
                SHADOW_FACES_BINDING.1,
                wgpu::BufferBindingType::Storage { read_only: true },
            ),
            buffer(
                CLUSTERED_LIGHTS_BINDING.1,
                wgpu::BufferBindingType::Storage { read_only: true },
            ),
            buffer(GLOBALS_BINDING.1, wgpu::BufferBindingType::Uniform),
            wgpu::BindGroupLayoutEntry {
                binding: DEPTH_PREPASS_BINDING.1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

pub struct GroupZero {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    pub view_buffer: wgpu::Buffer,
    pub audit: Vec<String>,
}

impl GroupZero {
    pub fn set_view(
        &self,
        queue: &wgpu::Queue,
        camera: &crate::camera::Camera,
        viewport: [f32; 4],
    ) -> String {
        let view = ViewUniform::from_camera(camera, viewport);
        queue.write_buffer(&self.view_buffer, 0, &to_uniform_bytes(&view));
        view_line(&view, camera)
    }
}

fn view_line(view: &ViewUniform, camera: &crate::camera::Camera) -> String {
    format!(
        "view：world_position ({:.3}, {:.3}, {:.3})｜exposure {:.9e}（位模式 {:08X}）｜viewport ({}, {}, {}, {})",
        camera.position.x,
        camera.position.y,
        camera.position.z,
        view.exposure,
        view.exposure.to_bits(),
        view.viewport[0],
        view.viewport[1],
        view.viewport[2],
        view.viewport[3]
    )
}

pub fn frame(
    device: &wgpu::Device,
    module: &naga::Module,
    camera: &crate::camera::Camera,
    ambient: f32,
    cluster: &[ClusteredLight],
    viewport: [f32; 4],
    depth: &wgpu::TextureView,
    shadow_cubes: &[&wgpu::TextureView],
    shadow_sampler: &wgpu::Sampler,
    shadow_page_table: &wgpu::Buffer,
    mesh_instances: &wgpu::Buffer,
    shadow_page_offsets: &wgpu::Buffer,
    shadow_faces: &wgpu::Buffer,
    shadow_note: &str,
) -> Result<GroupZero, String> {
    let (group, binding) = CLUSTERED_LIGHTS_BINDING;
    let array = storage_array_layout(module, group, binding)
        .map_err(|err| format!("反射聚类缓冲失败：{err}"))?;
    let light = std::mem::size_of::<ClusteredLight>();
    if array.stride as usize != light {
        return Err(format!(
            "聚类数组的步长是 {}，而 `ClusteredLight` 是 {light} 字节：同一份契约的两个数",
            array.stride
        ));
    }
    if cluster.len() > array.count as usize {
        return Err(format!(
            "文档里有 {} 盏灯，而聚类缓冲只有 {} 格（长度写在 shader 的 \
             `clustered_lights.data` 里）：装不下的那 {} 盏会被静默丢掉",
            cluster.len(),
            array.count,
            cluster.len() - array.count as usize
        ));
    }

    let view = ViewUniform::from_camera(camera, viewport);
    let lights = LightsUniform::ambient(ambient).with_light_count(cluster.len() as u32);
    let globals = globals_zero();
    let mut bytes: Vec<u8> = Vec::with_capacity(array.size as usize);
    for light in cluster {
        bytes.extend_from_slice(&to_bytes(light));
    }
    bytes.resize(array.size as usize, 0);

    let uniform = |label: &str, bytes: &[u8]| {
        device.create_buffer_init(&BufferInitDescriptor {
            label: Some(label),
            usage: wgpu::BufferUsages::UNIFORM,
            contents: bytes,
        })
    };
    let view_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("组 0：view（每帧只写它）"),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        contents: &to_uniform_bytes(&view),
    });
    let lights_buffer = uniform("组 0：lights", &to_uniform_bytes(&lights));
    let globals_buffer = uniform("组 0：globals", &to_uniform_bytes(&globals));
    let cluster_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("组 0：clustered_lights"),
        usage: wgpu::BufferUsages::STORAGE,
        contents: &bytes,
    });

    let layout = bind_group_layout(device);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("组 0（七格超集）"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: VIEW_BINDING.1,
                resource: view_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: LIGHTS_BINDING.1,
                resource: lights_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: POINT_SHADOW_TEXTURES_BINDING.1,
                resource: wgpu::BindingResource::TextureView(shadow_cubes[0]),
            },
            wgpu::BindGroupEntry {
                binding: POINT_SHADOW_SAMPLER_BINDING.1,
                resource: wgpu::BindingResource::Sampler(shadow_sampler),
            },
            wgpu::BindGroupEntry {
                binding: POINT_SHADOW_TEXTURES_L_BINDINGS[0].1,
                resource: wgpu::BindingResource::TextureView(shadow_cubes[1]),
            },
            wgpu::BindGroupEntry {
                binding: POINT_SHADOW_TEXTURES_L_BINDINGS[1].1,
                resource: wgpu::BindingResource::TextureView(shadow_cubes[2]),
            },
            wgpu::BindGroupEntry {
                binding: POINT_SHADOW_TEXTURES_L_BINDINGS[2].1,
                resource: wgpu::BindingResource::TextureView(shadow_cubes[3]),
            },
            wgpu::BindGroupEntry {
                binding: SHADOW_PAGE_TABLE_BINDING.1,
                resource: shadow_page_table.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: MESH_INSTANCES_BINDING.1,
                resource: mesh_instances.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: SHADOW_PAGE_OFFSETS_BINDING.1,
                resource: shadow_page_offsets.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: SHADOW_FACES_BINDING.1,
                resource: shadow_faces.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: CLUSTERED_LIGHTS_BINDING.1,
                resource: cluster_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: GLOBALS_BINDING.1,
                resource: globals_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: DEPTH_PREPASS_BINDING.1,
                resource: wgpu::BindingResource::TextureView(depth),
            },
        ],
    });
    let mut audit = vec![
        format!(
            "lights：ambient_color ({}, {}, {}, {})",
            lights.ambient_color[0],
            lights.ambient_color[1],
            lights.ambient_color[2],
            lights.ambient_color[3]
        ),
        format!(
            "globals：time {}｜delta_time {}｜frame_count {}",
            globals.time, globals.delta_time, globals.frame_count
        ),
        format!(
            "clustered_lights：{} 格 × {} 字节 = {} 字节；文档 {} 盏灯写进前 {} 格，其余**全零**\
                 （零 = 这盏灯不存在，内容 shader 自己判）",
            array.count,
            array.stride,
            array.size,
            cluster.len(),
            cluster.len()
        ),
        format!(
            "point_shadow_textures（group 0 binding {}）：**D2Array**（每面一层，层号 = 灯×6+面），\
                 {shadow_note}｜sampler（binding {}）：ClampToEdge×3 / Linear / Linear / Nearest / \
                 lod [0, 32] / GreaterEqual（虚拟影图改手动比较之后这一格不再被采）｜\
                 页表（binding {}）：{} 个 u32",
            POINT_SHADOW_TEXTURES_BINDING.1,
            POINT_SHADOW_SAMPLER_BINDING.1,
            SHADOW_PAGE_TABLE_BINDING.1,
            shadow_page_table.size() / 4
        ),
    ];
    for (index, light) in cluster.iter().enumerate() {
        audit.push(format!(
            "  data[{index}]：color×强度/4π ({:.6e}, {:.6e}, {:.6e})｜1/range² {:.6e}｜位置 ({:.3}, {:.3}, {:.3})｜radius {}｜flags {}（影子 {}）｜depth_bias {}｜normal_bias {:.9e}｜near_z {}｜custom_data ({}, {}, {}, {})｜decal_index {}｜range {}",
            light.color_inverse_square_range[0],
            light.color_inverse_square_range[1],
            light.color_inverse_square_range[2],
            light.color_inverse_square_range[3],
            light.position_radius[0],
            light.position_radius[1],
            light.position_radius[2],
            light.position_radius[3],
            light.flags,
            if light.flags & POINT_LIGHT_FLAGS_SHADOWS_ENABLED != 0 {
                "开"
            } else {
                "关"
            },
            light.shadow_depth_bias,
            light.shadow_normal_bias,
            light.shadow_map_near_z,
            light.light_custom_data[0],
            light.light_custom_data[1],
            light.light_custom_data[2],
            light.light_custom_data[3],
            light.decal_index,
            light.range
        ));
    }
    Ok(GroupZero {
        layout,
        bind_group,
        view_buffer,
        audit,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructLayout {
    pub name: String,
    pub size: u32,
    pub align: u32,
    pub members: Vec<(String, u32, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrayLayout {
    pub name: String,
    pub stride: u32,
    pub count: u32,
    pub size: u32,
}

pub fn struct_layout(
    module: &naga::Module,
    group: u32,
    binding: u32,
) -> Result<StructLayout, String> {
    let (global, handle) = global_at(module, group, binding)?;
    let name = global.name.clone().unwrap_or_else(|| "?".to_string());
    struct_layout_of(module, handle, &name)
}

pub fn element_struct_layout(
    module: &naga::Module,
    group: u32,
    binding: u32,
) -> Result<StructLayout, String> {
    let (global, handle) = global_at(module, group, binding)?;
    let outer = global.name.clone().unwrap_or_else(|| "?".to_string());
    let naga::TypeInner::Struct { members, .. } = &module.types[handle].inner else {
        return Err(format!(
            "@group({group}) @binding({binding}) 的 {outer} 不是结构体：{:?}",
            module.types[handle].inner
        ));
    };
    let member = members
        .first()
        .ok_or_else(|| format!("{outer} 是个空结构体：里面没有数组"))?;
    let naga::TypeInner::Array { base, .. } = &module.types[member.ty].inner else {
        return Err(format!(
            "{outer}.{} 不是数组：{:?}",
            member.name.clone().unwrap_or_else(|| "?".to_string()),
            module.types[member.ty].inner
        ));
    };
    let name = module.types[*base]
        .name
        .clone()
        .unwrap_or_else(|| format!("{} 的元素", outer));
    struct_layout_of(module, *base, &name)
}

fn struct_layout_of(
    module: &naga::Module,
    handle: naga::Handle<naga::Type>,
    name: &str,
) -> Result<StructLayout, String> {
    let naga::TypeInner::Struct { members, span } = &module.types[handle].inner else {
        return Err(format!(
            "{name} 不是结构体：{:?}",
            module.types[handle].inner
        ));
    };
    let layout = layouter(module)?[handle];
    Ok(StructLayout {
        name: name.to_string(),
        size: *span,
        align: layout.alignment.round_up(1),
        members: members
            .iter()
            .map(|member| {
                (
                    member.name.clone().unwrap_or_else(|| "?".to_string()),
                    member.offset,
                    type_name(module, member.ty),
                )
            })
            .collect(),
    })
}

pub fn storage_array_layout(
    module: &naga::Module,
    group: u32,
    binding: u32,
) -> Result<ArrayLayout, String> {
    let (global, handle) = global_at(module, group, binding)?;
    let name = global.name.clone().unwrap_or_else(|| "?".to_string());
    let naga::TypeInner::Struct { members, .. } = &module.types[handle].inner else {
        return Err(format!(
            "@group({group}) @binding({binding}) 的 {name} 不是结构体：{:?}",
            module.types[handle].inner
        ));
    };
    let member = members
        .first()
        .ok_or_else(|| format!("{name} 是个空结构体：里面没有数组"))?;
    let element = member.name.clone().unwrap_or_else(|| "?".to_string());
    let naga::TypeInner::Array {
        stride,
        size: naga::ArraySize::Constant(count),
        ..
    } = &module.types[member.ty].inner
    else {
        return Err(format!(
            "{name}.{element} 不是定长数组：{:?}",
            module.types[member.ty].inner
        ));
    };
    let count = count.get();
    Ok(ArrayLayout {
        name: format!("{name}.{element}"),
        stride: *stride,
        count,
        size: *stride * count,
    })
}

fn global_at(
    module: &naga::Module,
    group: u32,
    binding: u32,
) -> Result<(&naga::GlobalVariable, naga::Handle<naga::Type>), String> {
    for (_, global) in module.global_variables.iter() {
        if global.binding == Some(naga::ResourceBinding { group, binding }) {
            return Ok((global, global.ty));
        }
    }
    Err(format!(
        "组装后的 WGSL 里没有 @group({group}) @binding({binding}) 这个全局变量"
    ))
}

fn layouter(module: &naga::Module) -> Result<naga::proc::Layouter, String> {
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(naga::proc::GlobalCtx {
            types: &module.types,
            constants: &module.constants,
            overrides: &module.overrides,
            global_expressions: &module.global_expressions,
        })
        .map_err(|err| format!("算不出类型布局：{err}"))?;
    Ok(layouter)
}

fn type_name(module: &naga::Module, handle: naga::Handle<naga::Type>) -> String {
    use naga::{ArraySize, TypeInner};
    match &module.types[handle].inner {
        TypeInner::Scalar(scalar) => scalar_name(scalar),
        TypeInner::Vector { size, scalar } => {
            format!("vec{}<{}>", dimension(*size), scalar_name(scalar))
        }
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => format!(
            "mat{}x{}<{}>",
            dimension(*columns),
            dimension(*rows),
            scalar_name(scalar)
        ),
        TypeInner::Array { base, size, .. } => format!(
            "array<{}, {}>",
            type_name(module, *base),
            match size {
                ArraySize::Constant(count) => count.get().to_string(),
                ArraySize::Pending(_) => "?".to_string(),
                ArraySize::Dynamic => "runtime".to_string(),
            }
        ),
        TypeInner::Struct { .. } => "struct".to_string(),
        other => format!("{other:?}"),
    }
}

fn scalar_name(scalar: &naga::Scalar) -> String {
    use naga::ScalarKind;
    match (scalar.kind, scalar.width) {
        (ScalarKind::Float, 4) => "f32".to_string(),
        (ScalarKind::Uint, 4) => "u32".to_string(),
        (ScalarKind::Sint, 4) => "i32".to_string(),
        other => format!("{other:?}"),
    }
}

fn dimension(size: naga::VectorSize) -> u32 {
    match size {
        naga::VectorSize::Bi => 2,
        naga::VectorSize::Tri => 3,
        naga::VectorSize::Quad => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    const PROBE: &str = "#import bevy_pbr::forward_io::VertexOutput\n\
                         #import bevy_pbr::mesh_view_bindings::view\n\
                         #import bevy_pbr::mesh_view_bindings::lights\n\
                         #import bevy_pbr::mesh_view_bindings::clustered_lights\n\
                         #import bevy_pbr::mesh_view_bindings::globals\n\
                         #import bevy_pbr::mesh_view_bindings::depth_prepass_texture\n\
                         @fragment\n\
                         fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {\n\
                         \x20   let depth = textureLoad(depth_prepass_texture, vec2<i32>(in.position.xy), 0);\n\
                         \x20   let light = clustered_lights.data[0];\n\
                         \x20   return vec4<f32>(\n\
                         \x20       view.exposure + lights.ambient_color.x + globals.time + globals.delta_time\n\
                         \x20           + f32(light.flags) + f32(light.decal_index) + light.range + depth\n\
                         \x20           + view.view_from_clip[3][2] + view.world_from_view[3][3],\n\
                         \x20       view.world_position.x + view.view_from_world[0][0] + view.clip_from_view[0][0],\n\
                         \x20       view.viewport.x + view.viewport.w + in.world_position.x + in.world_normal.x,\n\
                         \x20       in.uv.x + in.uv.y,\n\
                         \x20   );\n\
                         }\n";

    fn probe_module() -> naga::Module {
        let modules = crate::shader::modules();
        let assembled = crate::shader::assemble(PROBE, &modules, crate::stubs::stubs);
        crate::shader::validate("group0 探针", &assembled).expect("组装 + 校验")
    }

    trait FieldLayout {
        const FIELDS: &'static [(&'static str, usize, &'static str)];
    }

    impl FieldLayout for ViewUniform {
        const FIELDS: &'static [(&'static str, usize, &'static str)] = &[
            (
                "world_position",
                offset_of!(ViewUniform, world_position),
                "vec3<f32>",
            ),
            ("exposure", offset_of!(ViewUniform, exposure), "f32"),
            (
                "view_from_world",
                offset_of!(ViewUniform, view_from_world),
                "mat4x4<f32>",
            ),
            (
                "clip_from_view",
                offset_of!(ViewUniform, clip_from_view),
                "mat4x4<f32>",
            ),
            ("viewport", offset_of!(ViewUniform, viewport), "vec4<f32>"),
            (
                "view_from_clip",
                offset_of!(ViewUniform, view_from_clip),
                "mat4x4<f32>",
            ),
            (
                "world_from_view",
                offset_of!(ViewUniform, world_from_view),
                "mat4x4<f32>",
            ),
        ];
    }

    impl FieldLayout for LightsUniform {
        const FIELDS: &'static [(&'static str, usize, &'static str)] = &[
            (
                "ambient_color",
                offset_of!(LightsUniform, ambient_color),
                "vec4<f32>",
            ),
            (
                "n_point_lights",
                offset_of!(LightsUniform, n_point_lights),
                "vec4<u32>",
            ),
        ];
    }

    impl FieldLayout for GlobalsUniform {
        const FIELDS: &'static [(&'static str, usize, &'static str)] = &[
            ("time", offset_of!(GlobalsUniform, time), "f32"),
            ("delta_time", offset_of!(GlobalsUniform, delta_time), "f32"),
            (
                "frame_count",
                offset_of!(GlobalsUniform, frame_count),
                "u32",
            ),
        ];
    }

    impl FieldLayout for ClusteredLight {
        const FIELDS: &'static [(&'static str, usize, &'static str)] = &[
            (
                "light_custom_data",
                offset_of!(ClusteredLight, light_custom_data),
                "vec4<f32>",
            ),
            (
                "color_inverse_square_range",
                offset_of!(ClusteredLight, color_inverse_square_range),
                "vec4<f32>",
            ),
            (
                "position_radius",
                offset_of!(ClusteredLight, position_radius),
                "vec4<f32>",
            ),
            ("flags", offset_of!(ClusteredLight, flags), "u32"),
            (
                "shadow_depth_bias",
                offset_of!(ClusteredLight, shadow_depth_bias),
                "f32",
            ),
            (
                "shadow_normal_bias",
                offset_of!(ClusteredLight, shadow_normal_bias),
                "f32",
            ),
            (
                "spot_light_tan_angle",
                offset_of!(ClusteredLight, spot_light_tan_angle),
                "f32",
            ),
            (
                "soft_shadow_size",
                offset_of!(ClusteredLight, soft_shadow_size),
                "f32",
            ),
            (
                "shadow_map_near_z",
                offset_of!(ClusteredLight, shadow_map_near_z),
                "f32",
            ),
            (
                "decal_index",
                offset_of!(ClusteredLight, decal_index),
                "u32",
            ),
            ("range", offset_of!(ClusteredLight, range), "f32"),
        ];
    }

    fn assert_layout<T: FieldLayout>(layout: &StructLayout) -> String {
        let mut lines = vec![format!(
            "{}｜naga span={} align={}｜Rust size={} align={}",
            layout.name,
            layout.size,
            layout.align,
            size_of::<T>(),
            align_of::<T>()
        )];
        assert_eq!(
            size_of::<T>(),
            layout.size as usize,
            "{} 的整块大小对不上（naga {} / Rust {}）",
            layout.name,
            layout.size,
            size_of::<T>()
        );
        assert_eq!(
            align_of::<T>(),
            layout.align as usize,
            "{} 的对齐对不上（naga {} / Rust {}）",
            layout.name,
            layout.align,
            align_of::<T>()
        );
        assert_eq!(
            T::FIELDS.len(),
            layout.members.len(),
            "{} 的成员个数对不上：naga 声明了 {:?}",
            layout.name,
            layout
                .members
                .iter()
                .map(|member| member.0.as_str())
                .collect::<Vec<_>>()
        );
        for (index, (name, offset, kind)) in T::FIELDS.iter().enumerate() {
            let member = &layout.members[index];
            assert_eq!(
                member.0, *name,
                "{} 第 {index} 个成员的名字对不上：naga 说 '{}'，Rust 说 '{name}'",
                layout.name, member.0
            );
            assert_eq!(
                *offset as u32, member.1,
                "{} 的成员 '{name}' 偏移对不上：naga {} / Rust {offset}",
                layout.name, member.1
            );
            assert_eq!(
                member.2, *kind,
                "{} 的成员 '{name}' 类型对不上：naga 说 {}，Rust 说 {kind}",
                layout.name, member.2
            );
            lines.push(format!(
                "  {name:<26} offset naga={:<4} rust={:<4} type={}",
                member.1, offset, member.2
            ));
        }
        lines.join("\n")
    }

    #[test]
    fn the_host_structs_match_the_reflected_wgsl_layout() {
        let module = probe_module();
        let mut report = Vec::new();
        for (what, (group, binding)) in [
            ("view", VIEW_BINDING),
            ("lights", LIGHTS_BINDING),
            ("globals", GLOBALS_BINDING),
        ] {
            let layout = struct_layout(&module, group, binding)
                .unwrap_or_else(|err| panic!("反射 {what} 失败：{err}"));
            let text = match what {
                "view" => assert_layout::<ViewUniform>(&layout),
                "lights" => assert_layout::<LightsUniform>(&layout),
                _ => assert_layout::<GlobalsUniform>(&layout),
            };
            report.push(text);
        }
        println!("{}", report.join("\n"));
    }

    #[test]
    fn the_group_zero_bindings_are_bevys_numbers() {
        let module = probe_module();
        let rows = crate::shader::bindings(&module);
        let mut table: Vec<((u32, u32), String)> = Vec::new();
        for (group, binding, space, var) in &rows {
            table.push(((*group, *binding), format!("{space} {var}")));
        }
        println!("group 0 契约（反射）：{table:?}");
        for (what, expected) in [
            ("view", VIEW_BINDING),
            ("lights", LIGHTS_BINDING),
            ("clustered_lights", CLUSTERED_LIGHTS_BINDING),
            ("globals", GLOBALS_BINDING),
            ("depth_prepass_texture", DEPTH_PREPASS_BINDING),
        ] {
            let found = table
                .iter()
                .find(|(binding, declaration)| *binding == expected && declaration.ends_with(what))
                .unwrap_or_else(|| {
                    panic!(
                        "组装后的 WGSL 里没有 {what} 在 @group({}) @binding({})",
                        expected.0, expected.1
                    )
                });
            println!(
                "{what:<22} @group({}) @binding({}) {}",
                found.0.0, found.0.1, found.1
            );
        }
        assert_eq!(rows.len(), 5, "探针引了五格，反射出来却是：{table:?}");
        let order: Vec<(u32, u32)> = table.iter().map(|row| row.0).collect();
        assert_eq!(
            order,
            vec![
                VIEW_BINDING,
                LIGHTS_BINDING,
                CLUSTERED_LIGHTS_BINDING,
                GLOBALS_BINDING,
                DEPTH_PREPASS_BINDING,
            ],
            "反射出来的 (group, binding) 次序与 `docs/renderer.md` 的组表不一致"
        );
        assert_eq!(
            table[2].1, "storage clustered_lights",
            "聚类缓冲必须是 storage（不是 uniform）"
        );
        assert_eq!(
            table[4].1, "handle depth_prepass_texture",
            "深度预通道那张图必须是 texture_depth_2d（handle）"
        );
    }

    #[test]
    fn the_storage_array_stride_is_the_light_struct() {
        let module = probe_module();
        let (group, binding) = CLUSTERED_LIGHTS_BINDING;
        let array = storage_array_layout(&module, group, binding).expect("反射聚类数组");
        println!(
            "{}｜步长 {}｜元素 {} 个｜整块 {} 字节｜Rust size_of::<ClusteredLight>() = {}",
            array.name,
            array.stride,
            array.count,
            array.size,
            size_of::<ClusteredLight>()
        );
        assert_eq!(
            array.stride as usize,
            size_of::<ClusteredLight>(),
            "数组步长与 ClusteredLight 的大小必须是同一个数"
        );
        assert_eq!(array.size, array.stride * array.count);
        let light = struct_layout(&module, group, binding).expect("反射外层结构体");
        assert_eq!(light.name, "clustered_lights");
        println!(
            "外层 {}｜span={}｜成员 {:?}",
            light.name,
            light.size,
            light
                .members
                .iter()
                .map(|member| (member.0.as_str(), member.1, member.2.as_str()))
                .collect::<Vec<_>>()
        );
        let element = element_struct_layout(&module, group, binding).expect("反射数组元素");
        assert_eq!(
            element.size, array.stride,
            "元素结构体的大小与数组步长必须是同一个数"
        );
        println!("{}", assert_layout::<ClusteredLight>(&element));
    }

    #[test]
    fn the_view_uniform_carries_the_probe_camera_and_the_exposure_bits() {
        assert_eq!(
            exposure().to_bits(),
            0x3A83_5274,
            "view.exposure 的位模式（按 f32 表达式算，不是 f64 取整）"
        );
        let camera = crate::camera::probe_camera(None, 960.0 / 640.0);
        let view = ViewUniform::from_camera(&camera, [0.0, 0.0, 960.0, 640.0]);
        let bytes = to_bytes(&view);
        assert_eq!(
            bytes.len(),
            288,
            "view 是 160 + 两条逆矩阵的 128 = 288 字节"
        );
        let word = |offset: usize| {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("四个字节"))
        };
        assert_eq!(word(0), camera.position.x.to_bits(), "world_position.x");
        assert_eq!(word(4), camera.position.y.to_bits(), "world_position.y");
        assert_eq!(word(8), camera.position.z.to_bits(), "world_position.z");
        assert_eq!(word(12), 0x3A83_5274, "exposure");
        assert_eq!(
            word(16),
            camera.view_from_world.x_axis.x.to_bits(),
            "view_from_world —— **世界 → 视图**那条逆，不是位姿矩阵"
        );
        assert_eq!(word(76), camera.view_from_world.w_axis.w.to_bits());
        assert_eq!(word(80), camera.clip_from_view.x_axis.x.to_bits());
        assert_eq!(word(140), camera.clip_from_view.w_axis.w.to_bits());
        assert_eq!(word(144), 0.0f32.to_bits(), "viewport.x = 0");
        assert_eq!(word(148), 0.0f32.to_bits(), "viewport.y = 0");
        assert_eq!(word(152), 960.0f32.to_bits(), "viewport.z = 宽");
        assert_eq!(word(156), 640.0f32.to_bits(), "viewport.w = 高");
        assert_eq!(
            word(160),
            camera.view_from_clip.x_axis.x.to_bits(),
            "view_from_clip 紧跟在 viewport 之后"
        );
        assert_eq!(
            word(284),
            camera.world_from_view.w_axis.w.to_bits(),
            "world_from_view 收尾（224 + 60）"
        );
        assert_ne!(
            camera.view_from_world.x_axis.y.to_bits(),
            camera.world_from_view.x_axis.y.to_bits(),
            "位姿与它的逆恰好在这一格上不同（0x00000000 vs 0x80000000）—— \
             同一个数写两处的那种接错法会在这里露出来"
        );
    }

    #[test]
    fn uniform_bytes_are_padded_to_sixteen() {
        let globals = GlobalsUniform {
            time: 0.0,
            delta_time: 0.0,
            frame_count: 0,
        };
        let exact = to_bytes(&globals);
        let padded = to_uniform_bytes(&globals);
        assert_eq!(exact.len(), 12, "globals 就是 12 字节（桩里只有三个标量）");
        assert_eq!(padded.len(), 16, "补齐到 16 的倍数");
        assert_eq!(&padded[..exact.len()], &exact[..], "前缀必须是同样的字节");
        assert!(padded[exact.len()..].iter().all(|byte| *byte == 0));
        assert_eq!(
            to_uniform_bytes(&ViewUniform::from_camera(
                &crate::camera::probe_camera(None, 1.5),
                [0.0, 0.0, 1.0, 1.0],
            ))
            .len(),
            288
        );
        assert_eq!(to_uniform_bytes(&LightsUniform::ambient(80.0)).len(), 32);
    }

    #[test]
    fn the_ambient_light_is_the_scene_number() {
        let bytes = to_bytes(&LightsUniform::ambient(80.0));
        for offset in [0, 4, 8, 12] {
            let word = u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("四个字节"));
            assert_eq!(
                word,
                80.0f32.to_bits(),
                "ambient_color 第 {} 格",
                offset / 4
            );
        }
        assert_eq!(
            to_bytes(&ClusteredLight::absent()).len(),
            80,
            "没写过的那一格是全零 80 字节（内容 shader 靠颜色判它在不在）"
        );
        assert!(
            to_bytes(&ClusteredLight::absent())
                .iter()
                .all(|byte| *byte == 0)
        );
    }

    #[test]
    fn the_point_light_policy_numbers_are_the_oracles_literals() {
        assert_eq!(
            POINT_LIGHT_DEFAULT_RANGE, 20.0,
            "PointLight::default().range"
        );
        assert_eq!(POINT_LIGHT_RADIUS, 0.0, "PointLight::default().radius");
        assert_eq!(
            POINT_LIGHT_SHADOW_DEPTH_BIAS, 0.08,
            "PointLight::DEFAULT_SHADOW_DEPTH_BIAS"
        );
        assert_eq!(
            POINT_LIGHT_SHADOW_NORMAL_BIAS, 0.6,
            "PointLight::DEFAULT_SHADOW_NORMAL_BIAS"
        );
        assert_eq!(
            POINT_LIGHT_SHADOW_MAP_NEAR_Z, 0.1,
            "PointLight::DEFAULT_SHADOW_MAP_NEAR_Z"
        );
        assert_eq!(
            POINT_LIGHT_SHADOW_MAP_SIZE, 1024,
            "PointLightShadowMap 的边长"
        );
        assert_eq!(POINT_LIGHT_FLAGS_SHADOWS_ENABLED, 1, "bit0");
        assert_eq!(
            POINT_LIGHT_FLAGS_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE, 8,
            "bit3 —— `PointLight::default()` 里这个开关是 true，所以它**每盏灯**都在"
        );
        assert_eq!(
            shadow_normal_bias().to_bits(),
            (0.6f32 * core::f32::consts::SQRT_2).to_bits(),
            "shadow_normal_bias＝0.6 × √2（**texel 数**）"
        );
    }

    #[test]
    fn the_sun_light_packs_into_the_oracles_eighty_bytes() {
        let position = [-4.2f32, 1.15, 2.35];
        let reach =
            (position[0] * position[0] + position[1] * position[1] + position[2] * position[2])
                .sqrt()
                * 2.5;
        let light = px_protocol::scene::Light::point("sun", position, [1.0, 1.0, 1.0], 7.6e5)
            .with_range(reach)
            .with_shadows(true);
        let packed = light_of(&light).expect("点光");
        assert_eq!(to_bytes(&packed).len(), 80);

        let intensity = 7.6e5f32 / (4.0 * core::f32::consts::PI);
        assert_eq!(
            packed.color_inverse_square_range,
            [intensity, intensity, intensity, 1.0 / (reach * reach)],
            "rgb = 颜色 × 强度/(4π)（少除一个 4π 就亮 12.57 倍）；w = 1/range²"
        );
        assert_eq!(
            packed.position_radius,
            [position[0], position[1], position[2], 0.0],
            "position_radius.w 是 radius（=0），range 另有一格"
        );
        assert_eq!(packed.flags, 9, "开影子 = bit0|bit3");
        assert_eq!(packed.shadow_depth_bias, 0.08);
        assert_eq!(packed.spot_light_tan_angle, 0.0, "点光没有锥角");
        assert_eq!(packed.soft_shadow_size, 0.0, "PCSS 没开 ⇒ 恒 0");
        assert_eq!(packed.shadow_map_near_z, 0.1);
        assert_eq!(packed.decal_index, u32::MAX, "没有 decal");
        assert_eq!(packed.range, reach);
        let face = Mat4::perspective_infinite_reverse_rh(core::f32::consts::FRAC_PI_2, 1.0, 0.1);
        assert_eq!(
            packed.light_custom_data,
            [face.z_axis.z, face.z_axis.w, face.w_axis.z, face.w_axis.w],
            "custom_data = 投影矩阵的 [2][2] [2][3] [3][2] [3][3]"
        );
        assert_eq!(
            packed.light_custom_data,
            [0.0, -1.0, POINT_LIGHT_SHADOW_MAP_NEAR_Z, 0.0],
            "π/2、aspect 1.0 时它算出来恰好是 (0,-1,near,0)"
        );
    }

    #[test]
    fn an_unlit_light_is_indistinguishable_from_an_empty_slot() {
        let dark =
            px_protocol::scene::Light::point("sun", [-4.2, 1.15, 2.35], [1.0, 1.0, 1.0], 0.0)
                .with_shadows(false);
        let packed = light_of(&dark).expect("点光");
        assert_eq!(
            packed.color_inverse_square_range[..3],
            [0.0, 0.0, 0.0],
            "强度 0 ⇒ 颜色 0 ⇒ shader 判 lit = false"
        );
        assert_eq!(packed.flags, 8, "关影子 = 只剩 bit3");
        assert_ne!(packed.position_radius, [0.0; 4]);
        assert_eq!(packed.position_radius[3], 0.0, "半径本来就是 0");
        let lit = light_of(&px_protocol::scene::Light::point(
            "sun",
            [-4.2, 1.15, 2.35],
            [1.0, 1.0, 1.0],
            1.0,
        ))
        .expect("点光");
        assert_ne!(
            packed.color_inverse_square_range[..3],
            lit.color_inverse_square_range[..3]
        );
        assert_eq!(packed.flags, lit.flags);
        assert_eq!(packed.position_radius, lit.position_radius);
        assert_eq!(packed.range, lit.range, "range 与强度无关（两盏都缺省）");
    }

    #[test]
    fn the_shadow_casting_lights_come_first_and_stay_stable() {
        let light = |id: &str, shadows: bool| {
            px_protocol::scene::Light::point(id, [1.0, 0.0, 0.0], [1.0, 1.0, 1.0], 10.0)
                .with_shadows(shadows)
        };
        let order = lights_of(&[
            light("a", false),
            light("b", true),
            light("c", false),
            light("d", true),
        ])
        .expect("四盏都是点光");
        let flags: Vec<u32> = order.iter().map(|packed| packed.flags).collect();
        assert_eq!(flags, vec![9, 9, 8, 8], "开影子的排前面");
        let probe = |id: &str, shadows: bool, x: f32| {
            px_protocol::scene::Light::point(id, [x, 0.0, 0.0], [1.0, 1.0, 1.0], 10.0)
                .with_shadows(shadows)
        };
        let order = lights_of(&[
            probe("a", false, 1.0),
            probe("b", true, 2.0),
            probe("c", false, 3.0),
            probe("d", true, 4.0),
        ])
        .expect("四盏都是点光");
        let xs: Vec<f32> = order
            .iter()
            .map(|packed| packed.position_radius[0])
            .collect();
        assert_eq!(xs, vec![2.0, 4.0, 1.0, 3.0], "同档内保持文档次序");
    }

    #[test]
    fn spot_and_directional_lights_are_refused() {
        let mut spot = px_protocol::scene::Light::point("s", [0.0; 3], [1.0; 3], 1.0);
        spot.kind = px_protocol::scene::LightKind::Spot;
        let err = light_of(&spot).expect_err("聚光必须拒");
        assert!(err.contains("只兑现**点光源**"), "{err}");
        let mut sun = px_protocol::scene::Light::point("d", [0.0; 3], [1.0; 3], 1.0);
        sun.kind = px_protocol::scene::LightKind::Directional;
        assert!(
            light_of(&sun).is_err(),
            "平行光在 oracle 里根本不住这个缓冲"
        );
    }
}
