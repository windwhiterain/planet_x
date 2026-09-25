use std::collections::BTreeMap;

use crate::art::{ArtBundle, AssetManifest};
use crate::fnv::fnv1a;
use crate::stream::{self, Frame};
use crate::wire::Blob;

pub struct PayloadBundle {
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<Blob>,
}

impl PayloadBundle {
    pub fn new(params: BTreeMap<String, f64>, blobs: Vec<Blob>) -> Self {
        Self { params, blobs }
    }

    pub fn fingerprint(&self, id: &str) -> u64 {
        payload_fingerprint(id, &self.blobs)
    }

    pub fn bytes(&self) -> usize {
        self.blobs.iter().map(|blob| blob.bytes.len()).sum()
    }

    pub fn to_bytes(&self, id: &str) -> Result<Vec<u8>, String> {
        let bundle = ArtBundle {
            assets: vec![AssetManifest {
                id: id.to_string(),
                params: self.params.clone(),
                blobs: self.blobs.iter().map(|blob| blob.header.clone()).collect(),
                fingerprint: self.fingerprint(id),
            }],
        };
        let mut frames = vec![Frame::Art(bundle)];
        frames.extend(self.blobs.iter().cloned().map(Frame::Blob));
        let mut out = Vec::new();
        stream::write_stream(&mut out, &frames).map_err(|err| err.to_string())?;
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = bytes;
        let frames = stream::read_stream(&mut cursor).map_err(|err| err.to_string())?;
        let params = frames
            .iter()
            .find_map(|frame| match frame {
                Frame::Art(bundle) => bundle.assets.first().map(|asset| asset.params.clone()),
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
        Ok(Self { params, blobs })
    }

    pub fn one(&self) -> Result<&Blob, String> {
        self.blobs
            .first()
            .ok_or_else(|| "产物里没有数据块".to_string())
    }
}

pub fn payload_fingerprint(id: &str, blobs: &[Blob]) -> u64 {
    let mut hash = fnv1a(id);
    for blob in blobs {
        for byte in &blob.bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(crate::fnv::FNV_PRIME);
        }
    }
    hash
}
