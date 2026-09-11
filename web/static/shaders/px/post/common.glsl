#ifndef PX_POST_COMMON_GLSL
#define PX_POST_COMMON_GLSL
  float luma(vec3 c){ return dot(c, vec3(0.2126, 0.7152, 0.0722)); }
  // 带时间种子的白噪声（胶片颗粒 / 抖动）。
  float grainNoise(vec2 uv, float t){
    return fract(sin(dot(uv + t, vec2(12.9898, 78.233))) * 43758.5453);
  }
#endif // PX_POST_COMMON_GLSL
