#ifndef PX_NOISE_WARPT_GLSL
#define PX_NOISE_WARPT_GLSL
#include <px/noise/warp.glsl>

  // 折叠安全的域扰动：turb 是**湍流倍数**（1.0 = 调用点的基准口径，由 config 的
  // turbulence 提供）。amt 与 warpK 的**乘积**决定会不会折叠（见上面那段推导），
  // 所以 turb 只放大 amt，同时把 warpK **反比压小** ⇒ 乘积恒定、永不折叠。
  // 观感上「越湍流」= 位移场越低频（更扭曲的大尺度弯路），而不是「更碎」。
  // ⚠ baseAmt · baseK 必须是调用点算好的安全乘积（≈0.30），本函数不负责兜底。
  vec3 warpT(vec3 p, float baseAmt, float baseK, float t, float turb){
    float k = clamp(turb, 0.05, 3.0);
    return warp(p, baseAmt * k, t, baseK / k);
  }
#endif // PX_NOISE_WARPT_GLSL
