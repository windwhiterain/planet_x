#ifndef PX_PLANET_CLOUD_FRAG
#define PX_PLANET_CLOUD_FRAG
#include <px/noise/fbm.glsl>
#include <px/noise/warp.glsl>

  precision highp float;

  uniform float uTime;
  uniform float uSpin;
  uniform float uAmount;
  uniform vec3  uTint;
  uniform float uOpacity;
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;
  void main(){
    vec3 d = normalize(vObjPos);
    // 云的自转与地表**不同步**（真实大气有风），所以这里用自己的一套旋转。
    vec3 ds = d;
    float ang = -uSpin;
    float c = cos(ang), s = sin(ang);
    ds = vec3(ds.x * c - ds.z * s, ds.y, ds.x * s + ds.z * c);

    // 云团：纬度带状的环流 + 域扰动，做出「涡旋/带状」而不是均匀的花花。
    float latB = 0.55 + 0.45 * sin(ds.y * 7.0 + fbm(ds * 3.0) * 2.2);
    vec3 w = warp(ds * 5.0, 0.60, uTime * 0.02, 0.50);
    float cov = fbm(w * 3.0 + vec3(0.0, 0.0, uTime * 0.01));
    // 叠一层更细的云絮：只有大尺度的云看起来像一团团棉花糖。
    cov = cov * 0.72 + fbm(ds * 11.0 + vec3(4.0, 0.0, uTime * 0.03)) * 0.34;
    float cover = smoothstep(0.52, 0.74, cov * (0.55 + 0.65 * latB));
    cover = clamp(cover * uAmount, 0.0, 1.0);
    if (cover < 0.006) discard;

    vec3 n = normalize(vNormal);
    vec3 lightDir = normalize(-vWorldPos);
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float ndl = dot(n, lightDir);
    float lit = smoothstep(-0.10, 0.22, ndl);
    // 云顶是白的，但边缘/厚度大处偏灰（自遮挡）。
    float thick = smoothstep(0.15, 0.85, cover);
    vec3 col = mix(uTint * 0.72, uTint, thick) * (0.10 + 0.90 * lit);
    // 散射：云在逆光时边缘透光。
    float fwd = pow(max(dot(-lightDir, viewDir), 0.0), 3.0);
    col += uTint * fwd * 0.25 * lit;
    // 临边处视线穿过更厚的一层云 → 稍微不透明一点。
    float rim = pow(1.0 - max(dot(n, viewDir), 0.0), 2.0);
    float alpha = clamp(cover * (0.82 + 0.18 * rim) * uOpacity, 0.0, 1.0);
    gl_FragColor = vec4(col, alpha);
  }
#endif // PX_PLANET_CLOUD_FRAG
