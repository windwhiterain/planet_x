//! **闭式覆盖度的单态化实例** —— 这一份里的 `bake<ClosedCover>` 是**在图侧 dylib 内**
//! 单态化出来的，不是静态链进图程序的。
//!
//! ⚠ 编辑面就这一个函数：改它 ⇒ `SOURCE_HASH` 变（`include_str!` 在 `lib.rs` 里）
//! ⇒ 缓存键变 ⇒ 必然重算；而**图脚本 `clouds.exe` 一个字节都不用重编**。

use px_cook::field_fn::CoverCloud;

/// 覆盖度：0 = 无云、1 = 满覆盖。
pub fn closed_cover(cloud: &CoverCloud, direction: [f32; 3]) -> f32 {
    let local = cloud.to_local(direction);
    let longitude = local[2].atan2(local[0]);
    let latitude = local[1].clamp(-1.0, 1.0).asin();
    let wave = (longitude * 11.25).sin() * (latitude * 4.9).cos();
    let ripple = ((longitude + latitude) * 21.7 + 3.0).sin();
    cloud.cover_from_mask((0.5 + 0.32 * wave + 0.18 * ripple).clamp(0.0, 1.0))
}
