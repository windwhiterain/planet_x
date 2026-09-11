#ifndef PX_SKY_STAR_VERT
#define PX_SKY_STAR_VERT
  attribute float aSize;
  attribute vec3  aColor;
  uniform float uPixelRatio;
  uniform float uScale;
  varying vec3 vCol;
  varying float vSize;
  void main(){
    vCol = aColor;
    vSize = aSize;
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_Position = projectionMatrix * mv;
    // 恒定屏幕像素：不做距离衰减。uPixelRatio 抵消 DPR 缩放，星点在 HiDPI 下不变粗。
    gl_PointSize = max(1.0, aSize * uPixelRatio * uScale);
  }
#endif // PX_SKY_STAR_VERT
