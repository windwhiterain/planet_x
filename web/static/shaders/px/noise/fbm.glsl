#ifndef PX_NOISE_FBM_GLSL
#define PX_NOISE_FBM_GLSL
#include <px/noise/pars.glsl>
#include <px/noise/vnoise.glsl>
#include <px/noise/rot.glsl>

  // 标准 fbm：uFbmOct 个八度，lacunarity 2.02（避开整数倍造成的格点对齐）。
  // ⚠ 八度之间必须**乘上 `PX_NOISE_ROT`**（推导见 px/noise/rot.glsl）：不转的话所有八度的
  // 格点朝向相同，叠出来是一张**方格纸**而不是分形 —— 太阳米粒放大后能直接看到方格。
  float fbm(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmOct; i++){ s += a * vnoise(p); p = PX_NOISE_ROT * p * 2.02; a *= 0.5; }
    return s;
  }
#endif // PX_NOISE_FBM_GLSL
