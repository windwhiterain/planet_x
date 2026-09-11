#ifndef PX_SUN_CORONA_VERT
#define PX_SUN_CORONA_VERT
  varying vec3 vWorld;
  void main(){
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorld = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
#endif // PX_SUN_CORONA_VERT
