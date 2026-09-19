//! 网格载荷的序列化：四个 blob（位置 / 法线 / uv / 索引）+ 清单里的顶点与三角形数。

use std::collections::BTreeMap;

use px_graph_schema::PayloadBundle;
use px_protocol::art::{AssetKind, MeshData};

pub fn encode(mesh: &MeshData) -> PayloadBundle {
    PayloadBundle::new(
        AssetKind::Mesh,
        BTreeMap::from([
            ("vertices".to_string(), mesh.vertices() as f64),
            ("triangles".to_string(), mesh.triangles() as f64),
        ]),
        mesh.blobs(),
    )
}

pub fn decode(bytes: &[u8]) -> Result<MeshData, String> {
    let bundle = PayloadBundle::from_bytes(bytes)?;
    let blobs: Vec<&px_protocol::wire::Blob> = bundle.blobs.iter().collect();
    MeshData::from_blobs(&blobs).map_err(|err| err.to_string())
}
