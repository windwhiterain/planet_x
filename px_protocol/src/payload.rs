//! 载荷的**线格式**那一半：`PayloadBundle` 就是 CAS 里那份文件的形状（清单帧 + 数据块）。
//!
//! ⚠ 读写它的是**两侧**：烘图侧写、渲染侧读（渲染器不该依赖契约层）。所以它住在
//!   线格式这一层，与它一起的还有清单里那一格 FNV 指纹（它是**编进帧里的字节**，
//!   不是"烘图怎么想"）。
//!
//! ⚠ 另一半（`Build`：`WITH_CAMERAS` / `detail` / `decode` —— 那三样都是**驱动**的概念）
//!   住 `px_graph_schema::build`（用户 2026-09-27 的裁定：拆开）。
//!   各域的 `impl` 跟着**自己的载荷类型**走（孤儿规则那条口径）。

use std::collections::BTreeMap;

use crate::art::{ArtBundle, AssetManifest};
use crate::fnv::fnv1a;
use crate::stream::{self, Frame};
use crate::wire::Blob;

/// **节点载荷**：清单参数 + 数据块。
///
/// ⚠ **没有"这是什么资产种类"那一栏**。缓存按代码位置取载荷 —— 类型是 `cached::<O>` 里
///   那个 `O::Payload`，**编译期就已知**（`Build::decode` 由它单态化）⇒ 字节里再写一遍
///   类型没有任何消费者。`AssetKind` 是**渲染**那一侧的概念（`load_texture` /
///   `load_mesh` 要从一坨字节里认出"这是什么"），它读的是**显式写进 CAS 的场景资产**
///   （`Generated` / `write_texture` / `write_generated_mesh`），不是这里的节点载荷。
pub struct PayloadBundle {
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<Blob>,
}

impl PayloadBundle {
    pub fn new(params: BTreeMap<String, f64>, blobs: Vec<Blob>) -> Self {
        Self { params, blobs }
    }

    /// 载荷内容的指纹（清单里那一格）。
    pub fn fingerprint(&self, id: &str) -> u64 {
        payload_fingerprint(id, &self.blobs)
    }

    /// 数据本身的字节数（不含清单帧与块头）—— 读数用。
    pub fn bytes(&self) -> usize {
        self.blobs.iter().map(|blob| blob.bytes.len()).sum()
    }

    /// 包成产物字节。`id` 是驱动补上的那一样。
    ///
    /// ⚠ **没有相机表**：相机是**场景脚本**里的普通数据（`px-scene` 的 recipe），
    ///   产物不该自带"该怎么看"。
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

    /// 从产物字节（CAS 里那份）还原。清单帧必须在最前面，与读法一致。
    ///
    /// ⚠ **只取参数与数据块** —— 类型由调用方（`Build::decode` 的单态化）说话。
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

/// 载荷内容的 FNV-1a。节点名也混进去，免得两个载荷相同的节点指纹撞上。
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
