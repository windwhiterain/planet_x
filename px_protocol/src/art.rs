use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::wire::BlobHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Field2D,
    Mesh,
    Instances,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetManifest {
    pub id: String,
    pub kind: AssetKind,
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<BlobHeader>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ArtBundle {
    pub assets: Vec<AssetManifest>,
}
