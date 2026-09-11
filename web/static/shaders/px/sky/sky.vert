#ifndef PX_SKY_SKY_VERT
#define PX_SKY_SKY_VERT
  varying vec3 vDir;
  void main(){
    vDir = position;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
#endif // PX_SKY_SKY_VERT
