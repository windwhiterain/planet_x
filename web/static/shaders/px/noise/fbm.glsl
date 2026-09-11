#ifndef PX_NOISE_FBM_GLSL
#define PX_NOISE_FBM_GLSL
#include <px/noise/pars.glsl>
#include <px/noise/vnoise.glsl>

  // 标准 fbm：uFbmOct 个八度，lacunarity 2.02（避开整数倍造成的格点对齐）。
  float fbm(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmOct; i++){ s += a * vnoise(p); p *= 2.02; a *= 0.5; }
    return s;
  }
#endif // PX_NOISE_FBM_GLSL
