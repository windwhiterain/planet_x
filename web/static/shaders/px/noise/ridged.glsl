#ifndef PX_NOISE_RIDGED_GLSL
#define PX_NOISE_RIDGED_GLSL
#include <px/noise/pars.glsl>
#include <px/noise/vnoise.glsl>

  // 山脊噪声：1-|2n-1|，八度加权更陡，用来做大陆山系 / 日珥丝 / 星云纤维。
  float ridged(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmOct; i++){
      float n = 1.0 - abs(2.0 * vnoise(p) - 1.0);
      s += a * n * n;
      p *= 2.13; a *= 0.5;
    }
    return s;
  }
#endif // PX_NOISE_RIDGED_GLSL
