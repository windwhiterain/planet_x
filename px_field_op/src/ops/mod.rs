// ⚠ `constant` / `mix` / `remap` 三档已删（2026-09-27）：用户裁定 A —— 只收**纯 pointwise**
//   的那三档，它们今天是 element 函数（`px_elem/body/*.rs`，由 `px build` 编成内容寻址的
//   实例库），不再住这份预置库。⚠ 它们用的那把尺子（`px_field_alg::Scale` / `map_grid`）
//   **没搬**：泛型实例（`art/inst/*.rs`）与 element 体文件都在链它。
pub mod craters;
pub mod fbm;
pub mod fbm3;
pub mod gradient;
pub mod ridged;
pub mod ridged3;
pub mod stamps;
pub mod warp;
pub mod warp3;
