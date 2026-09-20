//! 程序化贴图与网格的生成：色板贴图、发光贴图、覆盖度立方图、星空、环带与环网格。
//!
//! 它是从渲染器（`px_render/src/planet.rs` 的 `Palette` / `surface_textures` /
//! `mip_chain` / `pole_cap_filter` / `star_cube` / `ring_mesh` / `ring_image` 与
//! `px_render/src/clouds.rs` 的 `coverage_image` / `half_from_f32`）**逐字搬过来**的：
//! 像素公式、四舍五入、`powf(1.0 / 2.2)`、mip 的 `% w` 环绕与 `.min(h - 1)` 夹取、
//! `pole_cap_filter` 的权重、`mip_chain_cube` 的按面分块，一个字符都没改。
//! 搬运的判据是**逐字节相同**（`target/legacy-gen` ↔ `px_graphs --bin genprobe`）。
//!
//! 只换了两样东西（都是"取数的地方"，不是算法）：
//!   · 场从渲染器的 `Field`（`data` / `min` / `max` / `texel_latitude`）换成
//!     `px_graph::field::Field`（`data` / `stats()`；取数口径逐字照抄，见 [`load_field`]）；
//!   · 产物从 bevy 的 `Image` / `Mesh` 换成 [`TextureData`] / `px_protocol::art::MeshData`
//!     —— 写进 `Image.data` 的那串字节原样就是 [`TextureData::bytes`]。
//!
//! 采样器（`with_wrapping` 的 address mode / filter）与 `TextureViewDescriptor`
//! （cube 视图）**没有搬**：它们说的是"怎么采"不是"是什么"，进不了字节。
//! 渲染器照 [`TextureData::layers`] / `format` 重建 Image 时按产物那一档设即可。
//!
//! 一个文件装不下，按主题拆成了目录模块：色板 `palette` / 着色 `shade` /
//! 贴图与图像 `texture` / 几何 `mesh` / 落盘与读回 `store`；各自顶上有一句 `//!`
//! 说边界。拆分是**搬家**，不是重构：原来 `pub` 的名字都在这里 re-export，
//! `px_graph::generate::<名>` 与拆之前完全一样——跨模块共用的私有 helper 只是抬到
//! `pub(super)`，行为一个字没动。

mod mesh;
mod palette;
mod shade;
mod store;
mod texture;

pub use mesh::ring_mesh;
pub use palette::Palette;
pub use shade::{surface_color, texel_latitude};
pub use store::{
    fingerprint_of, load_field, write_generated_mesh, write_generated_mesh_at, write_texture,
    write_texture_at, Generated,
};
pub use texture::{coverage_cube, field_cube, ring_band, stars, TextureData};

pub use px_protocol::art::{MeshData, TextureFormat};
