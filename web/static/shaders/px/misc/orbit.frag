#ifndef PX_MISC_ORBIT_FRAG
#define PX_MISC_ORBIT_FRAG
  precision highp float;
  uniform vec3  uColor;
  uniform vec3  uHead;
  uniform float uHeadSharp;
  uniform float uOpacity;
  uniform float uHeadGain;
  varying vec3 vWorldPos;
  void main(){
    float d = distance(vWorldPos, uHead);
    float head = exp(-d * d * uHeadSharp);
    float a = uOpacity * (0.62 + 0.38 * head) + head * uHeadGain;
    gl_FragColor = vec4(uColor * (0.55 + 2.1 * head), a);
  }
#endif // PX_MISC_ORBIT_FRAG
