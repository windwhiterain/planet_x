#ifndef PX_NOISE_PERLIN_GLSL
#define PX_NOISE_PERLIN_GLSL
#include <px/noise/pars.glsl>
#include <px/noise/hash13.glsl>
#include <px/noise/rot.glsl>

  // --- 梯度噪声（Perlin）：**没有格点**，因此没有"方格纸" ------------------------
  // `vnoise` 是三线性插值的**格点**噪声，它的等值线在格子尺度上天然是轴向的方框；
  // 一个八度还好，`ridged` 的 `n*n` 把等值线一锐化，格子就变成了肉眼可见的**小方块**
  // （现象：太阳米粒放大后是一片方格；实测每块约 8 px —— 那正是 `p*SC` 里 SC=105 的格距）。
  // 只在八度之间转格子（px/noise/rot.glsl）治不了它：**基准八度**的格子还在。
  //
  // 梯度噪声的等值线是"围绕格点的斜切面"，没有轴向偏好 ⇒ 放大也看不出网格。
  // 代价与 vnoise 同阶：每个角点**一次** hash13（查 12 个固定方向之一），
  // 8 个角点 + 三次 mix，和 vnoise 的 8 次 hash13 差不多。
  //
  // 12 个方向是正二十面体的棱（Perlin 的 classic 取法）：它们是**单位向量**，
  // 于是梯度噪声天然落在 ±0.7 左右，配 `*0.72 + 0.5` 之后与 vnoise 的 0..1 同口径。
  const vec3 PX_GRAD12[12] = vec3[12](
    vec3( 1.0,  1.0,  0.0), vec3(-1.0,  1.0,  0.0),
    vec3( 1.0, -1.0,  0.0), vec3(-1.0, -1.0,  0.0),
    vec3( 1.0,  0.0,  1.0), vec3(-1.0,  0.0,  1.0),
    vec3( 1.0,  0.0, -1.0), vec3(-1.0,  0.0, -1.0),
    vec3( 0.0,  1.0,  1.0), vec3( 0.0, -1.0,  1.0),
    vec3( 0.0,  1.0, -1.0), vec3( 0.0, -1.0, -1.0));

  vec3 pgrad(vec3 i){
    return PX_GRAD12[int(hash13(i) * 11.999)];
  }

  float pnoise(vec3 p){
    vec3 i = floor(p), f = fract(p);
    // 五次淡入（Perlin 的 smootherstep）：一阶、二阶导都连续 ⇒ 没有格点上的方向感
    vec3 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    float n000 = dot(pgrad(i + vec3(0.0, 0.0, 0.0)), f - vec3(0.0, 0.0, 0.0));
    float n100 = dot(pgrad(i + vec3(1.0, 0.0, 0.0)), f - vec3(1.0, 0.0, 0.0));
    float n010 = dot(pgrad(i + vec3(0.0, 1.0, 0.0)), f - vec3(0.0, 1.0, 0.0));
    float n110 = dot(pgrad(i + vec3(1.0, 1.0, 0.0)), f - vec3(1.0, 1.0, 0.0));
    float n001 = dot(pgrad(i + vec3(0.0, 0.0, 1.0)), f - vec3(0.0, 0.0, 1.0));
    float n101 = dot(pgrad(i + vec3(1.0, 0.0, 1.0)), f - vec3(1.0, 0.0, 1.0));
    float n011 = dot(pgrad(i + vec3(0.0, 1.0, 1.0)), f - vec3(0.0, 1.0, 1.0));
    float n111 = dot(pgrad(i + vec3(1.0, 1.0, 1.0)), f - vec3(1.0, 1.0, 1.0));
    return mix(mix(mix(n000, n100, u.x), mix(n010, n110, u.x), u.y),
               mix(mix(n001, n101, u.x), mix(n011, n111, u.x), u.y), u.z) * 0.72 + 0.5;
  }

  // 与 fbm/ridged **同口径**（同样的八度权重与 lacunarity），只是底层换成梯度噪声。
  float fbmP(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmOct; i++){ s += a * pnoise(p); p = PX_NOISE_ROT * p * 2.02; a *= 0.5; }
    return s;
  }

  // 山脊：`pow(1-|2n-1|, 2)`，与 ridged() 一致。梯度噪声下它是**连绵的脊**（不是一排方框）。
  float ridgedP(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmOct; i++){
      float n = 1.0 - abs(2.0 * pnoise(p) - 1.0);
      s += a * n * n;
      p = PX_NOISE_ROT * p * 2.13; a *= 0.5;
    }
    return s;
  }
#endif // PX_NOISE_PERLIN_GLSL
