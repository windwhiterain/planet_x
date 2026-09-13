mod atmosphere;
mod clouds;
mod planet;
mod shaders;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::camera::RenderTarget;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::Skybox;
use bevy::pbr::AtmosphereSettings;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::render::render_resource::{
    CachedPipelineState, PipelineCache, TextureFormat, TextureUsages,
};
use bevy::render::view::screenshot::{Capturing, Screenshot, save_to_disk};
use bevy::render::{Render, RenderApp};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;

use px_protocol::client;
use px_protocol::render::{Lease, Request, Response, Scene};
use px_protocol::sim::WorldView;
use px_protocol::stream::{self, Frame};
use px_protocol::ProtocolId;

const GOOD_COLORS: [Srgba; 3] = [
    Srgba::new(0.86, 0.72, 0.34, 1.0),
    Srgba::new(0.52, 0.68, 0.88, 1.0),
    Srgba::new(0.74, 0.54, 0.82, 1.0),
];

const SPACING: f32 = 3.0;
const MAX_STACK: f32 = 5.0;
const MAX_BAR: f32 = 4.0;
const PIPELINE_DEADLINE: u32 = 1800;
const FRAMES_AFTER_JOB: u32 = 6;
const LEASE_CHECK_INTERVAL: u32 = 120;
const STAR_WIDTH: u32 = 2048;
const STAR_HEIGHT: u32 = 1024;

#[derive(Component)]
pub struct ScenePart;

#[derive(Resource, Clone, Copy)]
struct InitialSize(u32, u32);

#[derive(Resource, Clone)]
struct RenderReady(Arc<AtomicBool>);

#[derive(Resource)]
pub struct Canvas {
    pub size: (u32, u32),
    pub target: Handle<Image>,
}

#[derive(Resource)]
struct Stars(Handle<Image>);

#[derive(Resource)]
struct Inbox(std::sync::Mutex<Receiver<Job>>);

#[derive(Resource, Default)]
struct Active(Option<ActiveJob>);

#[derive(Resource, Default)]
struct Ticks(u32);

#[derive(Resource)]
struct LeaseWatch {
    path: PathBuf,
    pid: u32,
}

struct Job {
    request: Request,
    reply: SyncSender<Frame>,
}

struct ActiveJob {
    label: String,
    out: PathBuf,
    width: u32,
    height: u32,
    started: Instant,
    reply: SyncSender<Frame>,
    warm: u32,
    requested: bool,
}

#[derive(Debug)]
struct Options {
    serve: bool,
    view: bool,
    show: bool,
    shot: bool,
    autostart: bool,
    port: u16,
    width: u32,
    height: u32,
    stream: PathBuf,
    round: Option<u32>,
    out: Option<PathBuf>,
    planet: Option<PathBuf>,
    mesh: Option<PathBuf>,
    clouds: Option<PathBuf>,
    cloud: Option<f32>,
    fps: bool,
    novsync: bool,
    cloud_ablate: clouds::Ablate,
    palette: planet::Palette,
    displace: Option<f32>,
    sea_level: Option<f32>,
    radius: f32,
    ambient: Option<f32>,
    cam: Option<[f32; 3]>,
    atmo: Option<f32>,
    scatter: Option<String>,
    spin: Option<f32>,
    rings: Option<f32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            serve: false,
            view: false,
            show: false,
            shot: false,
            autostart: false,
            port: 0,
            width: 960,
            height: 640,
            stream: PathBuf::from("target/world.pxstream"),
            round: None,
            out: None,
            planet: None,
            mesh: None,
            clouds: None,
            cloud: None,
            fps: false,
            novsync: false,
            cloud_ablate: clouds::Ablate::None,
            palette: planet::Palette::Rocky,
            displace: None,
            sea_level: None,
            radius: 1.0,
            ambient: None,
            cam: None,
            atmo: None,
            scatter: None,
            spin: None,
            rings: None,
        }
    }
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut options = Self::default();

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut next = |needed: &str| args.next().ok_or_else(|| format!("{needed} 需要一个值"));
            let mut number = |needed: &str| -> Result<f32, String> {
                next(needed)?
                    .parse()
                    .map_err(|_| format!("{needed} 需要一个数"))
            };
            match arg.as_str() {
                "--serve" => options.serve = true,
                "--view" => options.view = true,
                "--show" => options.show = true,
                "--shot" => options.shot = true,
                "--autostart" => options.autostart = true,
                "--stream" => options.stream = PathBuf::from(next("--stream")?),
                "--planet" => options.planet = Some(PathBuf::from(next("--planet")?)),
                "--mesh" => options.mesh = Some(PathBuf::from(next("--mesh")?)),
                "--clouds" => options.clouds = Some(PathBuf::from(next("--clouds")?)),
                "--cloud" => options.cloud = Some(number("--cloud")?),
                "--fps" => options.fps = true,
                "--novsync" => options.novsync = true,
                "--cloud-ablate" => {
                    options.cloud_ablate = clouds::Ablate::parse(&next("--cloud-ablate")?)?
                }
                "--scatter" => {
                    let kind = next("--scatter")?;
                    if kind != "earth" && kind != "none" {
                        return Err("--scatter 只认 earth / none".to_string());
                    }
                    options.scatter = if kind == "none" { None } else { Some(kind) };
                }
                "--atmo" => {
                    options.atmo = Some(
                        next("--atmo")?
                            .parse()
                            .map_err(|_| "--atmo 要一个数（0 关掉大气）".to_string())?,
                    );
                }
                "--ambient" => {
                    options.ambient = Some(
                        next("--ambient")?
                            .parse()
                            .map_err(|_| "--ambient 要一个数".to_string())?,
                    );
                }
                "--cam" => {
                    let text = next("--cam")?;
                    let parts: Vec<f32> = text
                        .split(',')
                        .map(|part| part.trim().parse::<f32>())
                        .collect::<Result<_, _>>()
                        .map_err(|_| "--cam 要 yaw,pitch,dist 三个数".to_string())?;
                    if parts.len() != 3 {
                        return Err("--cam 要 yaw,pitch,dist 三个数".to_string());
                    }
                    options.cam = Some([parts[0], parts[1], parts[2]]);
                }
                "--out" => options.out = Some(PathBuf::from(next("--out")?)),
                "--palette" => {
                    let name = next("--palette")?;
                    options.palette = planet::Palette::parse(&name).ok_or_else(|| {
                        format!(
                            "--palette 只认 {}，收到 {name}",
                            planet::Palette::NAMES.join(" / ")
                        )
                    })?;
                }
                "--displace" => options.displace = Some(number("--displace")?),
                "--sea" => options.sea_level = Some(number("--sea")?),
                "--radius" => options.radius = number("--radius")?,
                "--spin" => options.spin = Some(number("--spin")?),
                "--rings" => options.rings = Some(number("--rings")?),
                "--round" => {
                    options.round = Some(
                        next("--round")?
                            .parse()
                            .map_err(|_| "--round 需要一个整数".to_string())?,
                    );
                }
                "--port" => {
                    options.port = next("--port")?
                        .parse()
                        .map_err(|_| "--port 需要一个整数".to_string())?;
                }
                "--width" => {
                    options.width = next("--width")?
                        .parse()
                        .map_err(|_| "--width 需要一个整数".to_string())?;
                }
                "--height" => {
                    options.height = next("--height")?
                        .parse()
                        .map_err(|_| "--height 需要一个整数".to_string())?;
                }
                "--help" | "-h" => {
                    println!("{}", usage());
                    std::process::exit(0);
                }
                other => return Err(format!("未知参数：{other}\n{}", usage())),
            }
        }

        if !options.serve && options.out.is_none() {
            options.out = Some(PathBuf::from("target/shot.png"));
        }
        Ok(options)
    }

    fn planet_spec(&self) -> Option<planet::PlanetSpec> {
        self.planet.as_ref().map(|path| {
            let (displace, sea_level, rings) = self.palette.defaults();
            planet::PlanetSpec {
                field: path.display().to_string(),
                mesh: self.mesh.as_ref().map(|mesh| mesh.display().to_string()),
                clouds: self.clouds.as_ref().map(|clouds| clouds.display().to_string()),
                atmo: self.atmo.unwrap_or(1.0),
                palette: self.palette,
                displace: self.displace.unwrap_or(displace),
                sea_level: self.sea_level.unwrap_or(sea_level),
                radius: self.radius,
                spin: self.spin.unwrap_or(0.0),
                rings: self.rings.unwrap_or(rings),
                ablate: self.cloud_ablate,
            }
        })
    }

    fn scene(&self) -> Scene {
        match self.planet_spec() {
            Some(spec) => Scene::Planet {
                field: spec.field,
                mesh: spec.mesh,
                clouds: spec.clouds,
                palette: spec.palette.name().to_string(),
                displace: spec.displace,
                sea_level: spec.sea_level,
                radius: spec.radius,
                spin: spec.spin,
                rings: spec.rings,
            },
            None => Scene::World {
                stream: self.stream.display().to_string(),
                round: self.round,
            },
        }
    }
}

fn usage() -> String {
    [
        "用法：",
        "  px_render --serve [--port N] [--width W] [--height H]",
        "      常驻渲染服务：启动时预热管线，之后按请求出图（每次 ~0.3 s）",
        "  px_render --stream PATH [--round N] [--out PNG] [--width W] [--height H]",
        "      经济世界：三根部门库存柱 + 三根价格柱",
        "  px_render --planet FIELD.pxart [--palette rocky|gas|ice|lava|desert] [--displace F]",
        "            [--sea F] [--radius F] [--spin F] [--rings F] [--clouds CUBEMAP.pxart]",
        "            [--cloud F] [--out PNG] [--width W] [--height H]",
        "      程序化星球：把 PCG 烘出来的高度场当位移，按色带着色，带星空背景；--rings 给个",
        "      大于 1 的倍数就加环系；--clouds 给一张 CubeMap 覆盖度就加体积云，--cloud 是",
        "      消光倍率（默认 1）；--fps 打开帧时间读数：日志每 120 帧打一行平均帧时间，",
        "      服务端不再按 60 Hz 限速（窗口仍按 vsync，那才是用户看到的手感）；",
        "      --novsync 再把窗口的 vsync 关掉，量纯 GPU 成本",
        "  服务没在跑时会提示；要顺手拉起一个就加 --autostart",
    ]
    .join("\n")
}

fn main() {
    let options = match Options::parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(64);
        }
    };

    if options.view {
        if let Err(message) = view(options) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    } else if options.show {
        if let Err(message) = show(&options) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    } else if options.serve {
        if let Err(message) = serve(options) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    } else {
        std::process::exit(request_once(options));
    }
}

fn request_once(options: Options) -> i32 {
    let out = options
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/shot.png"));
    let request = Request {
        scene: options.scene(),
        view: px_protocol::render::View {
            ambient: options.ambient,
            cam: options.cam,
            atmo: options.atmo,
            scatter: options.scatter.clone(),
            cloud: options.cloud,
        },
        width: options.width,
        height: options.height,
        out: out.display().to_string(),
    };

    match client::request_with(request, options.autostart) {
        Ok(response) => {
            println!(
                "{} → {}（{}×{}，{} 字节，耗时 {} ms，{}）",
                response.scene,
                response.out,
                response.width,
                response.height,
                response.bytes,
                response.millis,
                if response.warm { "服务已热" } else { "服务刚起" },
            );
            0
        }
        Err(px_protocol::ClientError::NoServer) => {
            eprintln!("没有在跑的渲染服务。先起一个：");
            eprintln!("    px_render --serve");
            eprintln!("租约文件：{}", client::lease_path().display());
            1
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn serve(options: Options) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", options.port))
        .map_err(|err| format!("绑定端口失败：{err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("拿不到端口：{err}"))?
        .port();

    let id = ProtocolId::local();
    let lease_path = client::lease_path();
    let lease = Lease {
        pid: std::process::id(),
        port,
        protocol_hash: id.protocol_hash,
        git_rev: id.git_rev.clone(),
        exe: std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    };
    client::write_lease(&lease_path, &lease).map_err(|err| err.to_string())?;

    println!(
        "渲染服务已启动：端口 {port}、协议指纹 {:016x}、git {}、pid {}",
        id.protocol_hash, id.git_rev, lease.pid,
    );
    println!("租约：{}", lease_path.display());

    let (job_tx, job_rx) = std::sync::mpsc::channel::<Job>();
    spawn_listener(listener, job_tx);

    let ready = Arc::new(AtomicBool::new(false));
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.004, 0.005, 0.010)))
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: asset_root(),
                    watch_for_changes_override: Some(true),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(atmosphere::AtmospherePlugin)
        .add_plugins(clouds::CloudsPlugin)
        .add_plugins(shaders::ShaderLibraryPlugin)
        .insert_resource(FrameProbe(options.fps))
        .insert_resource(ServerAblate(options.cloud_ablate))
        .add_plugins(ScheduleRunnerPlugin::run_loop(if options.fps {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(1.0 / 60.0)
        }))
        .insert_resource(RenderReady(ready.clone()))
        .insert_resource(Inbox(std::sync::Mutex::new(job_rx)))
        .insert_resource(InitialSize(options.width, options.height))
        .init_resource::<Active>()
        .init_resource::<Ticks>()
        .insert_resource(LeaseWatch {
            path: lease_path,
            pid: lease.pid,
        })
        .add_systems(Startup, warm_up)
        .add_systems(Update, (accept_jobs, drive, watch_lease))
        .add_systems(Update, report_frame_time)
        .add_systems(Update, idle_between_jobs.after(accept_jobs).before(drive));

    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .insert_resource(RenderReady(ready.clone()))
            .add_systems(Render, watch_pipelines);
    }

    app.run();
    Ok(())
}

fn spawn_listener(listener: TcpListener, jobs: Sender<Job>) {
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut stream) = connection else {
                continue;
            };
            let jobs = jobs.clone();
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(300)));

                let local = ProtocolId::local();
                match stream::read_frame(&mut stream) {
                    Ok(Some(Frame::Protocol(remote))) => {
                        if remote.schema_version != local.schema_version
                            || remote.protocol_hash != local.protocol_hash
                        {
                            let reason = format!(
                                "协议不一致：对端 schema {} / 指纹 {:016x} / git {}，本服务 schema {} / 指纹 {:016x} / git {}",
                                remote.schema_version,
                                remote.protocol_hash,
                                remote.git_rev,
                                local.schema_version,
                                local.protocol_hash,
                                local.git_rev,
                            );
                            let _ = stream::write_frame(&mut stream, &Frame::Refused(reason.clone()));
                            eprintln!("拒绝连接：{reason}");
                            return;
                        }
                    }
                    other => {
                        let reason = format!("握手指望 Protocol 帧，收到 {other:?}");
                        let _ = stream::write_frame(&mut stream, &Frame::Refused(reason));
                        return;
                    }
                }

                if stream::write_frame(&mut stream, &Frame::Protocol(local)).is_err() {
                    return;
                }

                let request = match stream::read_frame(&mut stream) {
                    Ok(Some(Frame::Request(request))) => request,
                    Ok(other) => {
                        let reason = format!("指望 Request 帧，收到 {other:?}");
                        let _ = stream::write_frame(&mut stream, &Frame::Refused(reason));
                        return;
                    }
                    Err(err) => {
                        eprintln!("读请求失败：{err}");
                        return;
                    }
                };

                let (reply_tx, reply_rx) = sync_channel::<Frame>(1);
                if jobs
                    .send(Job {
                        request,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    return;
                }
                match reply_rx.recv_timeout(Duration::from_secs(300)) {
                    Ok(frame) => {
                        let _ = stream::write_frame(&mut stream, &frame);
                    }
                    Err(err) => {
                        let _ = stream::write_frame(
                            &mut stream,
                            &Frame::Refused(format!("渲染超时或服务已退出：{err}")),
                        );
                    }
                }
            });
        }
    });
}

fn pipeline_label(descriptor: &bevy::material::descriptor::PipelineDescriptor) -> String {
    match descriptor {
        bevy::material::descriptor::PipelineDescriptor::RenderPipelineDescriptor(render) => render
            .label
            .as_deref()
            .unwrap_or("<无名渲染管线>")
            .to_string(),
        bevy::material::descriptor::PipelineDescriptor::ComputePipelineDescriptor(compute) => compute
            .label
            .as_deref()
            .unwrap_or("<无名计算管线>")
            .to_string(),
    }
}

fn watch_pipelines(
    cache: Res<PipelineCache>,
    ready: Res<RenderReady>,
    mut announced: Local<bool>,
    mut reported: Local<Vec<String>>,
) {
    if ready.0.load(Ordering::Relaxed) {
        return;
    }
    let total = cache.pipelines().count();
    let mut pending = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for pipeline in cache.pipelines() {
        match &pipeline.state {
            CachedPipelineState::Ok(_) => {}
            CachedPipelineState::Err(err) => {
                failures.push(format!("{}｜{}", pipeline_label(&pipeline.descriptor), err));
            }
            _ => pending += 1,
        }
    }
    if total > 0 && !*announced {
        *announced = true;
        println!("首个渲染管线入队：当前 {total} 条，待编译 {pending} 条");
    }
    if total == 0 || pending > 0 {
        return;
    }
    if failures != *reported {
        for line in &failures {
            eprintln!("管线编译失败｜{line}");
        }
        *reported = failures.clone();
    }
    if failures.is_empty() {
        ready.0.store(true, Ordering::Relaxed);
        println!("渲染管线全部就绪：共 {total} 条，失败 0 条");
    }
}

fn watch_lease(watch: Res<LeaseWatch>, mut ticks: ResMut<Ticks>, mut exit: MessageWriter<AppExit>) {
    ticks.0 += 1;
    if ticks.0 % LEASE_CHECK_INTERVAL != 0 {
        return;
    }
    match client::read_lease(&watch.path) {
        Some(lease) if lease.pid == watch.pid => {}
        _ => {
            println!("租约已易主，渲染服务退出");
            exit.write(AppExit::Success);
        }
    }
}

fn new_target(images: &mut Assets<Image>, width: u32, height: u32) -> Handle<Image> {
    let mut target = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    images.add(target)
}

pub(crate) fn asset_root() -> String {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("px_render/assets"));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cursor = exe.parent().map(|path| path.to_path_buf());
        while let Some(directory) = cursor {
            candidates.push(directory.join("px_render/assets"));
            cursor = directory.parent().map(|path| path.to_path_buf());
        }
    }
    candidates
        .into_iter()
        .find(|path| path.join("shaders/atmosphere.wgsl").exists())
        .unwrap_or_else(|| std::path::PathBuf::from("px_render/assets"))
        .to_string_lossy()
        .to_string()
}
fn warm_up(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut atmosphere_materials: ResMut<Assets<atmosphere::AtmosphereMaterial>>,
    mut cloud_materials: ResMut<Assets<clouds::CloudsMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    size: Res<InitialSize>,
    existing: Option<Res<Canvas>>,
) {
    if existing.is_some() {
        return;
    }
    let handle = new_target(&mut images, size.0, size.1);
    commands.insert_resource(Canvas {
        size: (size.0, size.1),
        target: handle.clone(),
    });
    commands.insert_resource(Stars(images.add(planet::star_cube(512))));

    commands.spawn((
        ScenePart,
        Camera3d::default(),
        DepthPrepass,
        Msaa::Off,
        RenderTarget::Image(handle.into()),
        Transform::from_xyz(0.0, 6.0, 16.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        ScenePart,
        AmbientLight {
            brightness: 200.0,
            ..default()
        },
    ));
    commands.spawn((
        ScenePart,
        DirectionalLight {
            illuminance: 9000.0,
            ..default()
        },
        Transform::from_xyz(7.0, 13.0, 9.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        ScenePart,
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.5, 0.5),
            ..default()
        })),
        Transform::from_xyz(0.0, 1.0, 0.0),
    ));
    planet::warm_atmosphere(&mut commands, &mut meshes, &mut atmosphere_materials);
    clouds::warm_clouds(&mut commands, &mut meshes, &mut cloud_materials);
}

fn accept_jobs(
    mut commands: Commands,
    inbox: Res<Inbox>,
    ablate: Res<ServerAblate>,
    mut active: ResMut<Active>,
    mut canvas: ResMut<Canvas>,
    stars: Res<Stars>,
    mut images: ResMut<Assets<Image>>,
    mut atmo_materials: ResMut<Assets<atmosphere::AtmosphereMaterial>>,
    mut cloud_materials: ResMut<Assets<clouds::CloudsMaterial>>,
    mut media: ResMut<Assets<bevy::light::atmosphere::ScatteringMedium>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    parts: Query<Entity, With<ScenePart>>,
) {
    if active.0.is_some() {
        return;
    }
    let job = match inbox.0.lock().expect("收件箱锁坏了").try_recv() {
        Ok(job) => job,
        Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
    };
    let request = &job.request;

    if canvas.size != (request.width, request.height) {
        let handle = new_target(&mut images, request.width, request.height);
        canvas.size = (request.width, request.height);
        canvas.target = handle;
    }

    for entity in parts.iter() {
        commands.entity(entity).despawn();
    }

    let built = match &request.scene {
        Scene::World { stream, round } => build_world_scene(
            &mut commands,
            &mut meshes,
            &mut materials,
            &canvas.target,
            stream,
            *round,
        ),
        Scene::Planet {
            field,
            mesh,
            clouds,
            palette,
            displace,
            sea_level,
            radius,
            spin,
            rings,
        } => {
            let Some(palette) = planet::Palette::parse(palette) else {
                let _ = job.reply.send(Frame::Refused(format!(
                    "不认识的色板 {palette}；可用：{}",
                    planet::Palette::NAMES.join(" / ")
                )));
                return;
            };
            let atmo = request.view.atmo;
            let camera_transform = planet::probe_camera(request.view.cam);
            planet::spawn_planet(
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut images,
                &stars.0,
                &mut atmo_materials,
                &mut media,
                &mut cloud_materials,
                request.view.scatter.as_deref(),
                camera_transform,
                &planet::PlanetSpec {
                    field: field.clone(),
                    mesh: mesh.clone(),
                    clouds: clouds.clone(),
                    atmo: atmo.unwrap_or(1.0),
                    palette,
                    displace: *displace,
                    sea_level: *sea_level,
                    radius: *radius,
                    spin: *spin,
                    rings: *rings,
                    ablate: ablate.0,
                },
                request.view.cloud.unwrap_or(1.0) * CLOUD_EXTINCTION,
            )
        }
    };

    let label = match built {
        Ok(label) => label,
        Err(err) => {
            let _ = job.reply.send(Frame::Refused(err));
            return;
        }
    };

    let scattering = request.view.scatter.is_some();
    let camera_transform = planet::probe_camera(request.view.cam);
    let camera = commands.spawn((
        ScenePart,
        atmosphere::RequestCamera,
        Camera3d::default(),
        DepthPrepass,
        Msaa::Off,
        RenderTarget::Image(canvas.target.clone().into()),
        Skybox {
            image: Some(stars.0.clone()),
            brightness: SKY_BRIGHTNESS,
            rotation: Quat::IDENTITY,
        },
        AmbientLight {
            brightness: request.view.ambient.unwrap_or(DEFAULT_AMBIENT),
            ..default()
        },
        camera_transform,
    )).id();
    if scattering {
        commands.entity(camera).insert(AtmosphereSettings::default());
    }

    active.0 = Some(ActiveJob {
        label,
        out: PathBuf::from(&request.out),
        width: request.width,
        height: request.height,
        started: Instant::now(),
        reply: job.reply,
        warm: 0,
        requested: false,
    });
}

fn build_world_scene(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    target: &Handle<Image>,
    stream_path: &str,
    round: Option<u32>,
) -> Result<String, String> {
    let path = PathBuf::from(stream_path);
    let worlds = stream::read_worlds(&path)
        .map_err(|err| format!("读 {} 失败：{err}", path.display()))?;
    if worlds.is_empty() {
        return Err(format!("{} 里没有 World 帧", path.display()));
    }
    if let Some(declared) = stream::declared_protocol(&path).map_err(|err| err.to_string())? {
        let local = ProtocolId::local();
        if declared.protocol_hash != local.protocol_hash {
            return Err(format!(
                "{} 是 {:016x}（git {}）录的，本服务是 {:016x}；用同一份代码重录",
                path.display(),
                declared.protocol_hash,
                declared.git_rev,
                local.protocol_hash,
            ));
        }
    }

    let world: WorldView = match round {
        Some(round) => worlds
            .iter()
            .find(|world| world.round == round)
            .cloned()
            .ok_or_else(|| format!("{} 里没有第 {round} 轮", path.display()))?,
        None => worlds.last().cloned().unwrap(),
    };

    commands.spawn((
        ScenePart,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(28.0, 28.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.10, 0.12, 0.17),
            perceptual_roughness: 0.9,
            ..default()
        })),
    ));

    let departments = world.departments.len().max(1);
    let peak_stack = world
        .departments
        .iter()
        .map(|department| {
            department
                .holdings
                .iter()
                .map(|holding| holding.max(0.0))
                .sum::<f32>()
        })
        .fold(0.0_f32, f32::max)
        .max(1e-3);

    for (index, department) in world.departments.iter().enumerate() {
        let x = (index as f32 - (departments as f32 - 1.0) / 2.0) * SPACING;
        let mut level = 0.0_f32;
        for (good, holding) in department.holdings.iter().enumerate() {
            let height = (holding.max(0.0) / peak_stack * MAX_STACK).max(0.04);
            commands.spawn((
                ScenePart,
                Mesh3d(meshes.add(Cuboid::new(1.5, height, 1.5))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::Srgba(GOOD_COLORS[good % GOOD_COLORS.len()]),
                    perceptual_roughness: 0.5,
                    ..default()
                })),
                Transform::from_xyz(x, level + height / 2.0, -2.0),
            ));
            level += height;
        }
    }

    let goods = world.goods.len().max(1);
    let peak_price = world
        .goods
        .iter()
        .map(|good| good.price.max(0.0))
        .fold(0.0_f32, f32::max)
        .max(1e-3);

    for (good, view) in world.goods.iter().enumerate() {
        let x = (good as f32 - (goods as f32 - 1.0) / 2.0) * SPACING;
        let height = (view.price.max(0.0) / peak_price * MAX_BAR).max(0.04);
        commands.spawn((
            ScenePart,
            Mesh3d(meshes.add(Cuboid::new(0.7, height, 0.7))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::Srgba(GOOD_COLORS[good % GOOD_COLORS.len()]),
                perceptual_roughness: 0.25,
                metallic: 0.6,
                ..default()
            })),
            Transform::from_xyz(x, height / 2.0, 3.0),
        ));
    }

    commands.spawn((
        ScenePart,
        AmbientLight {
            brightness: 200.0,
            ..default()
        },
    ));
    commands.spawn((
        ScenePart,
        DirectionalLight {
            illuminance: 9000.0,
            ..default()
        },
        Transform::from_xyz(7.0, 13.0, 9.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        ScenePart,
        Camera3d::default(),
        DepthPrepass,
        Msaa::Off,
        RenderTarget::Image(target.clone().into()),
        Transform::from_xyz(0.0, 6.0, 16.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));

    Ok(format!("第 {} 轮", world.round))
}

#[derive(Resource)]
struct FrameProbe(bool);

#[derive(Resource, Clone, Copy)]
struct ServerAblate(clouds::Ablate);

fn report_frame_time(
    probe: Res<FrameProbe>,
    mut since: Local<Option<Instant>>,
    mut frames: Local<u32>,
) {
    if !probe.0 {
        return;
    }
    let now = Instant::now();
    let Some(previous) = *since else {
        *since = Some(now);
        return;
    };
    *frames += 1;
    if *frames < FRAME_PROBE_WINDOW {
        return;
    }
    let elapsed = now.duration_since(previous);
    let count = f64::from(*frames);
    println!(
        "帧时间 {:.2} ms（{:.1} fps，{} 帧平均）",
        elapsed.as_secs_f64() * 1000.0 / count,
        count / elapsed.as_secs_f64(),
        *frames,
    );
    *since = Some(now);
    *frames = 0;
}

#[derive(Component)]
struct FpsReadout;

#[derive(Resource)]
struct ShowFps(bool);

fn spawn_fps_readout(mut commands: Commands) {
    commands.spawn((
        FpsReadout,
        Text::new("fps --"),
        TextFont {
            font_size: bevy::text::FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgb(0.92, 0.96, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(10.0),
            padding: UiRect::all(Val::Px(5.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
    ));
}

fn update_fps_readout(
    diagnostics: Res<DiagnosticsStore>,
    mut frames: Local<u32>,
    mut slowest: Local<f32>,
    mut readout: Query<(&mut Text, &mut Visibility), With<FpsReadout>>,
) {
    let Ok((mut text, mut visibility)) = readout.single_mut() else {
        return;
    };
    if *visibility == Visibility::Hidden {
        return;
    }
    *frames += 1;
    let Some(millis) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|diagnostic| diagnostic.value())
    else {
        return;
    };
    let millis = millis as f32;
    *slowest = slowest.max(millis);
    if *frames % FPS_REFRESH_FRAMES != 0 {
        return;
    }
    text.0 = format!(
        "{:.1} fps  {:.1} ms  worst {:.1} ms",
        1000.0 / millis.max(1e-3),
        millis,
        *slowest,
    );
    if *frames % FPS_WORST_FRAMES == 0 {
        *slowest = 0.0;
    }
}

fn idle_between_jobs(
    probe: Res<FrameProbe>,
    ready: Res<RenderReady>,
    active: Res<Active>,
    mut cameras: Query<&mut Camera, With<Camera3d>>,
) {
    let wanted = probe.0 || active.0.is_some() || !ready.0.load(Ordering::Relaxed);
    for mut camera in cameras.iter_mut() {
        if camera.is_active != wanted {
            camera.is_active = wanted;
        }
    }
}

fn drive(
    mut commands: Commands,
    mut active: ResMut<Active>,
    canvas: Res<Canvas>,
    ready: Res<RenderReady>,
    capturing: Query<Entity, With<Capturing>>,
    ticks: Res<Ticks>,
) {
    let Some(job) = active.0.as_mut() else {
        return;
    };

    if !ready.0.load(Ordering::Relaxed) && ticks.0 < PIPELINE_DEADLINE {
        return;
    }

    if !job.requested {
        job.warm += 1;
        if job.warm >= FRAMES_AFTER_JOB {
            commands
                .spawn(Screenshot::image(canvas.target.clone()))
                .observe(save_to_disk(job.out.clone()));
            job.requested = true;
        }
        return;
    }

    if !capturing.is_empty() {
        return;
    }

    let response = Response {
        out: job.out.display().to_string(),
        scene: job.label.clone(),
        width: job.width,
        height: job.height,
        bytes: std::fs::metadata(&job.out).map(|meta| meta.len()).unwrap_or(0),
        millis: job.started.elapsed().as_millis() as u64,
        warm: ready.0.load(Ordering::Relaxed),
    };
    println!(
        "出图：{} → {}（{}×{}，{} 字节，{} ms）",
        response.scene, response.out, response.width, response.height, response.bytes, response.millis,
    );
    let _ = job.reply.send(Frame::Response(response));
    active.0 = None;
}

const DEFAULT_AMBIENT: f32 = 80.0;
const SKY_BRIGHTNESS: f32 = 900.0;
const CLOUD_EXTINCTION: f32 = 900.0;
const FRAME_PROBE_WINDOW: u32 = 120;
const FPS_REFRESH_FRAMES: u32 = 10;
const FPS_WORST_FRAMES: u32 = 120;

const VIEW_REQUEST: &str = "target/viewer-scene.json";
const VIEW_LEASE: &str = "target/viewer.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ViewRequest {
    scene: Scene,
    at: u64,
    #[serde(default)]
    shot: bool,
}

#[derive(Resource)]
struct Viewer {
    spec: planet::PlanetSpec,
    field_modified: Option<std::time::SystemTime>,
    request_at: u64,
    spin: bool,
}

#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

#[derive(Resource)]
struct Rebuild(bool);

#[derive(Resource, Default)]
struct PendingShot(Option<u32>);

#[derive(Component)]
struct OrbitCamera;

fn keep_rendering(
    error: &bevy::render::error_handler::RenderError,
    _main: &mut World,
    _render: &mut World,
) -> bevy::render::error_handler::RenderErrorPolicy {
    eprintln!("预览窗口遇到渲染错误，选择继续（不退出）：{:?}", error.ty);
    bevy::render::error_handler::RenderErrorPolicy::Ignore
}

fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0)
}

fn describe(spec: &planet::PlanetSpec) -> String {
    format!(
        "{}｜位移 {:.3}｜海平面 {:.2}{}",
        spec.palette.name(),
        spec.displace,
        spec.sea_level,
        if spec.rings > 0.0 {
            format!("｜环 ×{:.2}", spec.rings)
        } else {
            String::new()
        },
    )
}

fn lease_age() -> Option<Duration> {
    let stamp: u64 = std::fs::read_to_string(VIEW_LEASE).ok()?.trim().parse().ok()?;
    Some(Duration::from_nanos(now_nanos().saturating_sub(stamp)))
}

fn show(options: &Options) -> Result<(), String> {
    let scene = options.scene();
    if matches!(scene, Scene::World { .. }) {
        return Err("--show 目前只支持星球场景（--planet）".to_string());
    }
    let request = ViewRequest {
        scene,
        at: now_nanos(),
        shot: options.shot,
    };
    let text = serde_json::to_string_pretty(&request).map_err(|err| err.to_string())?;
    std::fs::write(VIEW_REQUEST, text).map_err(|err| format!("写 {VIEW_REQUEST} 失败：{err}"))?;

    let note = match options.planet_spec() {
        Some(spec) => describe(&spec),
        None => String::from("（空）"),
    };
    println!("已推给常驻窗口：{note}");
    match lease_age() {
        Some(age) if age < Duration::from_secs(5) => {
            println!("窗口在线（心跳 {:.1} s 前）", age.as_secs_f32());
        }
        _ => println!("⚠ 没检测到在跑的窗口；先执行 `px_render --view` 开一个，它会一直留着"),
    }
    Ok(())
}

fn read_view_request() -> Option<(planet::PlanetSpec, u64, bool)> {
    let text = std::fs::read_to_string(VIEW_REQUEST).ok()?;
    let request: ViewRequest = serde_json::from_str(&text).ok()?;
    match request.scene {
        Scene::Planet {
            field,
            mesh,
            clouds,
            palette,
            displace,
            sea_level,
            radius,
            spin,
            rings,
        } => Some((
            planet::PlanetSpec {
                field,
                mesh,
                clouds,
                atmo: 1.0,
                palette: planet::Palette::parse(&palette)?,
                displace,
                sea_level,
                radius,
                spin,
                rings,
                ablate: clouds::Ablate::None,
            },
            request.at,
            request.shot,
        )),
        Scene::World { .. } => None,
    }
}

fn view(options: Options) -> Result<(), String> {
    let (spec, request_at, shot) = if options.planet.is_some() {
        let seen = std::fs::read_to_string(VIEW_REQUEST)
            .ok()
            .and_then(|text| serde_json::from_str::<ViewRequest>(&text).ok())
            .map(|request| request.at)
            .unwrap_or(0);
        (
            options
                .planet_spec()
                .ok_or_else(|| "命令行给的星球场景不完整".to_string())?,
            seen,
            options.shot,
        )
    } else {
        match read_view_request() {
            Some(found) => found,
            None => {
                return Err(
                    "--view 需要一个起始场景：给 --planet <FIELD.pxart>，或先用 --show 推一个"
                        .to_string(),
                );
            }
        }
    };
    let field_modified = std::fs::metadata(&spec.field)
        .and_then(|meta| meta.modified())
        .ok();
    let spin = options.spin.is_none();
    let ready = Arc::new(AtomicBool::new(false));

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(AssetPlugin {
        file_path: asset_root(),
        ..default()
    }).set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("px_render 预览 — {}", describe(&spec)),
            resolution: (1280_u32, 800_u32).into(),
            present_mode: if options.novsync {
                bevy::window::PresentMode::AutoNoVsync
            } else {
                bevy::window::PresentMode::AutoVsync
            },
            ..default()
        }),
        ..default()
    }))
    .add_plugins(atmosphere::AtmospherePlugin)
    .add_plugins(clouds::CloudsPlugin)
    .add_plugins(shaders::ShaderLibraryPlugin)
    .insert_resource(ClearColor(Color::srgb(0.004, 0.005, 0.010)))
    .insert_resource(Viewer {
        spec,
        field_modified,
        request_at,
        spin,
    })
    .insert_resource(Orbit {
        yaw: 0.0,
        pitch: 0.17,
        distance: 3.2,
    })
    .insert_resource(Rebuild(true))
    .insert_resource(PendingShot(shot.then_some(12)))
    .insert_resource(FrameProbe(options.fps))
    .insert_resource(ShowFps(true))
    .add_plugins(FrameTimeDiagnosticsPlugin::default())
    .insert_resource(RenderReady(ready.clone()))
    .insert_resource(bevy::render::error_handler::RenderErrorHandler(
        keep_rendering,
    ))
    .add_systems(Startup, (view_startup, spawn_fps_readout))
    .add_systems(
        Update,
        (
            orbit_camera,
            viewer_keys,
            poll_request,
            poll_field,
            rebuild_scene,
            spin_bodies,
            auto_shot,
            heartbeat,
            update_title,
            report_frame_time,
            update_fps_readout,
        )
            .chain(),
    );

    println!("预览窗口已开：左键拖动转视角、滚轮缩放");
    println!("  1-5 换色板｜[ ] 调海平面｜- = 调位移｜r 切环系｜空格 自转｜f 帧率开关｜s 存图｜q 退出");
    println!("  我把新效果推进来：px_render --show --planet <文件> --palette <色板> [--shot]");

    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .insert_resource(RenderReady(ready.clone()))
            .add_systems(Render, watch_pipelines);
    }

    app.run();
    let _ = std::fs::remove_file(VIEW_LEASE);
    Ok(())
}

fn view_startup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let stars = images.add(planet::star_cube(512));
    commands.insert_resource(Stars(stars.clone()));
    commands.spawn((
        OrbitCamera,
        Camera3d::default(),
        DepthPrepass,
        Msaa::Off,
        Skybox {
            image: Some(stars),
            brightness: SKY_BRIGHTNESS,
            rotation: Quat::IDENTITY,
        },
        AmbientLight {
            brightness: DEFAULT_AMBIENT,
            ..default()
        },
        Transform::from_xyz(0.0, 0.55, 3.2).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<Orbit>,
    mut camera: Query<&mut Transform, With<OrbitCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch = (orbit.pitch - motion.delta.y * 0.006).clamp(-1.25, 1.25);
    }
    if scroll.delta.y.abs() > 0.0 {
        orbit.distance = (orbit.distance * (1.0 - scroll.delta.y * 0.08)).clamp(1.5, 14.0);
    }
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let rotation = Quat::from_rotation_y(orbit.yaw) * Quat::from_rotation_x(orbit.pitch);
    transform.translation = rotation * Vec3::new(0.0, 0.0, orbit.distance);
    transform.look_at(Vec3::ZERO, Vec3::Y);
}

fn spin_bodies(
    viewer: Res<Viewer>,
    time: Res<Time>,
    mut bodies: Query<
        &mut Transform,
        Or<(
            With<planet::PlanetBody>,
            With<planet::PlanetRing>,
            With<clouds::PlanetCloud>,
        )>,
    >,
) {
    if !viewer.spin {
        return;
    }
    let step = time.delta_secs() * 0.12;
    for mut transform in bodies.iter_mut() {
        transform.rotate_y(step);
    }
}

fn viewer_keys(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut viewer: ResMut<Viewer>,
    mut rebuild: ResMut<Rebuild>,
    mut show_fps: ResMut<ShowFps>,
    readout: Query<Entity, With<FpsReadout>>,
    mut exit: MessageWriter<AppExit>,
) {
    let chosen = if keys.just_pressed(KeyCode::Digit1) {
        Some(planet::Palette::Rocky)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(planet::Palette::Gas)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(planet::Palette::Ice)
    } else if keys.just_pressed(KeyCode::Digit4) {
        Some(planet::Palette::Lava)
    } else if keys.just_pressed(KeyCode::Digit5) {
        Some(planet::Palette::Desert)
    } else {
        None
    };

    let mut changed = false;
    if let Some(palette) = chosen {
        let (displace, sea_level, rings) = palette.defaults();
        viewer.spec.palette = palette;
        viewer.spec.displace = displace;
        viewer.spec.sea_level = sea_level;
        viewer.spec.rings = rings;
        changed = true;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        viewer.spec.sea_level = (viewer.spec.sea_level - 0.02).clamp(0.05, 0.95);
        changed = true;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        viewer.spec.sea_level = (viewer.spec.sea_level + 0.02).clamp(0.05, 0.95);
        changed = true;
    }
    if keys.just_pressed(KeyCode::Minus) {
        viewer.spec.displace = (viewer.spec.displace - 0.01).max(0.0);
        changed = true;
    }
    if keys.just_pressed(KeyCode::Equal) {
        viewer.spec.displace = (viewer.spec.displace + 0.01).min(0.40);
        changed = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        viewer.spec.rings = if viewer.spec.rings > 0.0 { 0.0 } else { 2.35 };
        changed = true;
    }
    if keys.just_pressed(KeyCode::Space) {
        viewer.spin = !viewer.spin;
    }
    if keys.just_pressed(KeyCode::KeyF) {
        show_fps.0 = !show_fps.0;
        for entity in readout.iter() {
            commands.entity(entity).insert(if show_fps.0 {
                Visibility::Visible
            } else {
                Visibility::Hidden
            });
        }
    }
    if keys.just_pressed(KeyCode::KeyQ) || keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if changed {
        rebuild.0 = true;
    }
}

fn poll_request(
    mut viewer: ResMut<Viewer>,
    mut rebuild: ResMut<Rebuild>,
    mut shot: ResMut<PendingShot>,
) {
    let Some((spec, at, wants_shot)) = read_view_request() else {
        return;
    };
    if at == viewer.request_at {
        return;
    }
    viewer.request_at = at;
    viewer.field_modified = std::fs::metadata(&spec.field)
        .and_then(|meta| meta.modified())
        .ok();
    viewer.spec = spec;
    rebuild.0 = true;
    if wants_shot {
        shot.0 = Some(12);
    }
    println!("窗口切到：{}", describe(&viewer.spec));
}

fn auto_shot(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    ready: Res<RenderReady>,
    mut pending: ResMut<PendingShot>,
) {
    if keys.just_pressed(KeyCode::KeyS) {
        pending.0 = Some(3);
    }
    let Some(frames) = pending.0 else {
        return;
    };
    if frames > 0 {
        pending.0 = Some(frames - 1);
        return;
    }
    if !ready.0.load(Ordering::Relaxed) {
        return;
    }

    pending.0 = None;
    let path = "target/viewer-shot.png";
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    println!("预览截图 → {path}");
}

fn poll_field(mut viewer: ResMut<Viewer>, mut rebuild: ResMut<Rebuild>, mut ticks: Local<u32>) {
    *ticks += 1;
    if *ticks % 20 != 0 {
        return;
    }
    let Ok(meta) = std::fs::metadata(&viewer.spec.field) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    if viewer.field_modified != Some(modified) {
        viewer.field_modified = Some(modified);
        rebuild.0 = true;
        println!("场文件更新，重载：{}", viewer.spec.field);
    }
}

fn rebuild_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut atmo_materials: ResMut<Assets<atmosphere::AtmosphereMaterial>>,
    mut cloud_materials: ResMut<Assets<clouds::CloudsMaterial>>,
    mut media: ResMut<Assets<bevy::light::atmosphere::ScatteringMedium>>,
    stars: Res<Stars>,
    viewer: Res<Viewer>,
    mut rebuild: ResMut<Rebuild>,
    parts: Query<Entity, With<ScenePart>>,
) {
    if !rebuild.0 {
        return;
    }
    rebuild.0 = false;

    if let Err(message) = planet::check_scene(&viewer.spec) {
        eprintln!("⚠ 新场景读不出来，保留窗口里现在这张图：{message}");
        return;
    }

    for entity in parts.iter() {
        commands.entity(entity).despawn();
    }
    match planet::spawn_planet(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &stars.0,
        &mut atmo_materials,
        &mut media,
        &mut cloud_materials,
        None,
        Transform::from_xyz(0.0, 0.55, 3.2).looking_at(Vec3::ZERO, Vec3::Y),
        &viewer.spec,
        CLOUD_EXTINCTION,
    ) {
        Ok(label) => println!("{label}"),
        Err(message) => eprintln!("重建场景失败：{message}"),
    }
}

fn heartbeat(mut ticks: Local<u32>) {
    *ticks += 1;
    if *ticks % 30 != 0 {
        return;
    }
    let _ = std::fs::write(VIEW_LEASE, format!("{}", now_nanos()));
}

fn update_title(viewer: Res<Viewer>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let wanted = format!("px_render 预览 — {}", describe(&viewer.spec));
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if window.title != wanted {
        window.title = wanted;
    }
}










































