mod planet;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::camera::RenderTarget;
use bevy::prelude::*;
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
struct Canvas {
    size: (u32, u32),
    target: Handle<Image>,
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
    autostart: bool,
    port: u16,
    width: u32,
    height: u32,
    stream: PathBuf,
    round: Option<u32>,
    out: Option<PathBuf>,
    planet: Option<PathBuf>,
    palette: planet::Palette,
    displace: Option<f32>,
    sea_level: Option<f32>,
    radius: f32,
    spin: Option<f32>,
    rings: Option<f32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            serve: false,
            autostart: false,
            port: 0,
            width: 960,
            height: 640,
            stream: PathBuf::from("target/world.pxstream"),
            round: None,
            out: None,
            planet: None,
            palette: planet::Palette::Rocky,
            displace: None,
            sea_level: None,
            radius: 1.0,
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
                "--autostart" => options.autostart = true,
                "--stream" => options.stream = PathBuf::from(next("--stream")?),
                "--planet" => options.planet = Some(PathBuf::from(next("--planet")?)),
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

    fn scene(&self) -> Scene {
        match &self.planet {
            Some(path) => {
                let (displace, sea_level, rings) = match self.palette {
                    planet::Palette::Rocky => (0.075, 0.520, 0.0),
                    planet::Palette::Gas => (0.010, 0.450, 2.35),
                    planet::Palette::Ice => (0.055, 0.500, 0.0),
                    planet::Palette::Lava => (0.095, 0.480, 0.0),
                    planet::Palette::Desert => (0.085, 0.520, 0.0),
                };
                Scene::Planet {
                    field: path.display().to_string(),
                    palette: self.palette.name().to_string(),
                    displace: self.displace.unwrap_or(displace),
                    sea_level: self.sea_level.unwrap_or(sea_level),
                    radius: self.radius,
                    spin: self.spin.unwrap_or(0.0),
                    rings: self.rings.unwrap_or(rings),
                }
            }
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
        "            [--sea F] [--radius F] [--spin F] [--rings F] [--out PNG] [--width W] [--height H]",
        "      程序化星球：把 PCG 烘出来的高度场当位移，按色带着色，带星空背景；--rings 给个",
        "      大于 1 的倍数就加环系",
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

    if options.serve {
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
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
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
        .add_systems(Update, (accept_jobs, drive, watch_lease));

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

fn watch_pipelines(cache: Res<PipelineCache>, ready: Res<RenderReady>, mut announced: Local<bool>) {
    if ready.0.load(Ordering::Relaxed) {
        return;
    }
    let total = cache.pipelines().count();
    let mut pending = 0usize;
    let mut failed = 0usize;
    for pipeline in cache.pipelines() {
        match pipeline.state {
            CachedPipelineState::Ok(_) => {}
            CachedPipelineState::Err(_) => failed += 1,
            _ => pending += 1,
        }
    }
    if total > 0 && !*announced {
        *announced = true;
        println!("首个渲染管线入队：当前 {total} 条，待编译 {pending} 条");
    }
    if total > 0 && pending == 0 {
        ready.0.store(true, Ordering::Relaxed);
        println!("渲染管线全部就绪：共 {total} 条，失败 {failed} 条");
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

fn warm_up(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
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
    commands.insert_resource(Stars(images.add(planet::star_image(STAR_WIDTH, STAR_HEIGHT))));

    commands.spawn((
        ScenePart,
        Camera3d::default(),
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
}

fn accept_jobs(
    mut commands: Commands,
    inbox: Res<Inbox>,
    mut active: ResMut<Active>,
    mut canvas: ResMut<Canvas>,
    stars: Res<Stars>,
    mut images: ResMut<Assets<Image>>,
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
            planet::spawn_planet(
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut images,
                &canvas.target,
                &stars.0,
                &planet::PlanetSpec {
                    field: field.clone(),
                    palette,
                    displace: *displace,
                    sea_level: *sea_level,
                    radius: *radius,
                    spin: *spin,
                    rings: *rings,
                },
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
        Msaa::Off,
        RenderTarget::Image(target.clone().into()),
        Transform::from_xyz(0.0, 6.0, 16.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));

    Ok(format!("第 {} 轮", world.round))
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
