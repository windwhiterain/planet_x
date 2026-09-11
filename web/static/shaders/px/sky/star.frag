#ifndef PX_SKY_STAR_FRAG
#define PX_SKY_STAR_FRAG
  precision highp float;
  varying vec3 vCol;
  varying float vSize;
  void main(){
    vec2 p = gl_PointCoord * 2.0 - 1.0;
    float r2 = dot(p, p);
    if (r2 > 1.0) discard;
    float core = exp(-r2 * 4.2);
    // 只有大点（=亮星）才画十字衍射，与「人眼/望远镜看亮星」一致。
    float spike = 0.0;
    if (vSize > 2.0) {
      vec2 ap = abs(p);
      spike = (exp(-ap.x * 16.0) * exp(-ap.y * 1.8) + exp(-ap.y * 16.0) * exp(-ap.x * 1.8));
      spike *= smoothstep(2.0, 3.4, vSize) * 0.32;
    }
    gl_FragColor = vec4(vCol * (core + spike), 1.0);
  }
#endif // PX_SKY_STAR_FRAG
