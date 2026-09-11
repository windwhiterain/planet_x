#ifndef PX_MISC_PLUME_VERT
#define PX_MISC_PLUME_VERT
  varying vec2 vUvP;
  varying vec3 vPos;
  void main(){
    vUvP = uv;
    vPos = position;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
#endif // PX_MISC_PLUME_VERT
