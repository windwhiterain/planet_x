#ifndef PX_MISC_PLUME_FRAG
#define PX_MISC_PLUME_FRAG
  precision highp float;
  uniform float uTime;
  uniform vec3  uCore;
  uniform vec3  uEdge;
  uniform float uIntensity;
  varying vec2 vUvP;
  varying vec3 vPos;
  float h1(vec3 p){ p = fract(p * 0.1031); p += dot(p, p.zyx + 31.32); return fract((p.x + p.y) * p.z); }
  float n3(vec3 p){
    vec3 i = floor(p), f = fract(p); f = f*f*(3.0-2.0*f);
    return mix(mix(mix(h1(i), h1(i+vec3(1,0,0)), f.x), mix(h1(i+vec3(0,1,0)), h1(i+vec3(1,1,0)), f.x), f.y),
               mix(mix(h1(i+vec3(0,0,1)), h1(i+vec3(1,0,1)), f.x), mix(h1(i+vec3(0,1,1)), h1(i+vec3(1,1,1)), f.x), f.y), f.z);
  }
  void main(){
    // 锥面：uv.y 从尾喷口(0)到羽流末端(1)；uv.x 是绕轴的角度。
    float t = clamp(vUvP.y, 0.0, 1.0);
    // 沿轴向的衰减 + 湍流（羽流越远越散、越暗）。
    float turb = n3(vec3(vUvP.x * 9.0, t * 6.0 - uTime * 2.6, uTime * 0.7));
    float fall = pow(1.0 - t, 1.5) * (0.62 + 0.55 * turb);
    // 激波菱形（真实火箭羽流里的马赫环）。
    float mach = 0.75 + 0.45 * sin(t * 34.0 - uTime * 12.0) * exp(-t * 3.4);
    float flick = 0.88 + 0.12 * n3(vec3(uTime * 22.0, 0.0, 0.0));
    vec3 col = mix(uCore, uEdge, pow(t, 0.65));
    gl_FragColor = vec4(col * fall * mach * flick * uIntensity, fall);
  }
#endif // PX_MISC_PLUME_FRAG
