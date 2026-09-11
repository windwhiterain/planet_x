#ifndef PX_SUN_CHROMO_FRAG
#define PX_SUN_CHROMO_FRAG
#include <px/noise/ridged.glsl>

  precision highp float;

  uniform float uTime;
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;
  void main(){
    vec3 p = normalize(vObjPos);
    vec3 n = normalize(vNormal);
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float mu = abs(dot(n, viewDir));
    // 针状体（spicules）：沿径向拉长的细丝，随时间抖动。
    float spic = ridged(p * vec3(38.0, 38.0, 9.0) + vec3(0.0, 0.0, uTime * 0.9));
    float rim = pow(1.0 - mu, 3.4);
    vec3 col = vec3(1.35, 0.30, 0.14) * rim * (0.35 + 0.9 * spic);
    col += vec3(1.6, 0.55, 0.22) * pow(1.0 - mu, 7.0) * 0.7;
    gl_FragColor = vec4(col, 1.0);
  }
#endif // PX_SUN_CHROMO_FRAG
