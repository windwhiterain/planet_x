//! `sky.nebula`：**沿视线积分**（发射体积 + 星图 → 一条通道的立方贴图场）。
//!
//! ⚠ 通道来自**节点自己的参数**（`SkyParams::channel`）：一条通道一个节点、一份缓存。
//!   三条通道必须分别积 —— 它们的消光系数不同，颜色就是这么来的
//!   （合成一条再事后乘系数的话，那个系数与"光线穿过多浓的介质"脱节）。
//!
//! ⚠ 输出是 `CubeMap` **场**（暂时，不是贴图）：`px_volume_op` 链的是 `px_volume_alg`，
//!   而后者在 `px_graph` 下面，够不到 `TextureData`。三条通道由图脚本拼成贴图
//!   （`px_graph::generate::color_cube`）。
//!   ⚠ 这个形状是**要走掉的**：`TextureData` 的 `Build` 已经写好并判过
//!   （`px_graph::generate::payload`，《贴图编一下再解回来是逐字节恒等》），
//!   等把那个载荷类型搬到 `px_protocol::art`（与 `VolumeData` / `MeshData` 同住），
//!   这一档就能直接交出一张贴图、图脚本里的手工拼图随之删掉。

use px_volume_schema::ops::SkyNebula;

px_graph_schema::px_body! {
    SkyNebula,
    |p, i, _g| px_volume_alg::raymarch_sky(i.volume.value(), i.stars.value(), p)?
}
