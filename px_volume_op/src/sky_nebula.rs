//! `sky.nebula`：**沿视线积分**（发射体积 + 星图 → 一条通道的立方贴图场）。
//!
//! ⚠ 通道来自**节点自己的参数**（`SkyParams::channel`）：一条通道一个节点、一份缓存。
//!   三条通道要分别积，因为它们的消光系数不同（尘埃偏红就是这么来的）——
//!   合成一条再事后乘系数的话，那个系数与"光线穿过多浓的介质"脱节。
//!
//! ⚠ 输出是 `CubeMap` **场**（不是贴图）：`px_volume_op` 链的是 `px_volume_alg`，
//!   而后者在 `px_graph` 下面，够不到 `TextureData`。三条通道由图脚本拼成贴图。

use px_volume_schema::ops::SkyNebula;

px_graph_schema::px_body! {
    SkyNebula,
    |p, i, _g| px_volume_alg::raymarch_channel(
        i.volume.value(),
        Some(i.stars.value()),
        p,
        p.channel as usize,
    )
}
