#ifndef PX_POST_GRADE_FRAG
#define PX_POST_GRADE_FRAG
#include <px/post/common.glsl>

  precision highp float;

  uniform sampler2D tDiffuse;
  uniform vec2  uTexel;
  uniform float uCA;        // 色差强度（像素）
  uniform float uVignette;
  uniform float uGrain;
  uniform float uTime;
  varying vec2 vUv;

  void main(){
    vec2 uv = vUv;
    vec2 d = uv - 0.5;
    float r2 = dot(d, d);

    // --- 色差：横向色差随离画面中心的距离平方增长（真实镜头的横向色差就是这个形状）---
    vec2 off = d * r2 * uCA * 4.0 * uTexel;
    vec3 col;
    col.r = texture2D(tDiffuse, uv + off).r;
    col.g = texture2D(tDiffuse, uv).g;
    col.b = texture2D(tDiffuse, uv - off).b;

    // --- 暗角：cos^4 律的廉价近似（真实镜头的自然渐晕，不是「画个黑圈」）---------
    float vig = 1.0 - uVignette * pow(clamp(r2 * 2.0, 0.0, 1.0), 1.45);
    col *= vig;

    // --- 胶片颗粒：暗部多、亮部少（真实银盐颗粒的信噪比特征）-------------------
    if (uGrain > 0.0) {
      float n = grainNoise(uv * vec2(1920.0, 1080.0), fract(uTime) * 91.7) - 0.5;
      float l = luma(col);
      col += n * uGrain * (1.0 - l * 0.75);
    }

    gl_FragColor = vec4(max(col, 0.0), 1.0);
  }
#endif // PX_POST_GRADE_FRAG
