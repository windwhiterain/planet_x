#import bevy_pbr::shadows::fetch_point_shadow
#import bevy_pbr::mesh_view_bindings::clustered_lights

//! 金字塔降采样（用户裁决的 (ii)）：级 `k` 的一页 = 级 `k-1` 那四个孩子取 **max 深度**。
//!
//! ⚠⚠ **为什么取 max 不是 min/平均**：影图里存的是"沿这条射线最近的那个表面"（无限
//!    reverse-Z 下深度越大越近）。四个孩子合一个 texel 时，**取 max = 取最近的遮挡物**
//!    ⇒ 影只会**偏大**，不会漏光。单通道装不下 (min, max) 这一对（那才叫"不偏不倚"），
//!    而漏光是看得见的错、影偏大只是保守 —— **这一条偏置是故意的**，不是近似。
//!
//! ⚠ **上一级从自己的第 1 格读，不走组 0 那条定长链**：这条 pass 写着 `atlas_l{k}`，
//!    而 wgpu 不许同一条 pass 里把一张图既当（写的）深度附件、又当被绑的资源
//!    （`DEPTH_STENCIL_WRITE` 是独占用法）。于是它的组 0 走**哑图**那一档（四格全挂
//!    1×1 兜底）—— 那条链上 `textureDimensions` 会算出 0，**不能用**。
//!    自己这一格只绑 `atlas_l{k-1}`（"写谁就不绑谁"），页表照旧从组 0 取。
//!
//! ⚠ 输出是 `@builtin(frag_depth)`（`render` 里 `frag_depth=true`）：深度值只能在片元里
//!    算完写出来。而「取 max」其实有**两级** —— 片元里对四个孩子取一次，
//!    硬件的 `compare=greater_equal + depth_write` 再与这一页上已有的内容取一次
//!    （同一级上几何画上去的那些低 ρ 投影体）。
//!
//! 绑定契约与材质同一条：第 0 格是自己声明的参数块，贴图从第 1 格起。
//! `#{MATERIAL_BIND_GROUP}` 由组装器替成运行期那个数。
//!
//! ⚠ 参数块按**名字**打包（`Value::Num` → f32，偏移来自反射），所以字段是 f32，
//!    到用的地方再转 `u32`（这几格本来就是整数，量级远小于 2^24 ⇒ f32 无损）。

// ⚠ 这一支**不用** `fetch_point_shadow` 本身，但要它带进来的那份桩表：`px_shadow_page_slot`
//    （查上一级的页）、`PX_PAGE_SIZE`、`PX_CUBE_FACES` 都住在里面。宿主桩表是**按符号**
//    注入的（`px_shader::host_stubs::wgpu_host_stub`），所以这个依赖要**声明**出来 ——
//    不声明 ⇒ 组装器不注入 ⇒ 「unknown identifier」当场拒（`--bin shaders` 那一步）。
//    这比"让所有 CAS 节点都注入内容 shader 的宿主契约"干净：帧自有材质按需取，不白拿。

struct PxShadowDownParams {
    /// **级**：输出那张是 `atlas_l{level}`，读的是 `atlas_l{level-1}`。
    level: f32,
    /// 第几盏投影的点光（层号 = `light × 6 + face`）。
    light: f32,
    /// cube 的哪一面。
    face: f32,
    /// 页大小（texel，恒 `PX_PAGE_SIZE`；带上是为了不靠隐含约定）。
    page_size: f32,
    /// 这一页在**面内 texel** 的原点（**级 `level` 的坐标**）。
    origin_x: f32,
    origin_y: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PxShadowDownParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var px_child: texture_depth_2d_array;
// ⚠ 约定要"贴图 + 1 格是采样器"（`px_protocol::texture_slot_of`）：这一支走的是
//    `textureLoad`（逐 texel 取，比较采样器那一路取不到"某一页里的某一格"），
//    采样器**用不上**，但那一格仍然要声明 —— 少了它就是"有贴图没采样器"当场拒。
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var px_child_sampler: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @builtin(frag_depth) f32 {
    let level = u32(params.level);
    let light = u32(params.light);
    let face = u32(params.face);
    let page_size = u32(params.page_size);
    // 全屏三角的 `uv` 铺满**这一格 viewport**（就是那一页的 `page_size × page_size`）。
    // 取 texel 中心 ⇒ `floor(uv × page_size)` 就是页内坐标，没有半格偏移。
    let local = vec2<u32>(
        u32(clamp(uv.x * f32(page_size), 0.0, f32(page_size) - 0.001)),
        u32(clamp(uv.y * f32(page_size), 0.0, f32(page_size) - 0.001)),
    );
    // 这个输出 texel 在**级 `level`** 面内的位置，再折到**级 `level-1`** 上：一个 texel
    // 盖住上一级的 `2 × 2` 格（相邻两级之间就是 2×2，见 `px_shadow_locate` 那条注释）。
    let here = (vec2<u32>(u32(params.origin_x), u32(params.origin_y)) + local) * 2u;
    let child_level = level - 1u;
    let layer = i32(light * 6u + face);
    // ⚠ 这一张自己的页格边长只能**现算**（`textureDimensions`）：它与烘图侧分配出来的
    //    那个数必须一致，而纹理正好是「页格边长 × PX_PAGE_SIZE」（见 `px_shadow_grid`）。
    let grid = textureDimensions(px_child, 0).x / page_size;
    // ⚠ **max 深度**（见文件头：偏大 = 不漏光，故意的）。孩子那一页没分配 ⇒ 读到的是
    //    兜底 0.0（无限 reverse-Z 的"远"）⇒ 不参与 max，不把没有的东西算成遮挡。
    var nearest = 0.0;
    for (var dy = 0u; dy < 2u; dy = dy + 1u) {
        for (var dx = 0u; dx < 2u; dx = dx + 1u) {
            let child = here + vec2<u32>(dx, dy);
            let slot = px_shadow_page_slot(
                light, child_level, face, child.x / page_size, child.y / page_size);
            if (slot < 0) {
                continue;
            }
            let s = u32(slot);
            let texel = vec2<u32>(
                (s % grid) * page_size + child.x % page_size,
                (s / grid) * page_size + child.y % page_size,
            );
            nearest = max(nearest, textureLoad(px_child, texel, layer, 0));
        }
    }
    return nearest;
}
