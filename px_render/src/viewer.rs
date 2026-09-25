use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::gpu::Gpu;
use crate::render::{self, Views};
use crate::shader;
use crate::shot;

pub const VIEW_REQUEST: &str = "target/viewer-scene.json";
pub const VIEW_LEASE: &str = "target/viewer.json";
pub const VIEW_CAMERA: &str = "target/viewer-camera.json";
pub const VIEW_SHOT: &str = "target/viewer-shot.png";

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const HEARTBEAT: Duration = Duration::from_secs(1);
const LEASE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const LEASE_FRESH: Duration = Duration::from_secs(5);
const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

const YAW_PER_PIXEL: f64 = 0.006;
const PITCH_PER_PIXEL: f64 = 0.006;
const ZOOM_PER_NOTCH: f64 = 0.08;
const PITCH_LIMIT_RADIANS: f64 = 1.25;
const DISTANCE_MIN: f32 = 1.5;
const DISTANCE_MAX: f32 = 14.0;

const PITCH_DEGREES_LIMIT: f32 = 89.5;

fn drag_degrees(per_pixel: f64, pixels: f64) -> f32 {
    (pixels * per_pixel).to_degrees() as f32
}

fn pitch_limit_degrees() -> f32 {
    PITCH_LIMIT_RADIANS.to_degrees() as f32
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ViewRequest {
    scene: String,
    #[serde(default)]
    key: u64,
    at: u64,
    #[serde(default)]
    shot: bool,
    #[serde(default)]
    sheet: bool,
    #[serde(default)]
    ask_camera: bool,
    #[serde(default)]
    set_camera: Option<[f32; 3]>,
    #[serde(default)]
    shot_path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CameraReply {
    at: u64,
    yaw: f32,
    pitch: f32,
    distance: f32,
    position: [f32; 3],
    size: [u32; 2],
    #[serde(default)]
    derived: bool,
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0)
}

fn lease_age() -> Option<Duration> {
    let stamp: u64 = std::fs::read_to_string(VIEW_LEASE)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_nanos(now_nanos().saturating_sub(stamp)))
}

fn write_request(request: &ViewRequest) -> Result<(), String> {
    let text = serde_json::to_string_pretty(request).map_err(|err| err.to_string())?;
    std::fs::write(VIEW_REQUEST, text).map_err(|err| format!("写 {VIEW_REQUEST} 失败：{err}"))
}

fn read_request() -> Option<ViewRequest> {
    let text = std::fs::read_to_string(VIEW_REQUEST).ok()?;
    serde_json::from_str(&text).ok()
}

fn fingerprint_of(path: &Path) -> Result<u64, String> {
    let bundle = px_protocol::art::read_manifest(path)?;
    Ok(bundle
        .assets
        .first()
        .map(|asset| asset.fingerprint)
        .unwrap_or(0))
}

fn orbit_of(flag: &str, place: [f32; 3]) -> Result<[f32; 3], String> {
    if !place.iter().all(|value| value.is_finite()) {
        return Err(format!(
            "{flag} 要三个**有限的**数（yaw,pitch,distance），收到 {place:?}"
        ));
    }
    Ok([
        place[0],
        place[1].clamp(-PITCH_DEGREES_LIMIT, PITCH_DEGREES_LIMIT),
        place[2],
    ])
}

pub fn show(scene: &Path, shot: Option<PathBuf>) -> Result<(), String> {
    let scene = scene.display().to_string();
    let key = fingerprint_of(Path::new(&scene))?;
    let request = ViewRequest {
        scene: scene.clone(),
        key,
        at: now_nanos(),
        shot: shot.is_some(),
        sheet: false,
        ask_camera: false,
        set_camera: None,
        shot_path: shot.map(|path| path.display().to_string()),
    };
    write_request(&request)?;

    println!("已推给常驻窗口：{scene}（键 {key:016x}）");
    match lease_age() {
        Some(age) if age < LEASE_FRESH => {
            println!("窗口在线（心跳 {:.1} s 前）", age.as_secs_f32())
        }
        _ => println!(
            "⚠ 没检测到在跑的窗口；先执行 `px_render --view --scene …` 开一个，它会一直留着"
        ),
    }
    if request.shot {
        println!(
            "截图由**窗口**那边写（它每 {} ms 看一次请求文件）：{}",
            POLL_INTERVAL.as_millis(),
            request.shot_path.as_deref().unwrap_or(VIEW_SHOT)
        );
    }
    Ok(())
}

pub fn camera_query(place: Option<[f32; 3]>) -> Result<(), String> {
    let place = place.map(|place| orbit_of("--place", place)).transpose()?;
    let previous = read_request().ok_or_else(|| {
        format!(
            "读不到 {VIEW_REQUEST}：先 `px_render --view --scene <SCENE.pxart>` 开一个窗口，\
             或 `--show --scene <SCENE.pxart>` 推一份"
        )
    })?;
    let at = now_nanos();
    let request = ViewRequest {
        scene: previous.scene,
        key: previous.key,
        at,
        shot: false,
        sheet: previous.sheet,
        ask_camera: true,
        set_camera: place,
        shot_path: None,
    };
    write_request(&request)?;
    let _ = std::fs::remove_file(VIEW_CAMERA);

    let deadline = Instant::now() + REPLY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(VIEW_CAMERA) {
            if let Ok(reply) = serde_json::from_str::<CameraReply>(&text) {
                if reply.at == at {
                    println!(
                        "相机 yaw {:.4}｜pitch {:.4}｜distance {:.4}｜位置 ({:.3}, {:.3}, {:.3})｜视口 {}×{}",
                        reply.yaw,
                        reply.pitch,
                        reply.distance,
                        reply.position[0],
                        reply.position[1],
                        reply.position[2],
                        reply.size[0],
                        reply.size[1],
                    );
                    if reply.derived {
                        println!(
                            "⚠ 这三个角是**从探针位姿反推**的（窗口还没被鼠标或 --place 接管）：\
                             位置那一栏是真的，`--place` 它得到的是**轨道**那一档"
                        );
                    }
                    println!(
                        "--place {:.4},{:.4},{:.4}",
                        reply.yaw, reply.pitch, reply.distance
                    );
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!(
        "窗口没回话（{} s）：确认 `px_render --view` 在跑（租约 {}）",
        REPLY_TIMEOUT.as_secs(),
        VIEW_LEASE
    ))
}

pub fn view(options: &crate::Options) -> Result<(), String> {
    let request = match options.shots.first() {
        Some(shot) => {
            let scene = shot.scene.display().to_string();
            let key = fingerprint_of(Path::new(&scene))?;
            ViewRequest {
                scene,
                key,
                at: now_nanos(),
                shot: options.shot.is_some(),
                sheet: options.sheet,
                ask_camera: false,
                set_camera: None,
                shot_path: options
                    .shot
                    .as_ref()
                    .map(|path| path.display().to_string()),
            }
        }
        None => read_request().ok_or_else(|| {
            format!(
                "--view 需要一个起始场景：给 --scene <SCENE.pxart>，或先用 --show 推一份（{VIEW_REQUEST}）"
            )
        })?,
    };

    if let Some(age) = lease_age() {
        if age < LEASE_FRESH {
            return Err(format!(
                "已经有一个预览窗口在跑（{} 的心跳 {:.1} s 前，上限 {} s）：\
                 先关掉它（q / Esc / 关窗口），或删掉那个文件再起",
                VIEW_LEASE,
                age.as_secs_f32(),
                LEASE_FRESH.as_secs()
            ));
        }
    }

    println!(
        "预览窗口：当前目录 {}｜场景 {}｜请求文件 {VIEW_REQUEST}｜租约 {VIEW_LEASE}",
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "（拿不到）".to_string()),
        request.scene,
    );

    let orbit = match options.view_cam() {
        Some(place) => Some(orbit_of("--cam", place)?),
        None => None,
    };

    let mut viewer = Viewer {
        scene: request.scene.clone(),
        key: request.key,
        request_at: request.at,
        orbit,
        pending_shot: initial_shot(&request),
        ui_shot: options.ui_shot.clone(),
        ui_shot_ok: false,
        sheet: request.sheet,
        size: (options.width, options.height),
        pcg_root: options.pcg_root.clone(),
        novsync: options.novsync,
        window: None,
        gpu: None,
        surface: None,
        config: None,
        present: None,
        dragging: false,
        cursor: None,
        session: None,
        session_key: None,
        shaders: match ShaderWatch::new() {
            Ok(watch) => {
                println!("shader 热重载：看着 {}", watch.describe());
                Some(watch)
            }
            Err(err) => {
                eprintln!("⚠ shader 热重载这一路看不了盘：{err}");
                None
            }
        },
        shader_changes: Vec::new(),
        image_hash: options.image_hash,
        panel: None,
        edit_recipe: options.edit.clone(),
        panel_recipe: None,
        dirty: true,
        last: None,
        scene_modified: std::fs::metadata(&request.scene)
            .and_then(|meta| meta.modified())
            .ok(),
        first_frame: true,
        frames: 0,
        last_poll: Instant::now(),
        last_heartbeat: Instant::now(),
    };

    println!(
        "操作：左键拖动 = 转视角、滚轮 = 缩放；s = 存一张图（落到 --shot 那条路径）；q / Esc = 退出"
    );
    println!("  推一份新场景进来：px_render --show --scene <SCENE.pxart> [--shot PNG]");
    println!("  问/摆相机：px_render --where ｜ px_render --place yaw,pitch,distance");
    println!("  截图落点：{}（--shot 不给路径时）", VIEW_SHOT);
    if options.fps {
        println!(
            "（--fps 收下但不生效：本窗口是**按需渲染**的（相机/场景一变才画一帧），\
             没有逐帧的帧率可报 —— 帧率读数属于把渲染做成逐帧的那一档）"
        );
    }

    let event_loop = EventLoop::new().map_err(|err| format!("起事件循环失败：{err}"))?;
    spawn_lease_watch();
    event_loop
        .run_app(&mut viewer)
        .map_err(|err| format!("事件循环退出：{err}"))?;
    Ok(())
}

fn spawn_lease_watch() {
    std::thread::spawn(move || {
        let mut missing = 0;
        loop {
            std::thread::sleep(LEASE_CHECK_INTERVAL);
            if std::fs::read_to_string(VIEW_LEASE).is_ok() {
                missing = 0;
                continue;
            }
            missing += 1;
            if missing >= 2 {
                println!("租约没了（{VIEW_LEASE}），预览窗口退出");
                std::process::exit(0);
            }
        }
    });
}

fn initial_shot(request: &ViewRequest) -> Option<PathBuf> {
    if !request.shot {
        return None;
    }
    Some(PathBuf::from(
        request.shot_path.as_deref().unwrap_or(VIEW_SHOT),
    ))
}

fn write_lease() {
    let _ = std::fs::write(VIEW_LEASE, format!("{}", now_nanos()));
}

fn heartbeat_now() {
    if !std::path::Path::new(VIEW_LEASE).exists() {
        return;
    }
    write_lease();
}

struct Present {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind: Option<wgpu::BindGroup>,
    texture: Option<wgpu::Texture>,
    size: (u32, u32),
}

impl Present {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Present {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("px_render 呈现"),
            source: wgpu::ShaderSource::Wgsl(PRESENT_WGSL.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("px_render 呈现"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("px_render 呈现"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("px_render 呈现"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Present {
            layout,
            pipeline,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("px_render 呈现"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
            bind: None,
            texture: None,
            size: (0, 0),
        }
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        if self.texture.is_none() || self.size != (width, height) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("px_render 呈现源"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: shot::FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[shot::FORMAT.remove_srgb_suffix()],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("px_render 呈现源（非 sRGB）"),
                format: Some(shot::FORMAT.remove_srgb_suffix()),
                ..Default::default()
            });
            self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("px_render 呈现源"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
            self.texture = Some(texture);
            self.size = (width, height);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: self.texture.as_ref().expect("刚建过"),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("px_render 呈现"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let Some(bind) = &self.bind else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

const PRESENT_WGSL: &str = r#"
struct PxPresentOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var px_source: texture_2d<f32>;
@group(0) @binding(1) var px_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> PxPresentOut {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: PxPresentOut;
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: PxPresentOut) -> @location(0) vec4<f32> {
    return textureSample(px_source, px_sampler, in.uv);
}
"#;

fn read_back_ui(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    format: wgpu::TextureFormat,
    path: &Path,
) -> Result<(), String> {
    let width = texture.width();
    let height = texture.height();
    let unpadded = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("px_render --ui-shot 回读"),
        size: (padded as u64) * (height as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("px_render --ui-shot"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let swap_red_blue = matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    if format == wgpu::TextureFormat::Bgra8Unorm {
        eprintln!(
            "⚠ 交换链格式是 {format:?}（非 sRGB 的 BGRA）⇒ --ui-shot 没做线性→sRGB 编码，图会偏暗"
        );
    }
    let bytes = {
        let view = slice.get_mapped_range();
        let mut out = Vec::with_capacity((unpadded as usize) * (height as usize));
        for row in 0..height as usize {
            let start = row * padded as usize;
            let src = &view[start..start + unpadded as usize];
            if swap_red_blue {
                swizzle_bgra_to_rgba(src, &mut out);
            } else {
                out.extend_from_slice(src);
            }
        }
        out
    };
    buffer.unmap();
    shot::write_png(path, width, height, bytes).map(|_bytes| ())
}

fn swizzle_bgra_to_rgba(source: &[u8], out: &mut Vec<u8>) {
    for pixel in source.chunks_exact(4) {
        out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
}

struct Viewer {
    scene: String,
    key: u64,
    request_at: u64,
    orbit: Option<[f32; 3]>,
    pending_shot: Option<PathBuf>,
    ui_shot: Option<PathBuf>,
    ui_shot_ok: bool,
    sheet: bool,
    size: (u32, u32),
    pcg_root: PathBuf,
    novsync: bool,

    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    present: Option<Present>,

    dragging: bool,
    cursor: Option<(f64, f64)>,
    session: Option<render::Session>,
    session_key: Option<(String, u64)>,
    shaders: Option<ShaderWatch>,
    shader_changes: Vec<PathBuf>,
    image_hash: bool,
    panel: Option<crate::panel::Panel>,
    edit_recipe: Option<String>,
    panel_recipe: Option<String>,
    dirty: bool,
    last: Option<(u32, u32)>,
    first_frame: bool,
    frames: u64,
    scene_modified: Option<SystemTime>,
    last_poll: Instant,
    last_heartbeat: Instant,
}

struct ShaderWatch {
    files: Vec<Watched>,
}

struct Watched {
    path: PathBuf,
    alarm: Option<(SystemTime, u64)>,
    hash: Option<String>,
}

impl ShaderWatch {
    fn new() -> Result<ShaderWatch, String> {
        ShaderWatch::of(shader::watch_files()?)
    }

    fn of(files: Vec<PathBuf>) -> Result<ShaderWatch, String> {
        let files = files
            .into_iter()
            .map(|path| {
                Ok(Watched {
                    alarm: alarm_of(&path),
                    hash: content_hash_of(&path),
                    path,
                })
            })
            .collect::<Result<Vec<Watched>, String>>()?;
        Ok(ShaderWatch { files })
    }

    fn describe(&self) -> String {
        let names: Vec<String> = self
            .files
            .iter()
            .map(|watched| {
                watched
                    .path
                    .strip_prefix(shader::workspace())
                    .unwrap_or(&watched.path)
                    .display()
                    .to_string()
            })
            .collect();
        format!("{} 个：{}", names.len(), names.join(" / "))
    }

    fn changed(&mut self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut changed = Vec::new();
        let mut touched = Vec::new();
        for watched in self.files.iter_mut() {
            let alarm = alarm_of(&watched.path);
            if alarm == watched.alarm {
                continue;
            }
            watched.alarm = alarm;
            let hash = content_hash_of(&watched.path);
            if hash == watched.hash {
                touched.push(watched.path.clone());
                continue;
            }
            watched.hash = hash;
            changed.push(watched.path.clone());
        }
        (changed, touched)
    }
}

fn alarm_of(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn content_hash_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(crate::digest::sha256_hex(&bytes)[..16].to_string())
}

impl Viewer {
    fn panel_wants_pointer(&self) -> bool {
        self.panel
            .as_ref()
            .is_some_and(crate::panel::Panel::wants_pointer)
    }

    fn views(&self) -> Views {
        Views::Single(self.orbit)
    }

    fn camera(&self, width: u32, height: u32) -> crate::camera::Camera {
        crate::camera::probe_camera(self.orbit, width as f32 / height as f32)
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let physical = PhysicalSize::new(self.size.0, self.size.1);
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.title())
                        .with_inner_size(physical),
                )
                .map_err(|err| format!("建窗口失败：{err}"))?,
        );
        let instance = crate::gpu::instance();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|err| format!("把窗口接到 wgpu 失败：{err}"))?;

        let size = window.inner_size();
        let gpu = crate::gpu::connect_with(instance, Some(&surface));
        let capabilities = surface.get_capabilities(&gpu.adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .ok_or_else(|| {
                format!(
                    "这块 surface 一个 sRGB 格式都不给（{:?}）：画面是 sRGB 的字节，换一个后端",
                    capabilities.formats
                )
            })?;
        let capability = capabilities.usages;
        let copy_src = capability.contains(wgpu::TextureUsages::COPY_SRC);
        if self.ui_shot.is_some() && !copy_src {
            eprintln!(
                "⚠ 这块 surface 不支持 COPY_SRC（{:?}）⇒ --ui-shot 这一档用不了：\
                 界面截图要能把交换链那一张读回来。窗口照常开，画面照常画",
                capability
            );
            self.ui_shot = None;
        }
        let config = wgpu::SurfaceConfiguration {
            usage: if self.ui_shot.is_some() {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: if self.novsync {
                wgpu::PresentMode::AutoNoVsync
            } else {
                wgpu::PresentMode::AutoVsync
            },
            desired_maximum_frame_latency: 2,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![format.remove_srgb_suffix()],
        };
        surface.configure(&gpu.device, &config);

        println!(
            "表面：{:?}｜呈现模式 {:?}｜视图格式 {:?}｜物理尺寸 {}×{}（缩放因子 {}）｜适配器 {}",
            config.format,
            config.present_mode,
            config.view_formats,
            config.width,
            config.height,
            window.scale_factor(),
            gpu.adapter.get_info().name,
        );

        self.present = Some(Present::new(&gpu.device, format.remove_srgb_suffix()));
        self.size = (config.width, config.height);
        self.ui_shot_ok = copy_src;

        let recipe = self.edit_recipe.clone().or_else(|| {
            crate::edit::derive_recipe(shader::workspace(), &self.pcg_root, Path::new(&self.scene))
        });
        let store = recipe.as_deref().and_then(|recipe| {
            match crate::edit::ParamStore::open(shader::workspace(), recipe) {
                Ok(store) => Some(store),
                Err(err) => {
                    eprintln!("⚠ 调参面板的编辑面 `{recipe}` 开不起来：{err}");
                    None
                }
            }
        });
        let mut panel = crate::panel::Panel::new(&window, recipe.clone(), store);
        panel.ready(&gpu.device, config.format);
        println!(
            "调参面板：Tab 收起/展开｜编辑面 {}｜会话副本 {}",
            match self.edit_recipe.as_deref() {
                Some(recipe) => format!("场景配方 `{recipe}`（--edit 给的）"),
                None => match &recipe {
                    Some(recipe) => {
                        format!("场景配方 `{recipe}`（按产物名推的；可用 --edit 指明）")
                    }
                    None => "（没开：给 --edit <场景配方名>）".to_string(),
                },
            },
            crate::edit::store_root(shader::workspace()).display(),
        );
        self.panel_recipe = recipe;
        self.panel = Some(panel);

        self.gpu = Some(gpu);
        self.surface = Some(surface);
        self.config = Some(config);
        self.window = Some(window);

        let _ = write_request(&ViewRequest {
            scene: self.scene.clone(),
            key: self.key,
            at: self.request_at,
            shot: self.pending_shot.is_some(),
            sheet: self.sheet,
            ask_camera: false,
            set_camera: None,
            shot_path: self
                .pending_shot
                .as_ref()
                .map(|path| path.display().to_string()),
        });
        write_lease();
        Ok(())
    }

    fn title(&self) -> String {
        format!(
            "px_render 预览 — {}｜键 {:016x}｜{}",
            self.scene,
            self.key,
            describe_orbit(self.orbit)
        )
    }

    fn update_title(&self) {
        if let Some(window) = &self.window {
            window.set_title(&self.title());
        }
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        let (Some(surface), Some(config), Some(gpu)) = (
            self.surface.as_ref(),
            self.config.as_mut(),
            self.gpu.as_ref(),
        ) else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }
        config.width = size.width;
        config.height = size.height;
        surface.configure(&gpu.device, config);
        self.size = (size.width, size.height);
        self.dirty = true;
    }

    fn current_position(&self) -> [f32; 3] {
        let (width, height) = self.size;
        let camera = self.camera(width.max(1), height.max(1));
        [camera.position.x, camera.position.y, camera.position.z]
    }

    fn drag(&mut self, position: (f64, f64)) {
        let previous = self.cursor.replace(position);
        if !self.dragging {
            return;
        }
        let Some(previous) = previous else {
            return;
        };
        let (dx, dy) = (position.0 - previous.0, position.1 - previous.1);
        let mut state = self
            .orbit
            .unwrap_or_else(|| seed_orbit(self.current_position()));
        state[0] -= drag_degrees(YAW_PER_PIXEL, dx);
        let limit = pitch_limit_degrees();
        state[1] = (state[1] - drag_degrees(PITCH_PER_PIXEL, dy)).clamp(-limit, limit);
        self.orbit = Some(state);
        self.dirty = true;
        self.update_title();
    }

    fn wheel(&mut self, delta: MouseScrollDelta) {
        let notches = match delta {
            MouseScrollDelta::LineDelta(_, y) => f64::from(y),
            MouseScrollDelta::PixelDelta(position) => position.y / 100.0,
        };
        if notches == 0.0 {
            return;
        }
        let mut state = self
            .orbit
            .unwrap_or_else(|| seed_orbit(self.current_position()));
        state[2] = (f64::from(state[2]) * (1.0 - notches * ZOOM_PER_NOTCH))
            .clamp(f64::from(DISTANCE_MIN), f64::from(DISTANCE_MAX)) as f32;
        self.orbit = Some(state);
        self.dirty = true;
        self.update_title();
    }

    fn place(&mut self, place: [f32; 3]) {
        match orbit_of("--place", place) {
            Ok(place) => {
                self.orbit = Some(place);
                self.dirty = true;
                self.update_title();
                println!(
                    "相机被摆到 yaw {:.4}｜pitch {:.4}｜distance {:.4}",
                    place[0], place[1], place[2]
                );
            }
            Err(message) => eprintln!("{message}"),
        }
    }

    fn answer_camera(&self, at: u64) {
        let (width, height) = self.size;
        let camera = self.camera(width.max(1), height.max(1));
        let (yaw, pitch, distance, derived) = match self.orbit {
            Some(state) => (state[0], state[1], state[2], false),
            None => {
                let position = camera.position;
                let angles = seed_orbit([position.x, position.y, position.z]);
                (angles[0], angles[1], angles[2], true)
            }
        };
        let reply = CameraReply {
            at,
            yaw,
            pitch,
            distance,
            position: [camera.position.x, camera.position.y, camera.position.z],
            size: [width, height],
            derived,
        };
        match serde_json::to_string_pretty(&reply) {
            Ok(text) => {
                if let Err(err) = std::fs::write(VIEW_CAMERA, text) {
                    eprintln!("写相机回话失败：{err}");
                }
            }
            Err(err) => eprintln!("序列化相机回话失败：{err}"),
        }
    }

    fn poll_request(&mut self) {
        let Some(request) = read_request() else {
            return;
        };
        if request.at == self.request_at {
            return;
        }
        self.request_at = request.at;
        if let Some(place) = request.set_camera {
            self.place(place);
        }
        if request.ask_camera {
            self.answer_camera(request.at);
        }
        if request.shot {
            self.pending_shot = Some(PathBuf::from(
                request.shot_path.as_deref().unwrap_or(VIEW_SHOT),
            ));
            self.dirty = true;
        }
        let changed = request.scene != self.scene
            || request.key == 0
            || request.key != self.key
            || self.last.is_none();
        if request.scene != self.scene {
            self.scene = request.scene.clone();
            self.scene_modified = std::fs::metadata(&self.scene)
                .and_then(|meta| meta.modified())
                .ok();
            self.first_frame = true;
        }
        self.key = request.key;
        self.sheet = request.sheet;
        if request.sheet {
            println!("⚠ 预览窗口不出多视口对照图（那是 --serve 出图的事）：这一份的相机表只当参考");
        }
        if changed {
            self.dirty = true;
            self.update_title();
            println!("窗口切到：{}｜键 {:016x}", self.scene, self.key);
        } else {
            println!("同一份产物（键 {:016x}）再推一次 ⇒ 不重画", self.key);
        }
    }

    fn poll_shaders(&mut self) {
        let mut changed = std::mem::take(&mut self.shader_changes);
        let mut touched: Vec<PathBuf> = Vec::new();
        if let Some(watch) = self.shaders.as_mut() {
            let (now, alarms) = watch.changed();
            changed.extend(now);
            touched = alarms;
        }
        for path in &touched {
            println!(
                "shader 文件动过，但**字节没变**（{}）⇒ 不重载、不重画",
                path.strip_prefix(shader::workspace())
                    .unwrap_or(path)
                    .display()
            );
        }
        if changed.is_empty() {
            return;
        }
        changed.sort();
        changed.dedup();
        let Some(session) = self.session.as_mut() else {
            self.shader_changes = changed;
            return;
        };
        let report = session.reload_shaders(&changed);
        for line in report.readout() {
            println!("{line}");
        }
        if self.image_hash {
            println!(
                "｜热重载时刻 t={}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|since| since.as_millis())
                    .unwrap_or(0)
            );
        }
        if !report.reloaded.is_empty() {
            self.dirty = true;
            self.update_title();
        }
    }

    fn poll_cook(&mut self) {
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        let Some(update) = panel.poll() else {
            if panel.is_open() && panel.busy() {
                self.dirty = true;
            }
            return;
        };
        let Some((path, fingerprint)) = update.scene else {
            self.dirty = true;
            return;
        };
        if path.display().to_string() != self.scene {
            println!(
                "烘好的是 {}，而窗口正看着 {}：画面不动（要换过去用 --show）",
                path.display(),
                self.scene
            );
            self.dirty = true;
            return;
        }
        if fingerprint == self.key {
            println!("烘好的还是同一份内容（键 {fingerprint:016x}）⇒ 不重画");
            self.dirty = true;
            return;
        }
        self.scene_modified = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok();
        self.key = fingerprint;
        self.first_frame = true;
        self.dirty = true;
        self.update_title();
        println!("面板烘完，重载：{}（键 {fingerprint:016x}）", self.scene);
    }

    fn poll_scene_file(&mut self) {
        let Ok(meta) = std::fs::metadata(&self.scene) else {
            return;
        };
        let Ok(modified) = meta.modified() else {
            return;
        };
        if self.scene_modified == Some(modified) {
            return;
        }
        self.scene_modified = Some(modified);
        match fingerprint_of(Path::new(&self.scene)) {
            Ok(fingerprint) if fingerprint != 0 && fingerprint == self.key => {
                println!("场景产物动过，但载荷指纹没变（{fingerprint:016x}）⇒ 不重画");
            }
            Ok(fingerprint) => {
                self.key = fingerprint;
                self.dirty = true;
                self.update_title();
                println!(
                    "场景产物更新，重载：{}（键 {fingerprint:016x}）",
                    self.scene
                );
            }
            Err(err) => println!("场景产物动过，读不到清单：{err}"),
        }
    }

    fn draw(&mut self) {
        if self.window.is_none() || self.gpu.is_none() {
            return;
        }

        let mut note: Option<String> = None;
        if self.dirty {
            self.dirty = false;
            let views = self.views();
            let scene = self.scene.clone();
            let pcg_root = self.pcg_root.clone();
            let (width, height) = self.size;
            let started = Instant::now();
            let rendered = {
                let Some(gpu) = self.gpu.as_ref() else {
                    return;
                };
                let stale = match (&self.session, &self.session_key) {
                    (Some(_), Some(key)) => *key != (scene.clone(), self.key),
                    _ => true,
                };
                let mut failed: Option<String> = None;
                if stale {
                    match render::Session::open(
                        gpu,
                        Path::new(&scene),
                        &pcg_root,
                        views,
                        width,
                        height,
                    ) {
                        Ok(session) => {
                            self.session = Some(session);
                            self.session_key = Some((scene.clone(), self.key));
                        }
                        Err(message) => {
                            self.session = None;
                            self.session_key = None;
                            failed = Some(message);
                        }
                    }
                }
                match failed {
                    Some(message) => Err(message),
                    None => match self.session.as_mut() {
                        Some(session) => session.draw(gpu, views, width, height),
                        None => Err("保留态不见了（内部不一致）".to_string()),
                    },
                }
            };
            match rendered {
                Ok(rendered) => {
                    let (width, height) = (rendered.width, rendered.height);
                    if self.first_frame {
                        for line in &rendered.audit {
                            println!("{line}");
                        }
                        self.first_frame = false;
                    }
                    if let Some(gpu) = self.gpu.as_ref() {
                        if let Some(present) = self.present.as_mut() {
                            present.upload(
                                &gpu.device,
                                &gpu.queue,
                                width,
                                height,
                                &rendered.pixels,
                            );
                        }
                    }
                    let uploaded = std::time::Instant::now();
                    let image: Option<String> = if self.image_hash {
                        Some(crate::digest::sha256_hex(&rendered.pixels)[..16].to_string())
                    } else {
                        None
                    };
                    let mut shot_ms: u128 = 0;
                    if let Some(path) = self.pending_shot.take() {
                        match shot::write_png(&path, width, height, rendered.pixels) {
                            Ok(bytes) => println!(
                                "预览截图 → {}（{}×{}，{} 字节，sha256 {}{}）",
                                path.display(),
                                width,
                                height,
                                bytes,
                                crate::digest::short(&path),
                                match &image {
                                    Some(image) => format!("｜像素 {image}"),
                                    None => String::new(),
                                }
                            ),
                            Err(message) => eprintln!("{message}"),
                        }
                        shot_ms = uploaded.elapsed().as_millis();
                    }
                    self.last = Some((width, height));
                    self.frames += 1;
                    note = Some(format!(
                        "第 {} 帧：{}×{}｜{}｜画+上传 {} ms｜截图 {}{}",
                        self.frames,
                        width,
                        height,
                        describe_orbit(self.orbit),
                        (uploaded - started).as_millis(),
                        shot_ms,
                        if let Some(image) = &image {
                            format!(
                                "｜像素 {image}｜t={}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|since| since.as_millis())
                                    .unwrap_or(0)
                            )
                        } else {
                            String::new()
                        }
                    ));
                }
                Err(message) => {
                    eprintln!("⚠ 这一帧画不出来，保留窗口里现在这张图：{message}");
                }
            }
        }

        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let present_started = Instant::now();
        let (device, queue) = (&gpu.device, &gpu.queue);
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(device, config);
                println!(
                    "第 {} 帧画出来了，但**没有呈现**（交换链 Outdated/Lost ⇒ 重新配置，这一帧丢掉）\
                     —— 立刻再画一帧",
                    self.frames
                );
                self.dirty = true;
                self.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                println!(
                    "第 {} 帧画出来了，但**没有呈现**（交换链 Timeout/Occluded —— 窗口被挡住或不可见）",
                    self.frames
                );
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("⚠ 交换链拿不到下一张（校验错）：这一帧跳过");
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("px_render 呈现（非 sRGB）"),
            format: Some(config.format.remove_srgb_suffix()),
            ..Default::default()
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_render 呈现"),
        });
        if let Some(present) = self.present.as_ref() {
            present.draw(&mut encoder, &view);
        }
        queue.submit(Some(encoder.finish()));

        if let Some(panel) = self.panel.as_mut() {
            if panel.is_open() {
                if let Some(window) = self.window.as_ref() {
                    let [width, height] = [config.width, config.height];
                    let panel_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                        label: Some("px_render 面板（原生格式）"),
                        ..Default::default()
                    });
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("px_render 面板"),
                        });
                    panel.paint(
                        window,
                        device,
                        queue,
                        &mut encoder,
                        &panel_view,
                        [width, height],
                        shader::workspace(),
                    );
                    queue.submit(Some(encoder.finish()));
                }
            }
        }

        if let Some(path) = self.ui_shot.take() {
            match read_back_ui(device, queue, &frame.texture, config.format, &path) {
                Ok(()) => {
                    let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                    println!(
                        "界面截图（画面 + 面板）→ {}（{}×{}，{} 字节）",
                        path.display(),
                        config.width,
                        config.height,
                        bytes,
                    );
                }
                Err(message) => eprintln!("界面截图失败：{message}"),
            }
        }

        frame.present();
        if let Some(note) = note {
            println!("{note}｜呈现 {} ms", present_started.elapsed().as_millis());
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn shot_now(&mut self) {
        let path = self
            .pending_shot
            .clone()
            .unwrap_or_else(|| PathBuf::from(VIEW_SHOT));
        self.pending_shot = Some(path);
        self.dirty = true;
        self.request_redraw();
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_file(VIEW_LEASE);
        println!("预览窗口收工（{} 帧）", self.frames);
    }
}

fn seed_orbit(position: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = position;
    let length = (x * x + y * y + z * z).sqrt();
    [
        x.atan2(z).to_degrees(),
        if length > 0.0 {
            (y / length).asin().to_degrees()
        } else {
            0.0
        },
        length,
    ]
}

fn describe_orbit(orbit: Option<[f32; 3]>) -> String {
    match orbit {
        Some([yaw, pitch, distance]) => {
            format!("相机 yaw {yaw:.4}｜pitch {pitch:.4}｜distance {distance:.4}")
        }
        None => "相机：探针机位（不给 --cam 那一档）".to_string(),
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(message) = self.open(event_loop) {
            eprintln!("{message}");
            event_loop.exit();
            return;
        }
        self.update_title();
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            if key.state == ElementState::Pressed
                && matches!(
                    key.logical_key,
                    Key::Named(NamedKey::Tab) | Key::Named(NamedKey::F1)
                )
            {
                if let Some(panel) = self.panel.as_mut() {
                    panel.toggle();
                    println!(
                        "调参面板：{}",
                        if panel.is_open() { "展开" } else { "收起" }
                    );
                    if panel.is_open() {
                        self.dirty = true;
                        self.request_redraw();
                    }
                }
                return;
            }
        }
        let for_panel = !matches!(
            event,
            WindowEvent::RedrawRequested
                | WindowEvent::CloseRequested
                | WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
        );
        if for_panel {
            if let (Some(panel), Some(window)) = (self.panel.as_mut(), self.window.as_ref()) {
                if panel.on_window_event(window, &event) {
                    return;
                }
            }
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::ScaleFactorChanged { .. } => {}
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
                if !self.dragging {
                    println!(
                        "拖到：{}｜{}",
                        describe_orbit(self.orbit),
                        place_line(self.orbit)
                    );
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.panel_wants_pointer() {
                    return;
                }
                self.drag((position.x, position.y));
                if self.dirty {
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.wheel(delta);
                if self.dirty {
                    self.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Character(ref text) if text.eq_ignore_ascii_case("q") => event_loop.exit(),
                    Key::Character(ref text) if text.eq_ignore_ascii_case("s") => self.shot_now(),
                    Key::Character(ref text) if text.eq_ignore_ascii_case("u") => {
                        if !self.ui_shot_ok {
                            eprintln!(
                                "⚠ 这块 surface 不支持 COPY_SRC ⇒ 读不回交换链那一张，\
                                 界面截图用不了（`--shot` 那条路不受影响）"
                            );
                            return;
                        }
                        let path = self
                            .ui_shot
                            .clone()
                            .unwrap_or_else(|| PathBuf::from("target/viewer-ui.png"));
                        println!("界面截图将落到 {}", path.display());
                        self.ui_shot = Some(path);
                        self.dirty = true;
                        self.request_redraw();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now.duration_since(self.last_poll) >= POLL_INTERVAL {
            self.last_poll = now;
            self.poll_request();
            self.poll_scene_file();
            self.poll_shaders();
            self.poll_cook();
            if self.dirty {
                self.request_redraw();
            }
        }
        if now.duration_since(self.last_heartbeat) >= HEARTBEAT {
            self.last_heartbeat = now;
            heartbeat_now();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_poll + POLL_INTERVAL));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.cleanup();
    }
}

fn place_line(orbit: Option<[f32; 3]>) -> String {
    match orbit {
        Some([yaw, pitch, distance]) => format!("--place {yaw:.4},{pitch:.4},{distance:.4}"),
        None => "（探针机位：没有对应的 --place）".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bevy_shaped_request_still_parses_and_ours_still_parses_for_it() {
        let bevy = r#"{
            "scene": "a.pxart",
            "key": 18,
            "at": 7,
            "shot": true,
            "sheet": false,
            "ask_camera": false,
            "set_camera": null
        }"#;
        let request: ViewRequest = serde_json::from_str(bevy).expect("Bevy 写的那一份要解得开");
        assert_eq!(request.scene, "a.pxart");
        assert_eq!(request.at, 7);
        assert!(request.shot);
        assert_eq!(
            request.shot_path, None,
            "Bevy 那一版没有这一格 ⇒ 缺省是 None ⇒ 截图落到它写死的那条路径"
        );
        assert_eq!(
            initial_shot(&request),
            Some(PathBuf::from(VIEW_SHOT)),
            "缺省必须是 Bevy 的 `target/viewer-shot.png`（不是我们发明的路径）"
        );

        let ours = serde_json::to_string(&ViewRequest {
            scene: "b.pxart".to_string(),
            key: 1,
            at: 2,
            shot: true,
            sheet: false,
            ask_camera: true,
            set_camera: Some([1.0, 2.0, 3.0]),
            shot_path: Some("target/x.png".to_string()),
        })
        .expect("序列化");
        let value: serde_json::Value = serde_json::from_str(&ours).expect("解成值");
        for field in [
            "scene",
            "key",
            "at",
            "shot",
            "sheet",
            "ask_camera",
            "set_camera",
        ] {
            assert!(
                value.get(field).is_some(),
                "Bevy 要的那一格 '{field}' 一个都不能少（少了它整份请求解不开）"
            );
        }
    }

    #[test]
    fn a_place_angle_is_clamped_to_what_the_camera_will_actually_use() {
        assert_eq!(
            orbit_of("--place", [0.0, 8.0, 3.15]).unwrap(),
            [0.0, 8.0, 3.15]
        );
        assert_eq!(
            orbit_of("--place", [10.0, 120.0, 5.0]).unwrap(),
            [10.0, PITCH_DEGREES_LIMIT, 5.0]
        );
        assert_eq!(
            orbit_of("--place", [10.0, -120.0, 5.0]).unwrap(),
            [10.0, -PITCH_DEGREES_LIMIT, 5.0]
        );
        assert!(
            orbit_of("--place", [f32::NAN, 0.0, 3.0]).is_err(),
            "NaN 的 yaw 要当场拒"
        );
        assert!(
            orbit_of("--place", [0.0, 0.0, f32::INFINITY]).is_err(),
            "无穷远要当场拒"
        );
    }

    #[test]
    fn the_seed_angles_are_the_inverse_of_the_probe_pose_within_rounding() {
        let probe = crate::camera::probe_camera(None, 960.0 / 640.0);
        let position = [probe.position.x, probe.position.y, probe.position.z];
        assert_eq!(position, [0.0, 0.55, 3.15], "探针机位的位姿");

        let angles = seed_orbit(position);
        assert_eq!(angles[0], 0.0, "正前方 ⇒ yaw 0");
        assert!(
            (angles[1] - 9.9042).abs() < 0.001,
            "pitch 约 9.9°：{}",
            angles[1]
        );
        assert!(
            (angles[2] - 3.1977).abs() < 0.001,
            "distance 约 3.1977：{}",
            angles[2]
        );

        let back = crate::camera::probe_camera(Some(angles), 960.0 / 640.0);
        for (what, a, b) in [
            ("x", back.position.x, position[0]),
            ("y", back.position.y, position[1]),
            ("z", back.position.z, position[2]),
        ] {
            assert!(
                (a - b).abs() < 1e-3,
                "{what} 反推回去差了 {}：{a} vs {b}",
                (a - b).abs()
            );
        }

        assert!(seed_orbit([0.0, 0.0, 0.0]).iter().all(|v| v.is_finite()));
        assert!(seed_orbit([0.0, 0.0, 4.0])[1].abs() < 1e-4);
    }

    #[test]
    fn the_place_line_is_copy_pasteable_and_the_probe_pose_says_so() {
        assert_eq!(
            place_line(Some([35.0, 20.0, 8.0])),
            "--place 35.0000,20.0000,8.0000"
        );
        assert!(place_line(None).contains("探针机位"));
        assert!(describe_orbit(None).contains("探针机位"));
        assert!(describe_orbit(Some([1.0, 2.0, 3.0])).contains("distance 3.0000"));
    }

    #[test]
    fn the_mouse_deltas_are_radians_converted_to_the_degrees_we_store() {
        let turned = drag_degrees(YAW_PER_PIXEL, 100.0);
        assert!(
            (turned - 34.377_47).abs() < 0.001,
            "100 像素该转约 34.38°，实得 {turned}"
        );

        let limit = pitch_limit_degrees();
        assert!(
            (limit - 71.619_73).abs() < 0.001,
            "夹取该是 ±71.62°，实得 {limit}"
        );
        assert!(
            limit < PITCH_DEGREES_LIMIT,
            "鼠标的夹取比相机自己的 ±89.5° 紧（Bevy 就是这么定的），两条不是同一个数"
        );

        let mut pitch = 0.0_f32;
        pitch = (pitch + drag_degrees(PITCH_PER_PIXEL, 50.0)).clamp(-limit, limit);
        assert!(
            (pitch - 17.188_73).abs() < 0.001,
            "往下拖 50 像素该到约 17.19°，实得 {pitch}"
        );
        assert!(pitch < limit, "还没到夹取点");
    }

    #[test]
    fn the_window_asks_for_the_same_views_the_offline_path_does() {
        assert_eq!(Views::Single(None), Views::Single(None));
        assert_eq!(
            Views::Single(Some([0.0, 0.0, 3.15])),
            Views::Single(Some([0.0, 0.0, 3.15]))
        );
        assert_ne!(Views::Single(None), Views::Single(Some([0.0, 0.0, 3.15])));
    }

    #[test]
    fn the_ui_capture_reads_back_in_rgba_order() {
        let source = [10u8, 20, 30, 255, 40, 50, 60, 128];
        let mut out = Vec::new();
        swizzle_bgra_to_rgba(&source, &mut out);
        assert_eq!(out, [30, 20, 10, 255, 60, 50, 40, 128]);
        assert_eq!(out[1], source[1]);
        assert_eq!(out[3], source[3]);
        assert_eq!(out[5], source[5]);
        assert_eq!(out[7], source[7]);
    }
}
