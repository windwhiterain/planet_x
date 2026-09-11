#ifndef PX_POST_SUNFLARE_FRAG
#define PX_POST_SUNFLARE_FRAG
#include <px/post/common.glsl>

  precision highp float;

  uniform sampler2D tDiffuse;
  uniform vec2  uSun;         // 太阳的屏幕 uv（0..1，可能落在屏幕外）
  uniform float uVis;         // 太阳可见度（被天体挡住 → 0）
  uniform vec2  uTexel;       // 1 / 分辨率
  uniform vec2  uAspect;      // (aspect, 1)
  uniform float uGodStrength, uDecay, uDensity, uWeight, uThreshold;
  uniform float uStreak;
  uniform float uGhost;
  // 采样数 / 环数是 **uniform** 而不是 #define：常量上界会让编译器把循环完全展开
  // （理由与实测数据见 util.js 里 NOISE_GLSL 那段）。这三个都是全屏 pass 的主循环，
  // GODRAY 24 次、STREAK 25 次展开是实打实的编译量。
  uniform int uGodraySamples;
  uniform int uStreakTaps;
  uniform int uGhostCount;
  varying vec2 vUv;

  // 光圈核：一条**柔和的光斑**。
  // 旧版是「同心环 + 极角量化的六边形硬边」：cos(r*22.0) 画出一圈圈离散的环、
  // 极角 floor 出六条硬边，在虚空里读起来是一个**靶子**而不是镜头反射
  // （实测：太阳跑到画面外时，画面另一侧会浮出一整片粉色靶环 —— 用户报的「神秘的点」）。
  // 真实的鬼影是镀膜/光圈叶片的**离焦**像：一个软核加一圈很淡的外晕，没有离散环。
  float aperture(vec2 p){
    float r2 = dot(p, p);
    if (r2 > 1.0) return 0.0;
    float core = exp(-r2 * 4.5);            // 软核
    float halo = 0.16 * exp(-r2 * 1.5);     // 极淡的外晕
    // 边缘窗口保证收到 0，不留硬边（旧版 smoothstep(1.0,0.82) 太窄，等于没淡出）。
    return (core + halo) * smoothstep(1.0, 0.45, sqrt(r2));
  }

  // 只让「真的比白更亮」的东西参与散射（太阳本体 / 它的紧致辉光 / 引擎羽流）。
  // 关键：**不能**用 s * luma(s) —— 那是把亮度平方，HDR 下会瞬间炸到几十，整幅画面洗白。
  float sourceMask(vec3 s){
    return smoothstep(uThreshold, uThreshold * 3.0, luma(s));
  }

  void main(){
    vec4 base = texture2D(tDiffuse, vUv);
    float vis = uVis;
    if (vis <= 0.001) { gl_FragColor = base; return; }

    vec3 flare = vec3(0.0);

    // --- ① 体积光（沿「本像素 → 太阳」的径向散射）---------------------------------
    // delta 是**追到太阳为止**的整段路（乘以 density），所以每个像素的采样点分布相同；
    // 屏幕上的径向梯度来自下面那条 radial 包络，而不是来自采样几何（采样几何本身给不出梯度）。
    // 每条射线 24 次纹理采样，是全屏 pass 里最贵的一项；关掉时（低质量档）必须真的
    // 跳过循环，而不是把结果乘 0 —— 否则「关掉体积光」一点都不会变快。
    if (uGodStrength > 0.0) {
      vec2 delta = (uSun - vUv) * (uDensity / float(uGodraySamples)) * uAspect;
      vec2 uv = vUv;
      float illum = uWeight;
      vec3 acc = vec3(0.0);
      for (int i = 0; i < uGodraySamples; i++) {
        uv += delta;
        vec3 s = texture2D(tDiffuse, uv).rgb;
        acc += s * sourceMask(s) * illum;
        illum *= uDecay;
      }
      // 归一：sum(illum) ≈ uWeight/(1-uDecay)，再乘一个整体的观感系数。
      acc *= uGodStrength * (1.0 - uDecay) * 0.42 / max(uWeight, 1e-4);
      // 包络：集中在太阳附近（有遮挡物时的那道「光柱」就是这个包络被物体切开的样子）。
      float d2s = length((vUv - uSun) * uAspect);
      flare += acc * exp(-d2s * 3.1);
    }

    // --- ② 各向异性拉丝（变形宽银幕镜头那道水平蓝线） -------------------------
    // 采样是**锚在太阳的屏幕上**做的，所以整行的 st 是同一个值——必须再乘一条横向包络，
    // 否则会画出一条贯穿全屏的亮带（踩过这个坑）。
    if (uStreak > 0.0) {
      vec3 st = vec3(0.0);
      float wsum = 0.0;
      for (int i = -uStreakTaps; i <= uStreakTaps; i++) {
        float fi = float(i);
        // 指数核（真正的横向拖尾来自下面那条 hfall，这里只负责取平均）。
        float w = exp(-abs(fi) * 0.16);
        vec2 suv = vec2(uSun.x + fi * uTexel.x * 9.0, uSun.y);
        vec3 s = texture2D(tDiffuse, suv).rgb;
        st += s * sourceMask(s) * w;
        wsum += w;
      }
      st /= max(wsum, 1e-4);
      // 横向：长长的指数尾（这就是「拉丝」）；纵向：一条很窄的包络（不然整行都会被点亮）。
      float hfall = exp(-abs(vUv.x - uSun.x) * uAspect.x * 6.5);
      float vfall = exp(-pow((vUv.y - uSun.y) * uAspect.y * 46.0, 2.0));
      flare += st * uStreak * 0.55 * hfall * vfall * vec3(0.72, 0.84, 1.25);
    }

    // --- ③ 镜头鬼影：沿「太阳 → 屏幕中心」连线的一组彩色光圈 ------------------
    if (uGhost > 0.0) {
      vec2 c = vec2(0.5);
      vec2 axis = c - uSun;
      for (int i = 0; i < uGhostCount; i++) {
        // 每个鬼影在连线上有固定的位置/大小/色相（同一颗镜头，每次看到的都一样）。
        float t = -0.45 + float(i) * 0.42;
        float sc = 0.05 + 0.075 * fract(float(i) * 0.617 + 0.23);
        vec2 gp = (uSun + axis * t - vUv) * uAspect / sc;
        float ap = aperture(gp);
        vec3 tint = 0.5 + 0.5 * cos(vec3(0.0, 2.094, 4.188) + float(i) * 1.7);
        // 鬼影的亮度跟着太阳在画面里的「入射强度」走（越靠边越暗，符合真实镀膜反射）。
        float off = smoothstep(0.0, 0.85, length(uSun - c));
        // **屏幕边缘遮罩**：鬼影是屏幕空间的，画面边框是硬的 —— 不做遮罩它就会被
        // 边框**硬切**，在半空里露出半个靶环。按鬼影自己的屏幕位置（uv 0..1）淡出。
        vec2 ep = uSun + axis * t;
        float edge = smoothstep(0.0, 0.16, ep.x) * smoothstep(1.0, 0.84, ep.x)
                   * smoothstep(0.0, 0.16, ep.y) * smoothstep(1.0, 0.84, ep.y);
        flare += tint * ap * uGhost * (0.35 + 0.65 * off) * 0.5 * edge;
      }
    }

    gl_FragColor = vec4(base.rgb + flare * vis, base.a);
  }
#endif // PX_POST_SUNFLARE_FRAG
