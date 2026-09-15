use std::collections::BTreeMap;
use std::path::Path;

use px_protocol::art::{ArtBundle, AssetKind, AssetManifest, Camera};
use px_protocol::render::{Lease, Request, Response, Scene};
use px_protocol::sim::{DepartmentView, GoodView, Totals, WorldView};
use px_protocol::wire::{Blob, BlobHeader, DType};
use px_protocol::{Handshake, ProtocolId, SCHEMA_VERSION};

/// 资产种类必须**逐个列出**（穷尽匹配）：加一种资产而不动这份快照就编不过 ——
/// 快照一变 `protocol_hash` 就变，跨进程握手会因此拒绝旧对端，这正是要人看一眼的地方。
fn asset_kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Field2D => "field2d",
        AssetKind::OctahedralField => "octahedral_field",
        AssetKind::CubeField => "cube_field",
        AssetKind::CubeMap => "cube_map",
        AssetKind::Mesh => "mesh",
        AssetKind::Instances => "instances",
        AssetKind::Volume => "volume",
        AssetKind::Scene => "scene",
        AssetKind::Shader => "shader",
    }
}

fn canonical() -> String {
    let protocol_id = ProtocolId {
        schema_version: SCHEMA_VERSION,
        protocol_hash: 0,
        git_rev: "<rev>".to_string(),
    };
    let handshake = Handshake {
        id: protocol_id.clone(),
        pid: 0,
        exe: "<exe>".to_string(),
    };

    let world = WorldView {
        schema_version: SCHEMA_VERSION,
        round: 0,
        goods: vec![GoodView {
            name: "粮食".to_string(),
            price: 1.0,
        }],
        departments: vec![DepartmentView {
            name: "甲(农业)".to_string(),
            execution: 0.0,
            revenue: 0.0,
            payment: 0.0,
            holdings: vec![0.0, 0.0, 0.0],
        }],
        totals: Totals {
            cpi: 100.0,
            output: 0.0,
            consumption: 0.0,
            turnover: 0.0,
            granted: 0.0,
            treasury: 0.0,
        },
    };

    let header = BlobHeader {
        dtype: DType::F32,
        shape: vec![2, 2],
    };
    let blob = Blob::from_f32(vec![2, 2], &[0.0, 0.5, 1.0, -1.0]);

    let mut params = BTreeMap::new();
    params.insert("relief".to_string(), 0.34_f64);
    let art = ArtBundle {
        assets: vec![AssetManifest {
            id: "planet/terran".to_string(),
            kind: AssetKind::Field2D,
            params,
            blobs: vec![header.clone()],
            fingerprint: 0x0123_4567_89ab_cdef,
            cameras: vec![Camera::new([0.0, 0.0, 1.0], 3.15, "front")],
        }],
    };

    let request = Request {
        view: px_protocol::View {
            cam: Some([30.0, 15.0, 3.2]),
            sheet: true,
            columns: 4,
        },
        scene: Scene::Sequence {
            shots: vec![
                px_protocol::Shot {
                    scene: "target/pcg/ab/xx/bare.pxart".to_string(),
                    out: "target/bare.png".to_string(),
                    cam: None,
                },
                px_protocol::Shot {
                    scene: "target/pcg/ab/yy/orbit.pxart".to_string(),
                    out: "target/orbit.png".to_string(),
                    cam: Some([0.0, 8.0, 3.3]),
                },
            ],
        },
        width: 960,
        height: 640,
        out: "target/orbit.png".to_string(),
        job: px_protocol::Job::Perf {
            windows: 4,
            drop: 1,
        },
        report: "target/report.json".to_string(),
    };
    let response = Response {
        out: "target/orbit.png".to_string(),
        scene: "场景 orbit｜planet".to_string(),
        width: 960,
        height: 640,
        bytes: 326452,
        millis: 812,
        warm: true,
        shots: vec![
            "target/bare.png".to_string(),
            "target/orbit.png".to_string(),
        ],
        report: "{\"job\":\"perf\"}".to_string(),
        report_path: "target/report.json".to_string(),
    };
    // 两种活各自的形状（`Job` 是内部 tag 的枚举，两路的 JSON 必须都能被看到）。
    let job_shots = px_protocol::Job::Shots;
    let job_perf = px_protocol::Job::Perf {
        windows: 4,
        drop: 1,
    };
    let job_stable = px_protocol::Job::Stable { frames: 60 };
    let shot_report = px_protocol::ShotReport {
        scene: "target/pcg/ab/yy/orbit.pxart".to_string(),
        label: "场景 orbit｜rocky".to_string(),
        out: "target/orbit.png".to_string(),
        width: 2240,
        height: 1400,
        bytes: 1622592,
        sha256: "0".repeat(64),
        placeholder_px: 0,
        bright_px: 900000,
        diff_vs_ref_grid: 41234,
        declared_clouds: true,
        has_cloud: true,
        verdict: "有云".to_string(),
    };
    let perf_report = px_protocol::PerfReport {
        scene: "target/pcg/ab/yy/orbit.pxart".to_string(),
        label: "场景 orbit｜rocky".to_string(),
        windows: vec![63.5, 63.1, 62.9, 63.4],
        dropped: vec![45.7],
        min: 62.9,
        median: 63.25,
        n: 4,
        error_bar: px_protocol::ErrorBar {
            rule: "单次请求内：干净窗口的半极差 (max−min)/2".to_string(),
            value_ms: 0.3,
        },
        gpu: vec![px_protocol::GpuSample {
            window: 0,
            sm_mhz: vec![1245.0, 1942.0],
            power_w: vec![92.0, 112.0],
            util_pct: vec![98.0, 100.0],
            vram_mib_max: 481.0,
            temp_c_max: 80.0,
            samples: 3,
        }],
        frames: vec![62.9, 63.1, 63.5, 63.4],
        p50: 63.25,
        p90: 63.5,
        p99: 63.5,
        max: 63.5,
        key: "bee2a7ec00000000".to_string(),
        waits: Some(px_protocol::Waits {
            pipelines_ms: 412.0,
            assets_ms: 0.4,
            settle_ms: 96.0,
            sample_ms: 3810.0,
            total_ms: 4318.4,
        }),
        gpu_ms: Some(px_protocol::GpuMs {
            source: "render/main_opaque_pass_3d/elapsed_gpu=18.100 + render/main_transparent_pass_3d/elapsed_gpu=31.700".to_string(),
            lag_frames: 3,
            n: 4,
            min: 44.2,
            p50: 49.8,
            p90: 52.4,
            p99: 58.0,
            max: 58.0,
            frames: vec![44.2, 49.4, 49.8, 52.1],
            poll_us: 6.5,
        }),
        compare: Some(px_protocol::Compare {
            app_p50_ms: 63.25,
            app_mean_ms: 43.8,
            gpu_p50_ms: 49.8,
            ratio: 0.787,
            ratio_mean: 1.137,
            note: "以前拿 app 循环周期当 GPU 时间的代理量".to_string(),
        }),
    };
    let pair = px_protocol::Pair {
        measured: "场景 orbit-surface｜planet".to_string(),
        reference: "场景 orbit-bare｜planet".to_string(),
        app_delta_ms: 27.5,
        app_mean_delta_ms: 30.1,
        gpu_delta_ms: Some(24.1),
        rule: "配对差 = perf[0] − perf[1]（被测档 − 参照档）".to_string(),
    };
    let report = px_protocol::Report {
        schema_version: SCHEMA_VERSION,
        protocol_hash: "0000000000000000".to_string(),
        job: "stable".to_string(),
        width: 2240,
        height: 1400,
        millis: 91234,
        shots: vec![shot_report.clone()],
        perf: vec![perf_report.clone()],
        pair: Some(pair.clone()),
    };
    let world_scene = Scene::World {
        stream: "target/world.pxstream".to_string(),
        round: Some(200),
    };
    let artifact_scene = Scene::Artifact {
        scene: "target/pcg/ab/7f/scene.pxart".to_string(),
    };
    let sequence_scene = Scene::Sequence {
        shots: vec![px_protocol::Shot {
            scene: "target/pcg/ab/7f/scene.pxart".to_string(),
            out: "target/shot.png".to_string(),
            cam: Some([0.0, 12.0, 3.15]),
        }],
    };
    let scene_spec = px_protocol::SceneSpec {
        schema: px_protocol::SCENE_SCHEMA,
        name: "orbit".to_string(),
        ambient: 80.0,
        cameras: vec![Camera::new([0.0, 1.0, 0.0], 3.15, "review")],
        parts: vec![
            px_protocol::Part {
                id: "planet".to_string(),
                kind: "planet".to_string(),
                shader: "surface".to_string(),
                members: BTreeMap::from([
                    (
                        "height".to_string(),
                        px_protocol::Member::new("planet", "height", &"a".repeat(64)),
                    ),
                    (
                        "mesh".to_string(),
                        px_protocol::Member::new("planet", "surface", &"b".repeat(64)),
                    ),
                ]),
                params: BTreeMap::from([
                    ("palette".to_string(), px_protocol::Value::Text("rocky".to_string())),
                    ("displace".to_string(), px_protocol::Value::Num(0.06)),
                    ("radius".to_string(), px_protocol::Value::Num(1.0)),
                ]),
            },
            px_protocol::Part {
                id: "clouds".to_string(),
                kind: "clouds".to_string(),
                shader: "clouds".to_string(),
                members: BTreeMap::from([
                    (
                        "shader".to_string(),
                        px_protocol::Member::new("shaders", "clouds", &"9".repeat(64)),
                    ),
                    (
                        "field".to_string(),
                        px_protocol::Member::new("clouds", "mixed", &"c".repeat(64)),
                    ),
                    (
                        "slope_x".to_string(),
                        px_protocol::Member::new("clouds", "slope_x", &"d".repeat(64)),
                    ),
                    (
                        "slope_y".to_string(),
                        px_protocol::Member::new("clouds", "slope_y", &"e".repeat(64)),
                    ),
                    (
                        "slope_z".to_string(),
                        px_protocol::Member::new("clouds", "slope_z", &"f".repeat(64)),
                    ),
                ]),
                params: BTreeMap::from([
                    ("inner".to_string(), px_protocol::Value::Num(1.01)),
                    ("outer".to_string(), px_protocol::Value::Num(1.06)),
                    ("extinction".to_string(), px_protocol::Value::Num(900.0)),
                    ("coverage".to_string(), px_protocol::Value::Num(0.35)),
                    ("steps".to_string(), px_protocol::Value::Num(56.0)),
                    ("seed".to_string(), px_protocol::Value::Num(7.0)),
                    (
                        "tint".to_string(),
                        px_protocol::Value::Triple([0.44, 0.64, 0.98]),
                    ),
                ]),
            },
        ],
    };
    let lease = Lease {
        pid: 0,
        port: 0,
        protocol_hash: 0,
        git_rev: "<rev>".to_string(),
        exe: "<exe>".to_string(),
    };

    let asset_kinds: Vec<&'static str> = [
        AssetKind::Field2D,
        AssetKind::OctahedralField,
        AssetKind::CubeField,
        AssetKind::CubeMap,
        AssetKind::Mesh,
        AssetKind::Instances,
        AssetKind::Volume,
        AssetKind::Scene,
        AssetKind::Shader,
    ]
    .map(asset_kind_name)
    .to_vec();
    // 3D 标量网格（等值面算子的输入）的载荷形状：`[面, 径向层, t, s]`。
    let volume_shape = px_protocol::art::VolumeData {
        res: 65,
        layers: 65,
        inner: 1.01,
        outer: 1.06,
        data: vec![0.0; 6 * 65 * 65 * 65],
    }
    .blobs()
    .first()
    .map(|blob| {
        serde_json::json!({
            "dtype": blob.header.dtype,
            "shape": blob.header.shape,
            "bytes": blob.bytes.len(),
        })
    });

    let value = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "surfaces": {
            "ProtocolId": protocol_id,
            "Handshake": handshake,
            "sim::WorldView": world,
            "art::ArtBundle": art,
            "art::AssetKind.kinds": asset_kinds,
            "art::VolumeData.shape": volume_shape,
            "render::Request": request,
            "render::Response": response,
            "render::Job.shots": job_shots,
            "render::Job.perf": job_perf,
            "render::Job.stable": job_stable,
            "render::Report": report,
            "render::Pair": pair,
            "render::ShotReport": shot_report,
            "render::PerfReport": perf_report,
            "render::Scene.world": world_scene,
            "render::Scene.artifact": artifact_scene,
            "render::Scene.sequence": sequence_scene,
            "scene::SceneSpec": scene_spec,
            "render::Lease": lease,
            "wire::BlobHeader": header,
            "wire::Blob.payload_bytes": blob.bytes.len(),
            "stream::Frame.kinds": [
                "protocol", "world", "art", "scene", "blob", "request", "response", "refused"
            ],
            "stream::MAGIC": String::from_utf8_lossy(&px_protocol::stream::MAGIC),
            "stream::STREAM_VERSION": px_protocol::stream::STREAM_VERSION,
            "client::LEASE_PATH": px_protocol::client::LEASE_PATH,
        }
    });

    let mut text = serde_json::to_string_pretty(&value).expect("快照必须能序列化");
    text.push('\n');
    text
}

#[test]
fn protocol_snapshot_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshots/protocol.snapshot.json");
    let current = canonical();
    if std::env::var("PX_UPDATE_SNAPSHOT").as_deref() == Ok("1") {
        std::fs::write(&path, &current).expect("写快照失败");
        return;
    }
    let recorded = std::fs::read_to_string(&path).expect("缺少协议快照");
    assert_eq!(
        recorded, current,
        "协议形状变了。确认无误后跑 PX_UPDATE_SNAPSHOT=1 cargo test -p px_protocol 更新快照；\
         快照一变 protocol_hash 就变，跨进程握手会因此拒绝旧对端"
    );
}

#[test]
fn snapshot_drives_the_protocol_hash() {
    assert_ne!(px_protocol::protocol_hash(), 0);
    assert_eq!(px_protocol::protocol_hash_hex().len(), 16);
    let again = px_protocol::protocol_hash();
    assert_eq!(px_protocol::protocol_hash(), again);
}


