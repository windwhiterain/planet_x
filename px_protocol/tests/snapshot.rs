use std::collections::BTreeMap;
use std::path::Path;

use px_protocol::art::{ArtBundle, AssetKind, AssetManifest};
use px_protocol::render::{Lease, Request, Response};
use px_protocol::sim::{DepartmentView, GoodView, Totals, WorldView};
use px_protocol::wire::{Blob, BlobHeader, DType};
use px_protocol::{Handshake, ProtocolId, SCHEMA_VERSION};

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
        }],
    };

    let request = Request {
        stream: "target/world.pxstream".to_string(),
        round: Some(200),
        width: 960,
        height: 640,
        out: "target/shot.png".to_string(),
    };
    let response = Response {
        out: "target/shot.png".to_string(),
        round: 200,
        width: 960,
        height: 640,
        bytes: 326452,
        millis: 812,
        warm: true,
    };
    let lease = Lease {
        pid: 0,
        port: 0,
        protocol_hash: 0,
        git_rev: "<rev>".to_string(),
        exe: "<exe>".to_string(),
    };

    let value = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "surfaces": {
            "ProtocolId": protocol_id,
            "Handshake": handshake,
            "sim::WorldView": world,
            "art::ArtBundle": art,
            "render::Request": request,
            "render::Response": response,
            "render::Lease": lease,
            "wire::BlobHeader": header,
            "wire::Blob.payload_bytes": blob.bytes.len(),
            "stream::Frame.kinds": [
                "protocol", "world", "art", "blob", "request", "response", "refused"
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
