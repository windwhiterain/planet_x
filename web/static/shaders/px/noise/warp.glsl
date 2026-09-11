#ifndef PX_NOISE_WARP_GLSL
#define PX_NOISE_WARP_GLSL
#include <px/noise/vnoise.glsl>

  // 域扰动（「流体感」的廉价做法）：把采样点 p 按一个位移场推一下再去采噪声。
  //
  // ⚠ 位移场**必须是单一八度 vnoise，不能是 fbm** —— 这条注释本来就写着「单一八度」，
  // 但代码曾经调的是 fbm()，那是个真 bug，现象是星球/星云上出现**带尖点的锐利折痕**
  // （金星、天王星、云层、星云都实测到；折痕上还会露出 accent 色）。
  //
  // 原因：映射 p ↦ p + disp 会不会**反折**，只取决于位移的**梯度** |∇disp| < 1。
  //   |∇disp| = amt · warpK · |∇vnoise|
  // 而 fbm 的梯度随八度数**线性增长**（第 k 个八度贡献 0.5·2.02^k·|∇vnoise| ≈ 常数
  // ⇒ n 个八度就是 n 倍）。于是「折没折」会随画质档位（uFbmOct 2..7）变化 ——
  // 低档位正常、高档位出折痕，是最难查的那类 bug。单层 vnoise 的梯度有界（Hermite
  // 导数峰值 1.5/轴），折叠只取决于 amt·warpK，**与档位无关**。
  //
  // 取值的硬约束：amt · warpK ≲ 0.3（留 1/2.6 的余量）。想要更强的扰动，不要加 amt，
  // 而是**降 warpK** —— 位移场比被采样的图案低频即可，位移相对「被采样图案的特征尺度」
  // 依旧可以很大，观感上照样是强扰动，但映射不反折。
  vec3 warp(vec3 p, float amt, float t, float warpK){
    vec3 q = vec3(vnoise(p * warpK + vec3(0.0, 0.0, t)),
                  vnoise(p * warpK + vec3(5.2, 1.3, t)),
                  vnoise(p * warpK + vec3(9.7, 4.1, t)));
    return p + (q - 0.5) * amt;
  }
#endif // PX_NOISE_WARP_GLSL
