#ifndef PX_SUN_SUN_PHOTO_FRAG
#define PX_SUN_SUN_PHOTO_FRAG
#include <px/noise/fbm.glsl>
#include <px/noise/ridged.glsl>

  precision highp float;

  uniform float uTime;
  uniform float uIntensity;
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;

  void main(){
    vec3 p = normalize(vObjPos);
    vec3 n = normalize(vNormal);
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float mu = clamp(dot(n, viewDir), 0.0, 1.0);

    // 米粒组织：两套不同尺度/相位对流的噪声交叉淡入淡出，做出「沸腾」而不是「飘动」。
    float t = uTime;
    float ph = 0.5 + 0.5 * sin(t * 0.55);
    float SC = 95.0;                       // 米粒的基准空间频率（见上，26 时像海绵）
    float g1 = 1.0 - ridged(p * SC + vec3(0.0, 0.0, t * 0.35));
    float g2 = 1.0 - ridged(p * SC + vec3(37.0, 11.0, t * 0.35 + 41.0));
    float gran = mix(g1, g2, ph);
    // 超米粒：更大尺度的对流胞，给米粒组织一个「群」的结构（约 10 倍于米粒）。
    float superG = fbm(p * 9.0 + vec3(0.0, 0.0, t * 0.06));
    // 磁网络：米粒边界上的亮环（真实太阳临边附近的 faculae）。
    float network = smoothstep(0.55, 0.95, ridged(p * SC + vec3(0.0, 0.0, t * 0.35)));

    float heat = gran * 0.62 + superG * 0.38;
    heat = heat * 0.82 + network * 0.22;
    // 对比度重映射：米粒组织的自然动态范围只有 ±20%，在 ACES 之后几乎看不出结构。
    // 以 0.50 为中心拉开 1.9 倍，让颗粒/暗沟真的读得出来。
    heat = clamp((heat - 0.50) * 1.55 + 0.50, 0.0, 1.4);

    // 太阳黑子：大尺度的暗区，本影更黑、半影有丝状结构。
    float spotField = fbm(p * 2.6 + vec3(5.0, 9.0, t * 0.02));
    float penumbra = smoothstep(0.60, 0.74, spotField);
    float umbra = smoothstep(0.70, 0.80, spotField);
    float spot = penumbra * 0.55 + umbra * 0.65;
    // 黑子只在低纬带出现（真实黑子集中在 ±5°..±30°），别让极区也长斑。
    float latBand = exp(-pow(p.y / 0.55, 2.0));
    spot *= mix(0.25, 1.0, latBand);
    heat *= (1.0 - spot * 0.72);

    // 色带：冷 → 暖 → 白热。**整体往深橙红推**（对照 SDO 参考图：盘面是深橙红，
    // 只有活动区才是亮黄）。原来 cool/mid 偏黄，整颗太阳读起来苍白。
    vec3 cool = vec3(0.78, 0.20, 0.05);
    vec3 mid  = vec3(1.05, 0.48, 0.12);
    vec3 hot  = vec3(1.42, 0.82, 0.36);
    vec3 col = mix(cool, mid, smoothstep(0.28, 0.66, heat));
    col = mix(col, hot, smoothstep(0.62, 1.02, heat));

    // 临边昏暗：太阳最重要的「是颗球」的证据。亮度按 I(μ) 压，颜色同时偏红。
    // 系数从 0.62/0.18 收到 0.52/0.14：原来的律在最外圈把亮度压到 0.20（盘心 240 →
    // 日缘只剩 187），而色球又填不满那 25% 的坑，于是日缘留下一条**暗缝**（用户报的
    // 「黑边」）。0.34 的下限保住"是颗球"的临边昏暗，同时不至于在轮廓线上割出一道黑线。
    float limb = 1.0 - 0.52 * (1.0 - mu) - 0.14 * (1.0 - mu) * (1.0 - mu);
    col *= clamp(limb, 0.10, 1.0);
    col = mix(col, col * vec3(1.25, 0.55, 0.20), pow(1.0 - mu, 3.0) * 0.85);
    // 黑子区域再压一点蓝，读起来是「暗红的斑」而不是「灰色的洞」。
    col = mix(col, col * vec3(1.15, 0.6, 0.35), spot * 0.5);

    gl_FragColor = vec4(col * uIntensity, 1.0);
  }
#endif // PX_SUN_SUN_PHOTO_FRAG
