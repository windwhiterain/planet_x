// ⚠ 本文件被生成物 `#[path = "fields.rs"] mod fields;` 引进一个**模块**里
// ⇒ 不能有 `//!` 文档注释（E0753），函数上也不挂 `///`（会报"未使用的文档注释"）。
// 说明写在这里。
//
// clouds 这一张图的 **stage 1**：要单态化的那一半。
//
// 这里只有一个闭式场函数 —— 它是这一张图"图上现写的覆盖度"。
// `bake<F: FieldFn>` 的实例会**按这个类型**在别处单态化出来（见 `src/bin/mono-gen.rs`）。
//
// ⚠ 这个文件是**编辑面**：改它就等于换了一个覆盖度函数。
// 它被 `mono-gen` 整个读进实例身份 ⇒ 缓存键变 ⇒ 下一次烘必然重算，
// 而**图程序 `clouds.exe` 一个字节都不用重编**。

use px_cook::field_fn::CoverCloud;

// 覆盖度：0 = 无云、1 = 满覆盖。
//
// `cloud` 是算子建好的上下文（那一档云参数），`direction` 是世界方向。
// 闭式场不需要上游那张采样好的覆盖度场 —— 这正是它省掉的东西。
pub fn closed_cover(cloud: &CoverCloud, direction: [f32; 3]) -> f32 {
    let local = cloud.to_local(direction);
    let longitude = local[2].atan2(local[0]);
    let latitude = local[1].clamp(-1.0, 1.0).asin();
    let wave = (longitude * 13.5).sin() * (latitude * 4.9).cos();
    let ripple = ((longitude + latitude) * 21.7 + 3.0).sin();
    cloud.cover_from_mask((0.5 + 0.32 * wave + 0.18 * ripple).clamp(0.0, 1.0))
}
