use std::collections::BTreeMap;

use px_protocol::art::{ArtBundle, AssetKind, AssetManifest, Camera};
use px_protocol::stream::{self, Frame};
use px_protocol::wire::{Blob, BlobHeader, DType, WireError};
use px_protocol::{Handshake, HandshakeError, ProtocolId};

fn frames() -> Vec<Frame> {
    let mut params = BTreeMap::new();
    params.insert("relief".to_string(), 0.34_f64);
    let bundle = ArtBundle {
        assets: vec![AssetManifest {
            id: "planet/terran".to_string(),
            kind: AssetKind::Mesh,
            params,
            blobs: vec![BlobHeader {
                dtype: DType::U32,
                shape: vec![3],
            }],
            fingerprint: 0xdead_beef_1234_5678,
            cameras: vec![Camera::new([0.0, 0.0, 1.0], 3.15, "front")],
        }],
    };
    vec![
        Frame::Protocol(ProtocolId::local()),
        Frame::Art(bundle),
        Frame::Blob(Blob::from_f32(vec![2, 2], &[0.0, 0.5, 1.0, -3.25])),
    ]
}

#[test]
fn stream_round_trips_byte_identically() {
    let frames = frames();
    let mut first = Vec::new();
    stream::write_stream(&mut first, &frames).expect("写流失败");

    let replayed = stream::read_stream(&mut first.as_slice()).expect("读流失败");
    assert_eq!(replayed, frames);

    let mut second = Vec::new();
    stream::write_stream(&mut second, &replayed).expect("重放写流失败");
    assert_eq!(first, second, "重放后必须逐字节相同");
}

#[test]
fn stream_rejects_foreign_magic() {
    let bytes = b"NOPE\x01\x00\x00\x00".to_vec();
    assert_eq!(
        stream::read_stream(&mut bytes.as_slice()),
        Err(WireError::BadMagic)
    );
}

#[test]
fn handshake_mismatch_is_rejected() {
    let local = Handshake::local();
    assert!(local.verify(&local).is_ok());

    let mut other_version = local.clone();
    other_version.id.schema_version += 1;
    assert!(matches!(
        local.verify(&other_version),
        Err(HandshakeError::SchemaVersion { .. })
    ));

    let mut other_hash = local.clone();
    other_hash.id.protocol_hash ^= 1;
    assert!(matches!(
        local.verify(&other_hash),
        Err(HandshakeError::ProtocolHash { .. })
    ));
}

#[test]
fn blob_length_and_dtype_are_checked() {
    let header = BlobHeader {
        dtype: DType::F32,
        shape: vec![2, 2],
    };
    assert!(matches!(
        Blob::new(header.clone(), vec![0; 8]),
        Err(WireError::BadPayloadLength { .. })
    ));

    let u32_blob = Blob {
        header: BlobHeader {
            dtype: DType::U32,
            shape: vec![2],
        },
        bytes: vec![0; 8],
    };
    assert!(matches!(u32_blob.f32s(), Err(WireError::NotF32(DType::U32))));
}

#[test]
fn f32_payload_round_trips_exactly() {
    let values = [0.0_f32, 0.5, 1.0, -3.25, 1.0 / 3.0];
    let blob = Blob::from_f32(vec![5], &values);
    assert_eq!(blob.f32s().expect("读回 f32 失败"), values);

    let decoded = Blob::decode(&blob.encode().expect("编码失败")).expect("解码失败");
    assert_eq!(decoded, blob);
}
