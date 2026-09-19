// 天空盒顶点阶段（帧图的 `sky` 那条 pass 用它，§128）。
//
// 这一笔是**程序化**的：没有顶点缓冲（`ResolvedGeometry.vertices = None`），
// 三个顶点由 `vertex_index` 现算 —— 这就是判据里那条"没有顶点缓冲也画得出来"的路。
//
// ⚠ **输出只有 `@builtin(position)`** —— 这是照抄 Bevy，不是"先简陋一点"：
//    `bevy_core_pipeline-0.19.1/src/skybox/skybox.wgsl:48-50` 的 `VertexOutput`
//    就只有一个 `@builtin(position)`，方向的重建在**片元**那一侧
//    （同文件 `:19-46` 的 `coords_to_ray_direction`：拿 `in.position.xy` + `view.viewport`
//    + `view.view_from_clip` + `view.world_from_view` 反算）。
//
// ⚠ 这里**不许**多传一个裁剪坐标 varying（第一版就是那样：`@location(0) clip`）。
//    那个冗余不是无害的：它会让实现者顺手走"插值下来的裁剪坐标"那条公式，
//    而两条公式算的是**同一个方向、不同的算术路径** ⇒ 逐位判据下**整幅背景都差 ±1**、
//    而画面看起来完全正确。诊断提示：**背景差的是整齐的 ±1 ⇒ 先怀疑方向重建的算术路径**，
//    不是纹理、也不是亮度。
//
// 顶点摆位与 Bevy 的 `skybox_vertex`（`skybox.wgsl:63-72`）**逐字相同**：
// `vec2(f32(i & 1u), f32((i >> 1u) & 1u)) * 4.0 - vec2(1.0)`，深度取 `0.0`
// （无限 reverse-Z 的**远平面**）—— 于是它盖满屏幕、被任何写过的深度挡住。

struct SkyVertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32) -> SkyVertexOutput {
    // Bevy 那个式子（`skybox.wgsl:66-69`）：三个顶点分别是 (-1,-1) / (3,-1) / (-1,3)。
    let clip_position = vec2(
        f32(vertex_index & 1u),
        f32((vertex_index >> 1u) & 1u),
    ) * 4.0 - vec2(1.0);

    var out: SkyVertexOutput;
    out.position = vec4<f32>(clip_position, 0.0, 1.0);
    return out;
}
