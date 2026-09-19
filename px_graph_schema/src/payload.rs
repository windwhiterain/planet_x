//! 载荷的序列化：**算子之间流动的就是这一份字节**。
//!
//! 它和 CAS 里那份文件是同一个形状（`Frame::Art` + `Frame::Blob`，见 `px_protocol::stream`），
//! 只有三样是**驱动**才知道的东西不进载荷：节点名（`id`）、相机表、指纹。
//! ⇒ 算子只回一个「无名」的载荷，驱动把它补成产物 —— 逐字节与
//! **拆分前** `px_ops::write_artifact` 写出来的相同（判据就是六份冻产物）。

use std::collections::BTreeMap;

use px_protocol::art::{ArtBundle, AssetKind, AssetManifest, Camera};
use px_protocol::stream::{self, Frame};
use px_protocol::wire::Blob;

use crate::keys::payload_fingerprint;

pub struct PayloadBundle {
    pub kind: AssetKind,
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<Blob>,
}

impl PayloadBundle {
    pub fn new(kind: AssetKind, params: BTreeMap<String, f64>, blobs: Vec<Blob>) -> Self {
        Self {
            kind,
            params,
            blobs,
        }
    }

    /// 载荷内容的指纹（清单里那一格）。
    pub fn fingerprint(&self, id: &str) -> u64 {
        payload_fingerprint(id, &self.blobs)
    }

    /// 算子那一侧返回的就是这个：`id` 空、无相机。
    pub fn placeholder(&self) -> Result<Vec<u8>, String> {
        self.to_bytes("", &[])
    }

    /// 包成产物字节。`id` / 相机表 / 指纹是驱动补上的那三样。
    pub fn to_bytes(&self, id: &str, cameras: &[Camera]) -> Result<Vec<u8>, String> {
        let bundle = ArtBundle {
            assets: vec![AssetManifest {
                id: id.to_string(),
                kind: self.kind,
                params: self.params.clone(),
                blobs: self.blobs.iter().map(|blob| blob.header.clone()).collect(),
                fingerprint: self.fingerprint(id),
                cameras: cameras.to_vec(),
            }],
        };
        let mut frames = vec![Frame::Art(bundle)];
        frames.extend(self.blobs.iter().cloned().map(Frame::Blob));
        let mut out = Vec::new();
        stream::write_stream(&mut out, &frames).map_err(|err| err.to_string())?;
        Ok(out)
    }

    /// 从产物字节（CAS 里那份，或者算子刚回的）还原。清单帧必须在最前面，与
    /// `px_protocol` 的读法一致。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = bytes;
        let frames = stream::read_stream(&mut cursor).map_err(|err| err.to_string())?;
        let (kind, params) = frames
            .iter()
            .find_map(|frame| match frame {
                Frame::Art(bundle) => bundle
                    .assets
                    .first()
                    .map(|asset| (asset.kind, asset.params.clone())),
                _ => None,
            })
            .ok_or_else(|| "产物里没有清单帧（`Frame::Art`）".to_string())?;
        let blobs: Vec<Blob> = frames
            .iter()
            .filter_map(|frame| match frame {
                Frame::Blob(blob) => Some(blob.clone()),
                _ => None,
            })
            .collect();
        if blobs.is_empty() {
            return Err("产物里没有数据块".to_string());
        }
        Ok(Self {
            kind,
            params,
            blobs,
        })
    }

    pub fn one(&self) -> Result<&Blob, String> {
        self.blobs
            .first()
            .ok_or_else(|| "产物里没有数据块".to_string())
    }
}
