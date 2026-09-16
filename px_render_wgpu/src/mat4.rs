//! `Mat4::inverse()`，与 `glam 0.32.1` 在 x86_64 上**逐位一致**。**故意不用 `glam`。**
//!
//! x86_64-windows 上 glam 走的是 **SSE2** 后端（`src/f32/sse2/mat4.rs`）。SSE2 没有 FMA，
//! 所以只要**照抄操作数顺序**与**每一处 `a*b - c*d` 的结合**，标量转写就是逐位相同的。
//! 下面 `inverse()` 是 `src/f32/sse2/mat4.rs:697` 的 `inverse_checked::<false>`
//! （由 `:857` 的 `inverse()` 调用）的逐句转写：`fac0..fac5`、`sign_a/sign_b`、
//! `vec0..vec3`、`inv0..inv3`、`dot4` 取行列式、最后一次 `_mm_set1_ps(dot0.recip())`
//! 与四列 `_mm_mul_ps` —— **全部照原顺序**。
//!
//! 唯一替身是那几个 SSE 内建：`_mm_shuffle_ps(a, b, imm)` = `[a[imm&3], a[(imm>>2)&3],
//! b[(imm>>4)&3], b[(imm>>6)&3]]`，`_mm_set_ps` **参数是反的**（`set_ps(e3,e2,e1,e0)`）。
//!
//! 判据见 `target/oracle/bevy-view-vectors.txt`：6 个输入矩阵（刚体、带缩放、
//! 非正交都有），每个的 16 个位模式都要对上。
//!
//! ⚠ 这个模块还没有接进 `--device`/`--shot` 那条主路径，所以 `cargo build` 会报几条
//! dead-code —— `mesh.rs` / `vec.rs` / `icosphere.rs` 处在**同一阶段**，一样报
//! （`cargo test` 那条路是干净的，因为这些测试就是调用者）。
//! **故意不在这里加 `#![allow(dead_code)]`**：那会把"还没接线"与"真的写多了"
//! 一起盖掉，而后者正是要看得见的东西。S2 的渲染路径一接上，这些警告自己就没了。

/// `Vec4`：与 `glam::Vec4` 同样的四个 `f32`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    #[inline(always)]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
}

/// 列主序的 4x4 矩阵，与 `glam::Mat4` 同布局。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub x_axis: Vec4,
    pub y_axis: Vec4,
    pub z_axis: Vec4,
    pub w_axis: Vec4,
}

impl Mat4 {
    /// `glam 0.32.1` `src/f32/sse2/mat4.rs:114`。
    #[inline(always)]
    pub const fn from_cols(x_axis: Vec4, y_axis: Vec4, z_axis: Vec4, w_axis: Vec4) -> Self {
        Self {
            x_axis,
            y_axis,
            z_axis,
            w_axis,
        }
    }

    /// `glam 0.32.1` `src/f32/sse2/mat4.rs:857` -> `:697`（`CHECKED = false`）。
    ///
    /// 行列式为零时不检查（`glam_assert!` 在 release 里是编掉的），结果按 glam 一样是无效矩阵。
    #[must_use]
    pub fn inverse(&self) -> Self {
        /// `_mm_shuffle_ps(a, b, imm)`
        #[inline(always)]
        fn sh(a: [f32; 4], b: [f32; 4], imm: u32) -> [f32; 4] {
            [
                a[(imm & 3) as usize],
                a[((imm >> 2) & 3) as usize],
                b[((imm >> 4) & 3) as usize],
                b[((imm >> 6) & 3) as usize],
            ]
        }
        /// `_mm_mul_ps`
        #[inline(always)]
        fn mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
            [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
        }
        /// `_mm_sub_ps`
        #[inline(always)]
        fn sub(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
            [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
        }
        /// `_mm_add_ps`
        #[inline(always)]
        fn add(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
            [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
        }
        /// `_mm_set_ps(e3, e2, e1, e0)` —— 参数反着来。
        #[inline(always)]
        const fn set_ps(e3: f32, e2: f32, e1: f32, e0: f32) -> [f32; 4] {
            [e0, e1, e2, e3]
        }
        /// `_mm_set1_ps`
        #[inline(always)]
        const fn set1_ps(v: f32) -> [f32; 4] {
            [v, v, v, v]
        }
        /// glam `src/sse2.rs:72`：`(x*x' + z*z') + (y*y' + w*w')`。
        #[inline(always)]
        fn dot4(lhs: [f32; 4], rhs: [f32; 4]) -> f32 {
            let x2_y2_z2_w2 = mul(lhs, rhs);
            let z2_w2_0_0 = sh(x2_y2_z2_w2, x2_y2_z2_w2, 0b00_00_11_10);
            let x2z2_y2w2_0_0 = add(x2_y2_z2_w2, z2_w2_0_0);
            let y2w2_0_0_0 = sh(x2z2_y2w2_0_0, x2z2_y2w2_0_0, 0b00_00_00_01);
            add(x2z2_y2w2_0_0, y2w2_0_0_0)[0]
        }

        let (x, y, z, w) = (
            [self.x_axis.x, self.x_axis.y, self.x_axis.z, self.x_axis.w],
            [self.y_axis.x, self.y_axis.y, self.y_axis.z, self.y_axis.w],
            [self.z_axis.x, self.z_axis.y, self.z_axis.z, self.z_axis.w],
            [self.w_axis.x, self.w_axis.y, self.w_axis.z, self.w_axis.w],
        );

        let fac0 = {
            let swp0a = sh(w, z, 0b11_11_11_11);
            let swp0b = sh(w, z, 0b10_10_10_10);

            let swp00 = sh(z, y, 0b10_10_10_10);
            let swp01 = sh(swp0a, swp0a, 0b10_00_00_00);
            let swp02 = sh(swp0b, swp0b, 0b10_00_00_00);
            let swp03 = sh(z, y, 0b11_11_11_11);

            let mul00 = mul(swp00, swp01);
            let mul01 = mul(swp02, swp03);
            sub(mul00, mul01)
        };
        let fac1 = {
            let swp0a = sh(w, z, 0b11_11_11_11);
            let swp0b = sh(w, z, 0b01_01_01_01);

            let swp00 = sh(z, y, 0b01_01_01_01);
            let swp01 = sh(swp0a, swp0a, 0b10_00_00_00);
            let swp02 = sh(swp0b, swp0b, 0b10_00_00_00);
            let swp03 = sh(z, y, 0b11_11_11_11);

            let mul00 = mul(swp00, swp01);
            let mul01 = mul(swp02, swp03);
            sub(mul00, mul01)
        };
        let fac2 = {
            let swp0a = sh(w, z, 0b10_10_10_10);
            let swp0b = sh(w, z, 0b01_01_01_01);

            let swp00 = sh(z, y, 0b01_01_01_01);
            let swp01 = sh(swp0a, swp0a, 0b10_00_00_00);
            let swp02 = sh(swp0b, swp0b, 0b10_00_00_00);
            let swp03 = sh(z, y, 0b10_10_10_10);

            let mul00 = mul(swp00, swp01);
            let mul01 = mul(swp02, swp03);
            sub(mul00, mul01)
        };
        let fac3 = {
            let swp0a = sh(w, z, 0b11_11_11_11);
            let swp0b = sh(w, z, 0b00_00_00_00);

            let swp00 = sh(z, y, 0b00_00_00_00);
            let swp01 = sh(swp0a, swp0a, 0b10_00_00_00);
            let swp02 = sh(swp0b, swp0b, 0b10_00_00_00);
            let swp03 = sh(z, y, 0b11_11_11_11);

            let mul00 = mul(swp00, swp01);
            let mul01 = mul(swp02, swp03);
            sub(mul00, mul01)
        };
        let fac4 = {
            let swp0a = sh(w, z, 0b10_10_10_10);
            let swp0b = sh(w, z, 0b00_00_00_00);

            let swp00 = sh(z, y, 0b00_00_00_00);
            let swp01 = sh(swp0a, swp0a, 0b10_00_00_00);
            let swp02 = sh(swp0b, swp0b, 0b10_00_00_00);
            let swp03 = sh(z, y, 0b10_10_10_10);

            let mul00 = mul(swp00, swp01);
            let mul01 = mul(swp02, swp03);
            sub(mul00, mul01)
        };
        let fac5 = {
            let swp0a = sh(w, z, 0b01_01_01_01);
            let swp0b = sh(w, z, 0b00_00_00_00);

            let swp00 = sh(z, y, 0b00_00_00_00);
            let swp01 = sh(swp0a, swp0a, 0b10_00_00_00);
            let swp02 = sh(swp0b, swp0b, 0b10_00_00_00);
            let swp03 = sh(z, y, 0b01_01_01_01);

            let mul00 = mul(swp00, swp01);
            let mul01 = mul(swp02, swp03);
            sub(mul00, mul01)
        };
        let sign_a = set_ps(1.0, -1.0, 1.0, -1.0);
        let sign_b = set_ps(-1.0, 1.0, -1.0, 1.0);

        let temp0 = sh(y, x, 0b00_00_00_00);
        let vec0 = sh(temp0, temp0, 0b10_10_10_00);

        let temp1 = sh(y, x, 0b01_01_01_01);
        let vec1 = sh(temp1, temp1, 0b10_10_10_00);

        let temp2 = sh(y, x, 0b10_10_10_10);
        let vec2 = sh(temp2, temp2, 0b10_10_10_00);

        let temp3 = sh(y, x, 0b11_11_11_11);
        let vec3 = sh(temp3, temp3, 0b10_10_10_00);

        let mul00 = mul(vec1, fac0);
        let mul01 = mul(vec2, fac1);
        let mul02 = mul(vec3, fac2);
        let sub00 = sub(mul00, mul01);
        let add00 = add(sub00, mul02);
        let inv0 = mul(sign_b, add00);

        let mul03 = mul(vec0, fac0);
        let mul04 = mul(vec2, fac3);
        let mul05 = mul(vec3, fac4);
        let sub01 = sub(mul03, mul04);
        let add01 = add(sub01, mul05);
        let inv1 = mul(sign_a, add01);

        let mul06 = mul(vec0, fac1);
        let mul07 = mul(vec1, fac3);
        let mul08 = mul(vec3, fac5);
        let sub02 = sub(mul06, mul07);
        let add02 = add(sub02, mul08);
        let inv2 = mul(sign_b, add02);

        let mul09 = mul(vec0, fac2);
        let mul10 = mul(vec1, fac4);
        let mul11 = mul(vec2, fac5);
        let sub03 = sub(mul09, mul10);
        let add03 = add(sub03, mul11);
        let inv3 = mul(sign_a, add03);

        let row0 = sh(inv0, inv1, 0b00_00_00_00);
        let row1 = sh(inv2, inv3, 0b00_00_00_00);
        let row2 = sh(row0, row1, 0b10_00_10_00);

        let dot0 = dot4(x, row2);

        let rcp0 = set1_ps(dot0.recip());

        let (r0, r1, r2, r3) = (
            mul(inv0, rcp0),
            mul(inv1, rcp0),
            mul(inv2, rcp0),
            mul(inv3, rcp0),
        );

        Self {
            x_axis: Vec4::new(r0[0], r0[1], r0[2], r0[3]),
            y_axis: Vec4::new(r1[0], r1[1], r1[2], r1[3]),
            z_axis: Vec4::new(r2[0], r2[1], r2[2], r2[3]),
            w_axis: Vec4::new(r3[0], r3[1], r3[2], r3[3]),
        }
    }
}

// ---------------------------------------------------------------------------
// Vec3 —— `glam 0.32.1` `src/f32/vec3.rs`（非 SIMD 的普通类型）
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const ONE: Self = Self::new(1.0, 1.0, 1.0);
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);
    // ⚠ glam 那边 `NEG_X` 是**字面量** `-1.0`，不是"取负"（`src/f32/vec3.rs`）：
    //    `Vec3::ZERO - Vec3::X` 与 `Vec3::NEG_X` 在 `-0.0` 上不是同一个位模式，
    //    而 cube 那六面的 target/up 正是照字面量抄过来的（§109.2）。
    pub const NEG_X: Self = Self::new(-1.0, 0.0, 0.0);
    pub const NEG_Y: Self = Self::new(0.0, -1.0, 0.0);
    pub const NEG_Z: Self = Self::new(0.0, 0.0, -1.0);

    #[inline(always)]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline(always)]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v, z: v }
    }

    #[inline(always)]
    pub const fn from_array(a: [f32; 3]) -> Self {
        Self {
            x: a[0],
            y: a[1],
            z: a[2],
        }
    }

    #[inline(always)]
    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    /// `glam` `src/f32/vec3.rs:250`：`(x*x') + (y*y') + (z*z')`，左结合。
    #[inline]
    #[must_use]
    pub fn dot(self, rhs: Self) -> f32 {
        (self.x * rhs.x) + (self.y * rhs.y) + (self.z * rhs.z)
    }

    /// `glam` `src/f32/vec3.rs:264`
    #[inline]
    #[must_use]
    pub fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - rhs.y * self.z,
            y: self.z * rhs.x - rhs.z * self.x,
            z: self.x * rhs.y - rhs.x * self.y,
        }
    }

    /// `glam` `src/f32/vec3.rs:554`
    #[inline]
    #[must_use]
    pub fn length(self) -> f32 {
        f32::sqrt(self.dot(self))
    }

    /// `glam` `src/f32/vec3.rs:573`
    #[inline]
    #[must_use]
    pub fn length_recip(self) -> f32 {
        self.length().recip()
    }

    /// `glam` `src/f32/vec3.rs:626`
    #[inline]
    #[must_use]
    pub fn normalize(self) -> Self {
        self.mul(self.length_recip())
    }

    /// `glam` `src/f32/vec3.rs:657`
    #[inline]
    #[must_use]
    pub fn normalize_or_zero(self) -> Self {
        let rcp = self.length_recip();
        if rcp.is_finite() && rcp > 0.0 {
            self * rcp
        } else {
            Self::ZERO
        }
    }

    /// `glam` `src/f32/vec3.rs:641`（`try_normalize`，`look_to` 用它）
    #[inline]
    #[must_use]
    pub fn try_normalize(self) -> Option<Self> {
        let rcp = self.length_recip();
        if rcp.is_finite() && rcp > 0.0 {
            Some(self * rcp)
        } else {
            None
        }
    }

    #[inline]
    #[must_use]
    pub fn mul(self, rhs: f32) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }

    #[inline]
    #[must_use]
    pub fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl core::ops::Add for Vec3 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Vec3::add(self, rhs)
    }
}

impl core::ops::Sub for Vec3 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl core::ops::Mul<f32> for Vec3 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Vec3::mul(self, rhs)
    }
}

impl core::ops::Neg for Vec3 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

// ---------------------------------------------------------------------------
// Mat3 —— `glam 0.32.1` `src/f32/mat3.rs`
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub z_axis: Vec3,
}

impl Mat3 {
    pub const IDENTITY: Self = Self::from_cols(Vec3::X, Vec3::Y, Vec3::Z);

    #[inline(always)]
    pub const fn from_cols(x_axis: Vec3, y_axis: Vec3, z_axis: Vec3) -> Self {
        Self {
            x_axis,
            y_axis,
            z_axis,
        }
    }

    /// `glam` `src/f32/mat3.rs:212`（与 `src/f32/sse2/mat3a.rs:278` 的
    /// `Mat3A::from_quat` 是同一串算式）
    #[inline]
    #[must_use]
    pub fn from_quat(rotation: Quat) -> Self {
        let x2 = rotation.x + rotation.x;
        let y2 = rotation.y + rotation.y;
        let z2 = rotation.z + rotation.z;
        let xx = rotation.x * x2;
        let xy = rotation.x * y2;
        let xz = rotation.x * z2;
        let yy = rotation.y * y2;
        let yz = rotation.y * z2;
        let zz = rotation.z * z2;
        let wx = rotation.w * x2;
        let wy = rotation.w * y2;
        let wz = rotation.w * z2;

        Self::from_cols(
            Vec3::new(1.0 - (yy + zz), xy + wz, xz - wy),
            Vec3::new(xy - wz, 1.0 - (xx + zz), yz + wx),
            Vec3::new(xz + wy, yz - wx, 1.0 - (xx + yy)),
        )
    }

    /// `glam` `src/f32/mat3.rs:493`
    #[inline]
    #[must_use]
    pub fn transpose(&self) -> Self {
        Self {
            x_axis: Vec3::new(self.x_axis.x, self.y_axis.x, self.z_axis.x),
            y_axis: Vec3::new(self.x_axis.y, self.y_axis.y, self.z_axis.y),
            z_axis: Vec3::new(self.x_axis.z, self.y_axis.z, self.z_axis.z),
        }
    }

    /// `glam` `src/f32/mat3.rs:677`
    #[inline]
    #[must_use]
    pub fn mul_vec3(&self, rhs: Vec3) -> Vec3 {
        let mut res = self.x_axis.mul(rhs.x);
        res = res.add(self.y_axis.mul(rhs.y));
        res = res.add(self.z_axis.mul(rhs.z));
        res
    }

    /// `glam` `src/f32/mat3.rs:930`（`impl Mul for Mat3`，逐列 `mul_vec3`）
    #[inline]
    #[must_use]
    pub fn mul_mat3(&self, rhs: &Mat3) -> Mat3 {
        Mat3::from_cols(
            self.mul_vec3(rhs.x_axis),
            self.mul_vec3(rhs.y_axis),
            self.mul_vec3(rhs.z_axis),
        )
    }
}

// ---------------------------------------------------------------------------
// Quat —— `glam 0.32.1` `src/f32/sse2/quat.rs`
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self::from_xyzw(0.0, 0.0, 0.0, 1.0);

    #[inline(always)]
    pub const fn from_xyzw(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// `glam` `src/f32/sse2/quat.rs:178`
    #[inline]
    #[must_use]
    pub fn from_rotation_y(angle: f32) -> Self {
        let (s, c) = f32::sin_cos(angle * 0.5);
        Self::from_xyzw(0.0, s, 0.0, c)
    }

    /// `glam` `src/f32/sse2/quat.rs:277`
    #[inline]
    #[must_use]
    pub fn from_mat3(mat: &Mat3) -> Self {
        Self::from_rotation_axes(mat.x_axis, mat.y_axis, mat.z_axis)
    }

    /// `glam` `src/f32/sse2/quat.rs:208` `from_rotation_axes`（Shepperd /
    /// DirectXMath `XMQuaternionRotationMatrix`）：分支条件、四个 `four_*sq`、
    /// 每个分支自己的 `inv4* = 0.5 / sqrt(four_*sq)` 与分量顺序都照抄。
    #[inline]
    #[must_use]
    pub fn from_rotation_axes(x_axis: Vec3, y_axis: Vec3, z_axis: Vec3) -> Self {
        let (m00, m01, m02) = (x_axis.x, x_axis.y, x_axis.z);
        let (m10, m11, m12) = (y_axis.x, y_axis.y, y_axis.z);
        let (m20, m21, m22) = (z_axis.x, z_axis.y, z_axis.z);
        if m22 <= 0.0 {
            // x^2 + y^2 >= z^2 + w^2
            let dif10 = m11 - m00;
            let omm22 = 1.0 - m22;
            if dif10 <= 0.0 {
                // x^2 >= y^2
                let four_xsq = omm22 - dif10;
                let inv4x = 0.5 / f32::sqrt(four_xsq);
                Self::from_xyzw(
                    four_xsq * inv4x,
                    (m01 + m10) * inv4x,
                    (m02 + m20) * inv4x,
                    (m12 - m21) * inv4x,
                )
            } else {
                // y^2 >= x^2
                let four_ysq = omm22 + dif10;
                let inv4y = 0.5 / f32::sqrt(four_ysq);
                Self::from_xyzw(
                    (m01 + m10) * inv4y,
                    four_ysq * inv4y,
                    (m12 + m21) * inv4y,
                    (m20 - m02) * inv4y,
                )
            }
        } else {
            // z^2 + w^2 >= x^2 + y^2
            let sum10 = m11 + m00;
            let opm22 = 1.0 + m22;
            if sum10 <= 0.0 {
                // z^2 >= w^2
                let four_zsq = opm22 - sum10;
                let inv4z = 0.5 / f32::sqrt(four_zsq);
                Self::from_xyzw(
                    (m02 + m20) * inv4z,
                    (m12 + m21) * inv4z,
                    four_zsq * inv4z,
                    (m01 - m10) * inv4z,
                )
            } else {
                // w^2 >= z^2
                let four_wsq = opm22 + sum10;
                let inv4w = 0.5 / f32::sqrt(four_wsq);
                Self::from_xyzw(
                    (m12 - m21) * inv4w,
                    (m20 - m02) * inv4w,
                    (m01 - m10) * inv4w,
                    four_wsq * inv4w,
                )
            }
        }
    }
}

impl Mat4 {
    /// `glam` `src/f32/sse2/mat4.rs:192` `quat_to_axes` —— 与
    /// `Mat3A::from_quat`（`src/f32/sse2/mat3a.rs:278`）同一串算式，只是列是 `Vec4`
    /// 且 `w = 0.0`。
    #[inline]
    #[must_use]
    fn quat_to_axes(rotation: Quat) -> (Vec4, Vec4, Vec4) {
        let (x, y, z, w) = (rotation.x, rotation.y, rotation.z, rotation.w);
        let x2 = x + x;
        let y2 = y + y;
        let z2 = z + z;
        let xx = x * x2;
        let xy = x * y2;
        let xz = x * z2;
        let yy = y * y2;
        let yz = y * z2;
        let zz = z * z2;
        let wx = w * x2;
        let wy = w * y2;
        let wz = w * z2;

        (
            Vec4::new(1.0 - (yy + zz), xy + wz, xz - wy, 0.0),
            Vec4::new(xy - wz, 1.0 - (xx + zz), yz + wx, 0.0),
            Vec4::new(xz + wy, yz - wx, 1.0 - (xx + yy), 0.0),
        )
    }

    /// `glam` `src/f32/sse2/mat4.rs:246`
    #[inline]
    #[must_use]
    pub fn from_rotation_translation(rotation: Quat, translation: Vec3) -> Self {
        let (x_axis, y_axis, z_axis) = Self::quat_to_axes(rotation);
        Self::from_cols(
            x_axis,
            y_axis,
            z_axis,
            Vec4::new(translation.x, translation.y, translation.z, 1.0),
        )
    }

    /// `glam` `src/f32/sse2/mat4.rs:226`（等价于 `Affine3A::from_scale_rotation_translation`
    /// `src/f32/affine3a.rs:253` + `From<Affine3A> for Mat4` `src/f32/affine3a.rs:691`）
    #[inline]
    #[must_use]
    pub fn from_scale_rotation_translation(scale: Vec3, rotation: Quat, translation: Vec3) -> Self {
        let (x_axis, y_axis, z_axis) = Self::quat_to_axes(rotation);
        Self::from_cols(
            x_axis.mul(scale.x),
            y_axis.mul(scale.y),
            z_axis.mul(scale.z),
            Vec4::new(translation.x, translation.y, translation.z, 1.0),
        )
    }

    /// **法线矩阵**：`Affine3A::from(world_from_local).inverse().matrix3.transpose()`。
    ///
    /// 逐行移植自 Bevy 那条路（原文行号）：
    /// - `bevy_pbr-0.19.1/src/render/mesh.rs:664` 调 `world_from_local.inverse_transpose_3x3()`；
    /// - 那是 `bevy_math-0.19.1/src/affine3.rs:37-43` 的扩展 trait：
    ///   `Affine3A::from(self).inverse().matrix3.transpose()`；
    /// - `glam 0.32.1` `src/f32/affine3a.rs:470-479`（`Affine3A::inverse` 取 `matrix3.inverse()`）
    ///   + `src/f32/sse2/mat3a.rs:631`（`Mat3A::inverse` → `inverse_checked::<false>`，`:597-620`）。
    ///
    /// ⚠ 为什么不能拿 `world_from_local` 的 3×3 凑合（本仓库原先就是这么写的）：
    /// **数学等价、浮点不等价** —— 均匀缩放 1.0 时 `inv(M)ᵀ == M` 成立，但两条算术路径
    /// （一个走 `Mat3A::inverse`，一个只是取列）给出的是**不同的末位**。这一族在本仓库
    /// 已经现形四次（§110.1.1 / §116 / §118 / §132），第五次就是它。
    ///
    /// 返回**三列**（列主序），与 WGSL 的 `mat3x3<f32>` 同序：着色器算 `normalize(M * n)`。
    #[inline]
    #[must_use]
    pub fn normal_matrix_3x3(&self) -> [Vec3; 3] {
        // `Mat3A::inverse_checked::<false>`：三个叉积 + 行列式 + 每个分量乘 `det.recip()`。
        let (x, y, z) = (
            Vec3::new(self.x_axis.x, self.x_axis.y, self.x_axis.z),
            Vec3::new(self.y_axis.x, self.y_axis.y, self.y_axis.z),
            Vec3::new(self.z_axis.x, self.z_axis.y, self.z_axis.z),
        );
        let tmp0 = y.cross(z);
        let tmp1 = z.cross(x);
        let tmp2 = x.cross(y);
        let det = z.dot(tmp2);
        // `Vec3A::splat(det.recip())`：`recip()` 就是 `1.0 / x`（f32 除法）。
        let inv_det = 1.0 / det;
        // ⚠ 这里**不再转置**：`Mat3A::inverse_checked` 内部已经 `.transpose()` 过一次
        //    （`src/f32/sse2/mat3a.rs:617` 那句 `…from_cols(…).transpose()`），而 Bevy 那一行
        //    （`bevy_math-0.19.1/src/affine3.rs:38`）又 `.matrix3.transpose()` 一次 ——
        //    两次转置抵消 ⇒ 给出去的就是这三列。判据（真 glam 的**那条表达式**）钉着这一点。
        [tmp0.mul(inv_det), tmp1.mul(inv_det), tmp2.mul(inv_det)]
    }

    /// `glam` `src/f32/sse2/mat4.rs:1408`
    #[inline]
    #[must_use]
    pub fn mul_vec4(&self, rhs: Vec4) -> Vec4 {
        let mut res = self.x_axis.mul(rhs.x);
        res = res.add(self.y_axis.mul(rhs.y));
        res = res.add(self.z_axis.mul(rhs.z));
        res = res.add(self.w_axis.mul(rhs.w));
        res
    }

    /// `glam` `src/f32/sse2/mat4.rs:1431` -> `:1671`
    #[inline]
    #[must_use]
    pub fn mul_mat4(&self, rhs: &Mat4) -> Mat4 {
        Mat4::from_cols(
            self.mul_vec4(rhs.x_axis),
            self.mul_vec4(rhs.y_axis),
            self.mul_vec4(rhs.z_axis),
            self.mul_vec4(rhs.w_axis),
        )
    }

    /// `glam` `src/f32/sse2/mat4.rs:1197`（`math::tan` = `f32::tan`）
    #[inline]
    #[must_use]
    pub fn perspective_infinite_reverse_rh(
        fov_y_radians: f32,
        aspect_ratio: f32,
        z_near: f32,
    ) -> Self {
        let f = 1.0 / f32::tan(0.5 * fov_y_radians);
        Self::from_cols(
            Vec4::new(f / aspect_ratio, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, z_near, 0.0),
        )
    }
}

impl Vec4 {
    /// `glam` `src/f32/sse2/vec4.rs`：`impl Add for Vec4`（`_mm_add_ps`）
    #[inline]
    #[must_use]
    pub fn add(self, rhs: Self) -> Self {
        Self::new(
            self.x + rhs.x,
            self.y + rhs.y,
            self.z + rhs.z,
            self.w + rhs.w,
        )
    }

    /// `glam` `src/f32/sse2/vec4.rs`：`impl Mul<f32> for Vec4`
    #[inline]
    #[must_use]
    pub fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs, self.w * rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::{Mat4, Vec4};

    /// `target/oracle/bevy-view-vectors.txt` 的 6 个用例，(输入位模式, 期望位模式)，列主序。
    const CASES: [(&str, &str); 6] = [
        (
            "3F800000 00000000 00000000 00000000 00000000 3F7C2F4D BE302108 00000000 \
             00000000 3E302108 3F7C2F4D 00000000 00000000 3F0CCCCD 4049999A 3F800000",
            "3F800000 80000000 00000000 80000000 80000000 3F7C2F4F 3E302109 00000000 \
             00000000 BE302109 3F7C2F4F 80000000 00000000 B3800001 C04CA664 3F800000",
        ),
        (
            "3F800000 80000000 00000000 00000000 00000000 3F800000 00000000 00000000 \
             80000000 00000000 3F800000 00000000 00000000 00000000 40600000 3F800000",
            "3F800000 80000000 00000000 80000000 80000000 3F800000 80000000 00000000 \
             00000000 80000000 3F800000 80000000 80000000 00000000 C0600000 3F800000",
        ),
        (
            "3F51B3F3 32800000 BF12D5E7 00000000 BE48E205 3F708FB2 BE8F71FC 00000000 \
             3F09FAF5 3EAF1D44 3F450E69 00000000 3FA5938D 3F52231E 3FEC77B2 3F800000",
            "3F51B3F3 BE48E205 3F09FAF4 80000000 80000000 3F708FB2 3EAF1D44 00000000 \
             BF12D5E8 BE8F71FC 3F450E69 80000000 3165C024 338D8F73 C019999A 3F800000",
        ),
        (
            "BF000002 33000000 3F5DB3D8 00000000 BF359BAA 3F12D5E6 BED1B3F5 00000000 \
             BEFE53A0 BF51B3F4 BE92D5E8 00000000 C03EBEB8 C09D46F6 BFDC40DE 3F800000",
            "BF000000 BF359BAA BEFE539C 80000000 327FFFFE 3F12D5E7 BF51B3F3 80000000 \
             3F5DB3D6 BED1B3F5 BE92D5E6 80000000 3470FA78 B4CED9EC C0BFFFFF 3F800000",
        ),
        (
            "3F92D986 00000000 BF77612C 00000000 00000000 3F000000 00000000 00000000 \
             3FB988E1 00000000 3FDC4649 00000000 BF800000 40000000 3E800000 3F800000",
            "3F028877 80000000 3E929866 80000000 80000000 40000000 80000000 00000000 \
             BEDBE499 80000000 3EAE0B4A 80000000 3F1E050B C0800000 3E4E2B27 3F800000",
        ),
        (
            "3F800000 40000000 40400000 40800000 3F000000 BF800000 3E800000 40000000 \
             C0000000 3F400000 3FC00000 BF000000 40400000 3E000000 BF800000 3F800000",
            "BF2559CF 3F60F1BB 4022D532 40064B8A 3F6B0432 BFC97150 C038A7DE BFFBCDA4 \
             BF88D028 3FDBE880 4093B97F 40497150 3F410C97 BF38A7DE C029F79B BFF368EB",
        ),
    ];

    fn parse16(s: &str) -> [f32; 16] {
        let mut it = s.split_whitespace();
        let mut out = [0.0f32; 16];
        let mut n = 0;
        while let Some(tok) = it.next() {
            out[n] = f32::from_bits(u32::from_str_radix(tok, 16).expect("hex"));
            n += 1;
        }
        assert_eq!(n, 16, "要 16 个位模式");
        out
    }

    fn bits16(m: &Mat4) -> [f32; 16] {
        let c = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
        let mut out = [0.0f32; 16];
        for i in 0..4 {
            out[i * 4] = c[i].x;
            out[i * 4 + 1] = c[i].y;
            out[i * 4 + 2] = c[i].z;
            out[i * 4 + 3] = c[i].w;
        }
        out
    }

    #[test]
    fn inverse_matches_glam_bit_for_bit() {
        let mut failed = Vec::new();
        for (i, (inp, outp)) in CASES.iter().enumerate() {
            let a = parse16(inp);
            let want = parse16(outp);
            let m = Mat4::from_cols(
                Vec4::new(a[0], a[1], a[2], a[3]),
                Vec4::new(a[4], a[5], a[6], a[7]),
                Vec4::new(a[8], a[9], a[10], a[11]),
                Vec4::new(a[12], a[13], a[14], a[15]),
            );
            let got = bits16(&m.inverse());
            let mut bad = Vec::new();
            for k in 0..16 {
                if got[k].to_bits() != want[k].to_bits() {
                    bad.push(format!(
                        "[{k}] got {:08X} want {:08X}",
                        got[k].to_bits(),
                        want[k].to_bits()
                    ));
                }
            }
            if bad.is_empty() {
                println!("case {i}: PASS (16/16 bit patterns)");
            } else {
                println!("case {i}: FAIL {}", bad.join(", "));
                failed.push(i);
            }
        }
        assert!(failed.is_empty(), "cases failed: {failed:?}");
    }

    /// **`world_from_local` 那条路**（`Quat::from_xyzw` + `Mat4::from_scale_rotation_translation`
    /// + `Mat4::mul_mat4`）与**真的 glam** 逐位相同。
    ///
    /// 为什么单列这一条：`inverse` 早就有逐位判据，而"物体变换"这条路只有**逐行移植的注释**
    /// 没有判据。它一旦差一个末位，症状是**盘内散落的 ±1**（法线/位置各偏一丝 ⇒ 着色偏一丝
    /// ⇒ 只有恰好压在舍入边界上的那些像素翻一格），而轮廓、矩阵、贴图、采样器**全都是对的**
    /// —— 这一族本仓库已经踩过四次（§110.1.1 / §116 / §118 / §132），每次都是"数学等价、
    /// 浮点不等价"。
    ///
    /// ⚠ 比的是**真的 glam crate**（只在 dev-dependencies 里），不是把同一段公式再抄一遍：
    /// 抄一遍只能证明"我抄得跟我抄的一样"。
    #[test]
    fn the_object_transform_matches_glam_bit_for_bit() {
        use crate::mat4::{Mat4, Quat, Vec3};
        let rotations = [
            // 文档里那一颗（`orbit-bare-nolight` 的 planet）：
            [0.16918235_f32, 0.0, 0.0, 0.9855848],
            [0.0, 0.0, 0.0, 1.0],
            [0.70710677, 0.0, 0.0, 0.70710677],
            [0.1, -0.2, 0.3, 0.92736185],
            [-0.5, 0.5, 0.5, 0.5],
        ];
        let scales = [
            [1.0_f32, 1.0, 1.0],
            [1.14, 1.14, 1.14],
            [0.5, 2.0, 1.25],
        ];
        let translations = [[0.0_f32, 0.0, 0.0], [1.0, -2.0, 0.5]];
        let mut mismatches = Vec::new();
        for rotation in rotations {
            for scale in scales {
                for translation in translations {
                    let ours = Mat4::from_scale_rotation_translation(
                        Vec3::new(scale[0], scale[1], scale[2]),
                        Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
                        Vec3::new(translation[0], translation[1], translation[2]),
                    );
                    let theirs = glam::Mat4::from_scale_rotation_translation(
                        glam::Vec3::new(scale[0], scale[1], scale[2]),
                        glam::Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
                        glam::Vec3::new(translation[0], translation[1], translation[2]),
                    );
                    for (column, their_column) in
                        bits16(&ours).iter().zip(theirs.to_cols_array().iter())
                    {
                        if column.to_bits() != their_column.to_bits() {
                            mismatches.push(format!(
                                "rot {rotation:?} scale {scale:?} trans {translation:?}：\
                                 got {:08X} want {:08X}",
                                column.to_bits(),
                                their_column.to_bits()
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "物体变换与 glam 不逐位相同（{} 处）：{mismatches:#?}",
            mismatches.len()
        );
    }

    /// 同一件事再来一遍：**列相乘**（`clip_from_world = clip_from_view × view_from_world`）    /// 也要与 glam 逐位相同。它是 `MeshStage::view_proj` 那颗数。
    #[test]
    fn the_matrix_product_matches_glam_bit_for_bit() {
        use crate::mat4::{Mat4, Quat, Vec3};
        let rotations = [
            [0.16918235_f32, 0.0, 0.0, 0.9855848],
            [0.3, 0.4, -0.2, 0.84261495],
        ];
        let mut mismatches = Vec::new();
        for rotation in rotations {
            let ours = Mat4::from_scale_rotation_translation(
                Vec3::new(1.0, 1.0, 1.0),
                Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
                Vec3::new(0.0, 0.0, 0.0),
            );
            let theirs = glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::ONE,
                glam::Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
                glam::Vec3::ZERO,
            );
            let product = Mat4::mul_mat4(&ours, &ours);
            let their_product = theirs * theirs;
            for (column, their_column) in
                bits16(&product).iter().zip(their_product.to_cols_array().iter())
            {
                if column.to_bits() != their_column.to_bits() {
                    mismatches.push(format!(
                        "rot {rotation:?}：got {:08X} want {:08X}",
                        column.to_bits(),
                        their_column.to_bits()
                    ));
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "矩阵相乘与 glam 不逐位相同（{} 处）：{mismatches:#?}",
            mismatches.len()
        );
    }

    /// **法线矩阵**与 Bevy 那条路逐位相同：`Affine3A::from(m).inverse().matrix3.transpose()`。
    ///
    /// ⚠ 判据里比的必须是**这条表达式**（用真 glam 写成 Bevy 那一行），不能比"我认为等价的
    /// 另一条"：`world_from_local` 的 3×3 在均匀缩放下数学上就等于它，而**浮点上不等**
    /// —— 这一条判据存在的全部理由就是把这两者分开。
    #[test]
    fn the_normal_matrix_matches_bevys_expression_bit_for_bit() {
        use crate::mat4::{Mat4, Quat, Vec3};
        let rotations = [
            [0.16918235_f32, 0.0, 0.0, 0.9855848],
            [0.0, 0.0, 0.0, 1.0],
            [0.70710677, 0.0, 0.0, 0.70710677],
            [0.1, -0.2, 0.3, 0.92736185],
        ];
        let scales = [[1.0_f32, 1.0, 1.0], [1.14, 1.14, 1.14], [0.5, 2.0, 1.25]];
        let translations = [[0.0_f32, 0.0, 0.0], [1.0, -2.0, 0.5]];
        let mut mismatches = Vec::new();
        for rotation in rotations {
            for scale in scales {
                for translation in translations {
                    let ours_matrix = Mat4::from_scale_rotation_translation(
                        Vec3::new(scale[0], scale[1], scale[2]),
                        Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
                        Vec3::new(translation[0], translation[1], translation[2]),
                    );
                    let theirs_matrix = glam::Mat4::from_scale_rotation_translation(
                        glam::Vec3::new(scale[0], scale[1], scale[2]),
                        glam::Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
                        glam::Vec3::new(translation[0], translation[1], translation[2]),
                    );
                    // Bevy 那一行（`bevy_math-0.19.1/src/affine3.rs:37-43`）：
                    let theirs = glam::Affine3A::from_mat4(theirs_matrix)
                        .inverse()
                        .matrix3
                        .transpose();
                    let ours = ours_matrix.normal_matrix_3x3();
                    let want = [
                        [theirs.x_axis.x, theirs.x_axis.y, theirs.x_axis.z],
                        [theirs.y_axis.x, theirs.y_axis.y, theirs.y_axis.z],
                        [theirs.z_axis.x, theirs.z_axis.y, theirs.z_axis.z],
                    ];
                    for column in 0..3 {
                        for row in 0..3 {
                            let got = ours[column].to_array()[row];
                            if got.to_bits() != want[column][row].to_bits() {
                                mismatches.push(format!(
                                    "rot {rotation:?} scale {scale:?} trans {translation:?} \
                                     列{column} 行{row}：got {:08X} want {:08X}",
                                    got.to_bits(),
                                    want[column][row].to_bits()
                                ));
                            }
                        }
                    }
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "法线矩阵与 Bevy 那条路不逐位相同（{} 处）：{mismatches:#?}",
            mismatches.len()
        );
    }
}


impl Mat4 {
    /// 全零矩阵。`camera.rs` 的测试拿它当「没算出来」的哨兵。
    /// 用结构体字面量而不是 `Vec4::new(..)`：这样 `const` 一定成立，
    /// 不必去猜 `Vec4::new` 是不是 `const fn`。
    pub const ZERO: Self = Self {
        x_axis: Vec4 { x: 0.0, y: 0.0, z: 0.0, w: 0.0 },
        y_axis: Vec4 { x: 0.0, y: 0.0, z: 0.0, w: 0.0 },
        z_axis: Vec4 { x: 0.0, y: 0.0, z: 0.0, w: 0.0 },
        w_axis: Vec4 { x: 0.0, y: 0.0, z: 0.0, w: 0.0 },
    };
}
