#ifndef PX_SUN_PROM_FRAG
#define PX_SUN_PROM_FRAG
#include <px/noise/vnoise.glsl>

  precision highp float;

  uniform float uTime;
  uniform float uIntensity;   // HDR 强度（>1 交给 bloom）
  uniform vec3  uColorHot;    // 根部/亮部
  uniform vec3  uColorCool;   // 尖端/暗部（304Å 的日珥是深红橙到亮橙）
  varying vec2 vUv;
  varying vec3 vWorld;
  varying float vSeed;
  varying float vLift;

  void main(){
    float x = vUv.x - 0.5;
    float t = vLift;

    // ① 横向剖面：中间实、两边虚 ⇒ 每一片读起来是**一根须**，不是一块板。
    //    指数越大越细（0.14 时大约只有中间 30% 是实心）。
    float core = 1.0 - smoothstep(0.10, 0.42, abs(x));

    // ② 沿长度的**断裂**：真实日珥不是一条光滑的带，而是几段亮节 + 断口。
    //    用一个沿长度的高频噪声 + 每片自己的相位（vSeed）⇒ 每根须的亮节位置都不同。
    float n = vnoise(vec3(t * 5.5, vSeed * 37.0, uTime * 0.22));
    float n2 = vnoise(vec3(t * 14.0, vSeed * 11.0 + 5.0, uTime * 0.31));
    float broken = smoothstep(0.28, 0.62, n * 0.72 + n2 * 0.28 + (1.0 - t) * 0.34);

    // ③ 尖端散开：越靠尖端越虚（针尖是散的），根部几乎实心
    float tip = 1.0 - smoothstep(0.55, 1.0, t);

    float a = core * broken * tip;

    // ④ **只在掠射时可见**：正对着看的针只是一个亮点，不该是一条长条。
    // 没这一条时，整个可见半球都被针盖住（像一只毛球），
    // 而 304Å 的盘面应该是**斑驳的表面**，针只在日缘一圈。
    vec3 vd = normalize(cameraPosition - vWorld);
    vec3 rd = normalize(vWorld);                 // 太阳在世界原点
    float graze = 1.0 - abs(dot(vd, rd));        // 日缘处 ≈ 1，盘心处 ≈ 0
    // 又因为盘面上的针在 304Å 里几乎看不见（盘面自己太亮），
    // 所以越靠盘心越淡：留一个线性因子，而不是硬切。
    a *= smoothstep(0.25, 0.75, graze) * (0.20 + 0.80 * graze);

    if (a < 0.02) discard;

    // 颜色：根部亮橙、尖端深红（304Å 的日珥就是这个走向），再乘一点每片的色偏
    vec3 col = mix(uColorCool, uColorHot, clamp((1.0 - t) * 0.85 + n * 0.35, 0.0, 1.0));
    col *= 0.85 + 0.30 * n2;

    // 加色混合：日珥是**发光体**（光学薄），叠加是对的；深度的遮挡由 depthTest 保证。
    gl_FragColor = vec4(col * uIntensity * a, a);
  }
#endif // PX_SUN_PROM_FRAG
