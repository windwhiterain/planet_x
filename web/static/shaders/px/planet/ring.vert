#ifndef PX_PLANET_RING_VERT
#define PX_PLANET_RING_VERT
  varying vec3 vWorldPos;
  varying vec3 vLocal;
  void main(){
    vLocal = position;
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
#endif // PX_PLANET_RING_VERT
