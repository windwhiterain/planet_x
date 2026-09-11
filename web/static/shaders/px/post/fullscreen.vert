#ifndef PX_POST_FULLSCREEN_VERT
#define PX_POST_FULLSCREEN_VERT
  varying vec2 vUv;
  void main(){
    vUv = uv;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
#endif // PX_POST_FULLSCREEN_VERT
