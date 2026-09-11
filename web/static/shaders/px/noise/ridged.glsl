#ifndef PX_NOISE_RIDGED_GLSL
#define PX_NOISE_RIDGED_GLSL
#include <px/noise/pars.glsl>
#include <px/noise/vnoise.glsl>
#include <px/noise/rot.glsl>

  // 山脊噪声：1-|2n-1|，八度加权更陡，用来做大陆山系 / 日珥丝 / 星云纤维。
  // 八度之间换格子朝向的道理与常量见 fbm.glsl（**方格纸**那个坑）—— 这个函数尤其需要：
  // `n*n` 把 |2n-1| 的等值线**锐化**了，格子边界因此比 fbm 明显得多。
  float ridged(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmOct; i++){
      float n = 1.0 - abs(2.0 * vnoise(p) - 1.0);
      s += a * n * n;
      p = PX_NOISE_ROT * p * 2.13; a *= 0.5;
    }
    return s;
  }
#endif // PX_NOISE_RIDGED_GLSL
