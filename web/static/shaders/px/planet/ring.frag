#ifndef PX_PLANET_RING_FRAG
#define PX_PLANET_RING_FRAG
#include <px/noise/pars.glsl>

  precision highp float;

  uniform vec3  uCol;
  uniform vec3  uCol2;
  uniform float uInner;
  uniform float uOuter;
  uniform vec3  uCenter;      // 行星中心（世界）
  uniform float uPlanetR;
  varying vec3 vWorldPos;
  varying vec3 vLocal;

  float hash1(float n){ return fract(sin(n * 91.3458) * 47453.5453); }

  // 径向密度：真实环不是等距条纹——用噪声调制疏密，再挖出卡西尼缝/恩克缝。
  float density(float t){
    float ringlets = 0.5 + 0.5 * sin(t * 118.0 + hash1(floor(t * 30.0)) * 6.283);
    float a = 0.58 + 0.42 * ringlets;
    // 内圈 C 环：稀薄。
    a *= 0.42 + 0.58 * smoothstep(0.05, 0.30, t);
    // 卡西尼缝。
    a *= 1.0 - 0.88 * smoothstep(0.415, 0.445, t) * (1.0 - smoothstep(0.478, 0.508, t));
    // 恩克缝（外侧细缝）。
    a *= 1.0 - 0.55 * smoothstep(0.855, 0.872, t) * (1.0 - smoothstep(0.888, 0.905, t));
    // 一条更细的次级缝。
    a *= 1.0 - 0.35 * smoothstep(0.66, 0.672, t) * (1.0 - smoothstep(0.682, 0.695, t));
    // ⚠ **内外缘必须淡出到 0**。原来 t=0 处还剩 0.42、t=1 处直接 discard，于是环的内缘和
    // 外缘都是**硬切边** —— 掠射视角下那两条边就是两条笔直的硬线，看起来像"接缝"。
    // 真实环的外缘是逐渐稀薄的（过了 F 环很快就没了），内缘也一样。
    a *= smoothstep(0.0, 0.09, t) * (1.0 - smoothstep(0.88, 1.0, t));
    return clamp(a, 0.0, 1.0);
  }

  void main(){
    float rad = length(vLocal.xy);
    float t = (rad - uInner) / max(uOuter - uInner, 1e-4);
    if (t < 0.0 || t > 1.0) discard;
    float dens = density(t);

    // 前后向散射：相位角接近 0（太阳在相机背后）时环变亮——「冲日效应」，真实土星环最
    // 引人注目的一条。用 HG 相函数的一个廉价近似。
    vec3 toSun = normalize(-vWorldPos);
    vec3 toCam = normalize(cameraPosition - vWorldPos);
    float cosPhase = dot(toSun, toCam);
    float phase = 0.55 + 0.75 * pow(clamp(1.0 + cosPhase, 0.0, 2.0) * 0.5, 2.2);

    // 行星在环上的投影：从环上这点朝太阳的射线，是否穿过行星球。
    vec3 w = uCenter - vWorldPos;
    float tc = dot(w, toSun);
    float shadow = 0.0;
    if (tc > 0.0) {
      float d2 = dot(w, w) - tc * tc;
      float rr = uPlanetR;
      // 半影：太阳是有限大的，影子边缘有一段过渡。
      float pen = max(rr * 0.10, 1e-3);
      shadow = 1.0 - smoothstep(rr * rr, (rr + pen) * (rr + pen), d2);
    }
    // 环自身的厚度：掠射角下变亮（视线穿过更多粒子）。
    float graze = 1.0 - abs(dot(normalize(vec3(0.0, 1.0, 0.0)), toCam));

    vec3 col = mix(uCol, uCol2, smoothstep(0.35, 0.85, t) * 0.6);
    float alpha = dens * (1.0 - shadow * 0.86) * (0.55 + 0.45 * phase);
    alpha *= 0.75 + 0.55 * graze;
    gl_FragColor = vec4(col * (0.55 + 0.75 * phase) * (1.0 - shadow * 0.72), clamp(alpha, 0.0, 1.0));
  }
#endif // PX_PLANET_RING_FRAG
