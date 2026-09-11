#ifndef PX_NOISE_HASH13_GLSL
#define PX_NOISE_HASH13_GLSL
  float hash13(vec3 p){
    p = fract(p * 0.1031);
    p += dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
  }
#endif // PX_NOISE_HASH13_GLSL
