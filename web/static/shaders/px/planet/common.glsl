#ifndef PX_PLANET_COMMON_GLSL
#define PX_PLANET_COMMON_GLSL
#include <px/noise/vnoise.glsl>

  vec3 rotAxis(vec3 v, vec3 a, float ang){
    float c = cos(ang), s = sin(ang);
    return v * c + cross(a, v) * s + a * dot(a, v) * (1.0 - c);
  }
  vec3 hash3t(vec3 p){
    p = vec3(dot(p, vec3(127.1, 311.7, 74.7)),
             dot(p, vec3(269.5, 183.3, 246.1)),
             dot(p, vec3(113.5, 271.9, 124.6)));
    return fract(sin(p) * 43758.5453123);
  }
  // 法线差分专用的低八度 fbm。**上界也是 uniform**：原来手写成
  // 0.5*vnoise(p) + 0.25*vnoise(p*2.02) + 0.125*vnoise(p*4.08)，就是「手工展开 3 份
  // vnoise」，而它有 5 个调用点 ⇒ 15 份内联 vnoise。改成动态循环后只剩一份。
  uniform int uFbmFastOct;
  float fbmFast(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmFastOct; i++){ s += a * vnoise(p); p *= 2.02; a *= 0.5; }
    return s;
  }
  // 陨石坑场：每格最多一个坑，坑心被限制在格子内部（±0.16，影响半径 ≲0.34）⇒ 不需要查
  // 邻格，也不会有格边界的硬切。返回 = 碗(负) + 环脊(正)。
  float craterField(vec3 p, float S, float density){
    vec3 sp = p * S;
    vec3 cell = floor(sp);
    vec3 r = hash3t(cell);
    if (r.z > density) return 0.0;
    vec3 c = cell + 0.5 + (r - 0.5) * 0.32;   // 必须整体用 vec3：vec3 + vec2 不合法
    float d = length(sp - c);
    float rad = 0.14 + 0.10 * hash13(cell + 3.7);
    float t = d / rad;
    if (t > 1.7) return 0.0;
    float bowl = -smoothstep(0.0, 0.92, t);
    float rim = exp(-pow((t - 0.98) / 0.17, 2.0));
    return bowl * 0.55 + rim * 0.45;
  }
#endif // PX_PLANET_COMMON_GLSL
