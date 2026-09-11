#ifndef PX_SKY_SKY_FRAG
#define PX_SKY_SKY_FRAG
#include <px/noise/fbm.glsl>
#include <px/noise/ridged.glsl>
#include <px/noise/warp.glsl>

  precision highp float;

  varying vec3 vDir;

  // 银道极（随便取一个不与坐标轴平行的方向，让银河斜着穿过天空）。
  const vec3 GAL_N = vec3(0.3180, 0.8000, -0.5080);

  void main(){
    vec3 d = normalize(vDir);
    vec3 col = vec3(0.0);

    vec3 gu = normalize(cross(GAL_N, vec3(0.0, 0.0, 1.0)));
    vec3 gv = cross(GAL_N, gu);
    float lat = abs(dot(d, GAL_N));
    // 银道面：|sin(b)| 的高斯型包络。
    float band = exp(-pow(lat / 0.19, 1.7));
    // 银心方向（银经 0 附近最亮最厚）。
    float coreness = exp(-pow(length(vec2(dot(d, gu), dot(d, gv) - 1.0)) / 0.85, 2.0));
    // 云状结构：域扰动 fbm，再用 ridged 抠出暗尘带。
    vec3 wp = warp(d * 5.5, 0.65, 0.0, 0.45);
    float cloud = fbm(wp * 1.7);
    float dust = ridged(d * 7.5 + 3.1);
    // 暗尘带要**黑得下去**（对照 scratch/ref/milkyway-core.jpg：银河最抓人的是亮星云
    // 与黑尘带的强对比，而不是一层均匀的灰雾）。
    float bright = band * (0.10 + 0.90 * cloud) * (1.0 - 0.88 * smoothstep(0.28, 0.86, dust));
    // 银河的色：银心偏暖黄（老年星族），外围偏冷蓝（年轻星族 + 尘埃散射）。
    vec3 galCol = mix(vec3(0.42, 0.52, 0.86), vec3(1.05, 0.92, 0.70), coreness * 0.85 + 0.12);
    col += galCol * bright * 0.165;
    // 弥漫的银道面辉光（不带结构的底光）。
    col += vec3(0.28, 0.33, 0.55) * band * 0.020;

    // --- 星云：几团定点 + fbm 调制；发射线配色（Hα 红 / OIII 青 / 反射星云蓝）------
    // 中心是**固定常量**，不是随机——星空每次加载必须一模一样，截图才可比。
    const vec3 NEB[3] = vec3[3](
      vec3(0.86, 0.31, 0.24), vec3(-0.62, 0.44, 0.65), vec3(0.15, -0.78, 0.60)
    );
    const vec3 NEB_COL[3] = vec3[3](
      vec3(1.00, 0.24, 0.30),   // Hα
      vec3(0.22, 0.90, 0.78),   // OIII
      vec3(0.35, 0.48, 1.00)    // 反射星云
    );
    for (int i = 0; i < 3; i++) {
      vec3 nc = normalize(NEB[i]);
      float dd = length(d - nc);
      float fall = exp(-pow(dd / 0.42, 2.0));
      if (fall > 0.002) {
        vec3 nq = warp(d * 9.0 + float(i) * 21.0, 1.07, 0.0, 0.28);
        float fil = ridged(nq * 1.4);
        float wisp = smoothstep(0.42, 0.95, fil) * (0.35 + 0.65 * fbm(nq * 3.1));
        col += NEB_COL[i] * fall * wisp * 0.075;
      }
    }

    // 极暗的「未分辨恒星」底噪：真实长曝光下天空从不全黑。
    col += vec3(0.014, 0.017, 0.028);

    gl_FragColor = vec4(col, 1.0);
  }
#endif // PX_SKY_SKY_FRAG
