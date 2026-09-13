use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::camera::RenderTarget;
use bevy::camera::visibility::ViewVisibility;
use bevy::prelude::*;
use bevy::render::render_resource::{
    CachedPipelineState, PipelineCache, TextureFormat, TextureUsages,
};
use bevy::render::view::screenshot::{Capturing, Screenshot, save_to_disk};
use bevy::render::{Render, RenderApp};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;

use px_protocol::sim::WorldView;
use px_protocol::stream::{self, Frame};

const GOOD_COLORS: [Srgba; 3] = [
    Srgba::new(0.86, 0.72, 0.34, 1.0),
    Srgba::new(0.52, 0.68, 0.88, 1.0),
    Srgba::new(0.74, 0.54, 0.82, 1.0),
];

const SPACING: f32 = 3.0;
const MAX_STACK: f32 = 5.0;
const MAX_BAR: f32 = 4.0;
const PIPELINE_DEADLINE: u32 = 1800;

#[derive(Resource, Clone)]
struct RenderReady(Arc<AtomicBool>);

fn watch_pipelines(
    cache: Res<PipelineCache>,
    ready: Res<RenderReady>,
    mut announced: Local<bool>,
    mut peak: Local<usize>,
) {
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
    if total > *peak {
        *peak = total;
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

#[derive(Resource)]
struct Setup {
    world: WorldView,
    out: Option<PathBuf>,
    capture_at_tick: u32,
    width: u32,
    height: u32,
}

#[derive(Resource)]
struct Target(Handle<Image>);

#[derive(Resource, Default)]
struct Ticks(u32);

struct Options {
    stream: PathBuf,
    round: Option<u32>,
    out: Option<PathBuf>,
    ticks: u32,
    width: u32,
    height: u32,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut options = Self {
            stream: PathBuf::from("target/world.pxstream"),
            round: None,
            out: None,
            ticks: 4,
            width: 960,
            height: 640,
        };

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut next = |needed: &str| args.next().ok_or_else(|| format!("{needed} 需要一个值"));
            match arg.as_str() {
                "--stream" => options.stream = PathBuf::from(next("--stream")?),
                "--round" => {
                    options.round = Some(
                        next("--round")?
                            .parse()
                            .map_err(|_| "--round 需要一个整数".to_string())?,
                    );
                }
                "--out" => options.out = Some(PathBuf::from(next("--out")?)),
                "--ticks" => {
                    options.ticks = next("--ticks")?
                        .parse()
                        .map_err(|_| "--ticks 需要一个整数".to_string())?;
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

        Ok(options)
    }

    fn world(&self) -> WorldView {
        let bytes = std::fs::read(&self.stream)
            .unwrap_or_else(|err| panic!("读不到 {}：{err}", self.stream.display()));
        let frames = stream::read_stream(&mut bytes.as_slice())
            .unwrap_or_else(|err| panic!("解析 {} 失败：{err}", self.stream.display()));

        if let Some(declared) = frames.iter().find_map(|frame| match frame {
            Frame::Protocol(id) => Some(id.clone()),
            _ => None,
        }) {
            let local = px_protocol::ProtocolId::local();
            if declared.protocol_hash != local.protocol_hash {
                panic!(
                    "协议指纹不匹配：流是 {:016x}（git {}），本进程是 {:016x}",
                    declared.protocol_hash,
                    declared.git_rev,
                    local.protocol_hash,
                );
            }
        }

        let worlds: Vec<WorldView> = frames
            .into_iter()
            .filter_map(|frame| match frame {
                Frame::World(world) => Some(world),
                _ => None,
            })
            .collect();

        assert!(!worlds.is_empty(), "流里没有 World 帧");
        match self.round {
            Some(round) => worlds
                .into_iter()
                .find(|world| world.round == round)
                .unwrap_or_else(|| panic!("流里没有第 {round} 轮")),
            None => worlds.last().cloned().unwrap(),
        }
    }
}

fn usage() -> String {
    [
        "用法：px_render [选项]",
        "  --stream PATH   .pxstream 录制文件（默认 target/world.pxstream）",
        "  --round N       看哪一轮（默认最后一轮）",
        "  --out PNG       离屏渲染后存成 PNG（不给就只跑渲染不落盘）",
        "  --ticks N       管线就绪后再渲染几帧才截图（默认 4）",
        "  --width W       离屏画布宽（默认 960，就是像素数，不受 DPI 影响）",
        "  --height H      离屏画布高（默认 640）",
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

    let world = options.world();
    println!(
        "第 {} 轮：物价指数 {:.2}、产出 {:.2}、政策消耗 {:.2}、成交额 {:.2}、拨款 {:.2}、国库 {:.2}",
        world.round,
        world.totals.cpi,
        world.totals.output,
        world.totals.consumption,
        world.totals.turnover,
        world.totals.granted,
        world.totals.treasury,
    );
    for good in &world.goods {
        println!("  商品 {:<10} 价格 {:.4}", good.name, good.price);
    }
    for department in &world.departments {
        println!(
            "  {:<10} 执行 {:.2} 收 {:>7.2} 付 {:>7.2} 库存 {}",
            department.name,
            department.execution,
            department.revenue,
            department.payment,
            department
                .holdings
                .iter()
                .map(|holding| format!("{holding:.2}"))
                .collect::<Vec<String>>()
                .join("/"),
        );
    }

    println!(
        "离屏画布 {}×{}（无窗口、无 DPI 缩放）；渲染 {} 帧后{}",
        options.width,
        options.height,
        options.ticks,
        match &options.out {
            Some(path) => format!("存到 {}", path.display()),
            None => "退出".to_string(),
        },
    );

    let ready = Arc::new(AtomicBool::new(false));
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.09)))
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
        .insert_resource(Setup {
            world,
            out: options.out,
            capture_at_tick: options.ticks,
            width: options.width,
            height: options.height,
        })
        .init_resource::<Ticks>()
        .add_systems(Startup, build_scene)
        .add_systems(Update, (diagnose, drive));

    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .insert_resource(RenderReady(ready.clone()))
            .add_systems(Render, watch_pipelines);
    }

    app.run();
}

fn diagnose(
    ticks: Res<Ticks>,
    mut done: Local<bool>,
    meshes: Query<(Entity, &ViewVisibility, &Transform), With<Mesh3d>>,
) {
    if *done || ticks.0 < 5 {
        return;
    }
    *done = true;
    let mut visible = 0usize;
    for (entity, visibility, transform) in meshes.iter() {
        if visibility.get() {
            visible += 1;
        } else {
            println!("  被剔除：{entity} 在 {:?}", transform.translation);
        }
    }
    println!("带 Mesh3d 的实体 {} 个，其中可见 {} 个", meshes.iter().count(), visible);
}

fn build_scene(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    setup: Res<Setup>,
) {
    let mut target = Image::new_target_texture(
        setup.width,
        setup.height,
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let handle = images.add(target);
    commands.insert_resource(Target(handle.clone()));

    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        RenderTarget::Image(handle.into()),
        Transform::from_xyz(0.0, 6.0, 16.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));

    commands.spawn(AmbientLight {
        brightness: 200.0,
        ..default()
    });

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(28.0, 28.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.10, 0.12, 0.17),
            perceptual_roughness: 0.9,
            ..default()
        })),
    ));

    let departments = setup.world.departments.len().max(1);
    let peak_stack = setup
        .world
        .departments
        .iter()
        .map(|department| department.holdings.iter().map(|holding| holding.max(0.0)).sum::<f32>())
        .fold(0.0_f32, f32::max)
        .max(1e-3);

    for (index, department) in setup.world.departments.iter().enumerate() {
        let x = (index as f32 - (departments as f32 - 1.0) / 2.0) * SPACING;
        let mut level = 0.0_f32;
        for (good, holding) in department.holdings.iter().enumerate() {
            let height = (holding.max(0.0) / peak_stack * MAX_STACK).max(0.04);
            commands.spawn((
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

    let goods = setup.world.goods.len().max(1);
    let peak_price = setup
        .world
        .goods
        .iter()
        .map(|good| good.price.max(0.0))
        .fold(0.0_f32, f32::max)
        .max(1e-3);

    for (good, view) in setup.world.goods.iter().enumerate() {
        let x = (good as f32 - (goods as f32 - 1.0) / 2.0) * SPACING;
        let height = (view.price.max(0.0) / peak_price * MAX_BAR).max(0.04);
        commands.spawn((
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
        DirectionalLight {
            illuminance: 9000.0,
            ..default()
        },
        Transform::from_xyz(7.0, 13.0, 9.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn drive(
    mut commands: Commands,
    mut ticks: ResMut<Ticks>,
    setup: Res<Setup>,
    ready: Res<RenderReady>,
    target: Res<Target>,
    capturing: Query<Entity, With<Capturing>>,
    mut requested: Local<bool>,
    mut warm: Local<u32>,
    mut warned: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    ticks.0 += 1;

    if !ready.0.load(Ordering::Relaxed) {
        if ticks.0 < PIPELINE_DEADLINE {
            return;
        }
        if !*warned {
            *warned = true;
            eprintln!("警告：等了 {} 帧管线仍未全部就绪，仍按当前状态截图", ticks.0);
        }
    }

    *warm += 1;
    if *warm == 1 {
        println!(
            "第 {} 帧管线就绪，再渲染 {} 帧后截图",
            ticks.0, setup.capture_at_tick
        );
    }

    if !*requested {
        if *warm >= setup.capture_at_tick {
            match &setup.out {
                Some(path) => {
                    commands
                        .spawn(Screenshot::image(target.0.clone()))
                        .observe(save_to_disk(path.clone()));
                }
                None => {
                    exit.write(AppExit::Success);
                    return;
                }
            }
            *requested = true;
        }
        return;
    }

    if capturing.is_empty() {
        exit.write(AppExit::Success);
    }
}
