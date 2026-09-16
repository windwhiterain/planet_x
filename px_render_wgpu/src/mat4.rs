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
}
