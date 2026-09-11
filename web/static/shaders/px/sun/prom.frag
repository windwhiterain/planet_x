#ifndef PX_SUN_PROM_FRAG
#define PX_SUN_PROM_FRAG
#include <px/noise/perlin.glsl>
#include <px/noise/vnoise.glsl>

  precision highp float;

  uniform float uTime;
  uniform float uIntensity;   // HDR 强度（>1 交给 bloom）
  uniform vec3  uColorHot;    // 根部/亮部
  uniform vec3  uColorCool;   // 尖端/暗部（304Å 的日珥是深红橙到亮橙）
  uniform float uFilFreq;     // 丝的**世界尺度**频率（1/世界单位，沿带宽方向）
  uniform float uFilAlong;    // 顺带长方向的频率比（越小 ⇒ 丝越长）
  varying vec2 vUv;
  varying vec3 vWorld;
  varying vec2 vSize;         // 本带子的世界尺寸（x=宽 y=高）
  varying float vSeed;
  varying float vLift;
  varying float vArc;
  varying float vMask;
  varying float vEdge;
  varying float vLife;      // 生命期包络（顶点算好传过来）
  varying vec4 vStyle;      // x=丝距倍率 y=丝长倍率 z=宽度剖面 w=色温偏移
  varying vec2 vShape;      // x=截面锐度（0=平铺面纱 1=中间一道脊） y=丝尖参差度

  // 一条带子内部要画出**很多细小的日珥**（用户原话）。
  //
  // ⚠ **这里的噪声是本地空间的，不是世界空间的** —— 这件事第一版我做反了，记下来：
  //   · 世界空间共享的那个场（现在烘在每实例属性里，见 `map3d/sunfield.js`）负责的是
  //     **带子的扭曲/朝向**，让相邻带子成片地倒向同一侧、扭向同一侧；
  //   · **带子内部的纹理属于这条带子自己**（沿带宽/带长的 uv + 每片的 seed）。
  //   把内部纹理也钉在世界坐标上的后果：纹理尺度被"世界单位"锁死，一条窄带子里只剩两三根丝，
  //   而且纹理和带子的形状毫无关系（带子扭过去，纹理不动）。
  //   现在的做法：**参数化在本地**（`vUv`），但用 `vSize` 把 uv 换算成**世界尺度**再乘频率
  //   ⇒ 丝的粗细对任何尺寸的带子都一致，而纹理跟着带子走。
  void main(){
    float x = vUv.x - 0.5;      // 横向（本地）
    float t = vLift;            // 沿长度（本地，0=扎进日面的根）
    float arcMix = smoothstep(0.25, 0.85, vArc);

    float u = x * vSize.x;      // 世界单位的横向坐标（本带子的局部世界尺度）
    float v = t * vSize.y;      // 世界单位的纵向坐标

    // ① 并排的细丝：沿带宽**高频**、沿带长**低频** ⇒ 一根根又细又长的丝。
    //    ⚠ 频率/丝长是**每片自己**的（`vStyle.xy`，来自形态种群）—— 这一条是"灵性"的关键：
    //    以前它们是全局 uniform，所以每条带子的表面长得一模一样（用户："像种草一样"）。
    float kFil = uFilFreq * vStyle.x;
    float kAlong = uFilAlong * vStyle.y;
    vec3 lp = vec3(u * kFil, v * kFil * kAlong, vSeed * 9.0);
    float c1 = ridgedP(lp);
    float c2 = ridgedP(lp * 2.3 + vec3(11.0, 3.0, 7.0));   // 丝上的更细结构
    float c3 = ridgedP(lp * 0.42 + vec3(3.0, 29.0, 17.0));  // 丝束的**粗分簇**（让丝成束、不成栅栏）
    // ⚠ 阈值定得低（0.22/0.58）：`ridgedP` 均值只有 ~0.25、脊峰 ~0.8，
    //   用 0.52/0.86 那种高阈值会把纹理筛没，画面上只剩几缕飘着的火星。
    float thread = smoothstep(0.22, 0.58, c1) * (0.55 + 0.60 * smoothstep(0.24, 0.62, c2));
    // 粗分簇：一簇一簇地亮 ⇒ 读成"一束束丝"而不是等距栅栏（"灵性"的另一半）
    thread *= 0.55 + 0.75 * smoothstep(0.30, 0.70, c3);

    // ② **每根丝有自己的长度**：横向上一个与丝的间距同量级的低频场，给每根丝一个
    //    "到多高就散"的高度 ⇒ 一片带子读起来是**几十根长短不一的日珥**，
    //    而不是一块均匀的条纹布。这是"细小日珥"的第二个必要条件（第一个是 ① 的并排细丝）。
    //  ⚠ 上限必须 < 1：`tEnd` 一旦超过 1，那根丝就一直顶到几何的**上边缘** ⇒
    //    整片带子又出现一条**笔直的截断**（这是插片最容易被认出来的破绽）。
    //    压到 0.92 之后，每一根丝都在带子的几何边界**之前**就淡掉了。
    float tEnd = 0.40 + 0.52 * vnoise(vec3(u * kFil * 0.85, vSeed * 17.0, uTime * 0.030));
    // 淡出区间给得宽（≈0.34/0.20）：窄了每一根丝的末端都是一记硬收，
    // 几十根并排就是一片"毛刺"（用户：毛躁）。宽区间读成"散开"。
    // 参差度 `vShape.y` 每片不同：喷流整齐、灌木丛散乱。
    float ls = vShape.y;
    float threadLen = 1.0 - smoothstep(tEnd - ls, tEnd + ls * 0.59, t);

    // ③ 根部略实（那是扎进色球的脚），往上迅速被 ① ② 切成丝。
    // ⚠ 不能取实心 1.0：会露出**四边形的几何边**（插片最大的破绽）。
    // ⚠ 也不能让丝之间归零：那样带子读成**一缕缕烧完的灰**；《群星》里的日珥是
    //   **发光的等离子体**。留 0.30 的底光 ⇒ 带子整体是一团发光体，丝是它上面更亮的筋。
    float fil = mix(0.82, 0.30 + 0.95 * thread, clamp((t - 0.06) * 2.0, 0.0, 1.0));

    // ④ 横向剖面：带子只是个"宽松的容器"，边缘必须是软的。
    //    截面**每片不同**（`vShape.x`）：0 = 平铺的面纱（宽而匀），1 = 中间一道脊（细喷流）。
    //    只有一种剖面时，正面看过去每条都长得一样 —— 那也是"草地感"的来源之一。
    float flatP = 1.0 - smoothstep(mix(0.13, 0.05, arcMix), mix(0.50, 0.42, arcMix), abs(x));
    float ridgeP = 1.0 - smoothstep(0.02, mix(0.22, 0.30, arcMix), abs(x));
    float lateral = mix(flatP, ridgeP, vShape.x);

    // ⑤ 沿长度的亮节/断口（本地空间 + 每片 seed）：一条日珥上有明暗起伏，不是均匀发光。
    float knots = smoothstep(0.28, 0.72, vnoise(vec3(v * 1.9, vSeed * 23.0, uTime * 0.050)));
    knots = clamp(knots * (0.80 + 0.40 * vnoise(vec3(v * 5.0, vSeed * 7.0, uTime * 0.07))), 0.0, 1.0);

    // ⑥ 尖端：丝是散的；拱的两头是**落回日面的脚**，该是实的。
    float tip = mix(1.0 - smoothstep(0.55, 1.00, t),
                    1.0 - 0.45 * smoothstep(0.88, 1.00, t),
                    arcMix);

    // 生命期进 alpha 与亮度：正在喷发的带子更亮更热，落回去的只剩一点余烬。
    float life = 0.32 + 0.68 * vLife;
    float a = lateral * fil * threadLen * (0.55 + 0.45 * knots) * tip * vMask * vEdge * life;

    // ⑦ **只在掠射时可见**：正对着看的带子只是一块亮 patch，不该盖住整个盘面。
    //    没这一条时，整个可见半球都被盖住（像一只毛球），
    //    而 304Å 的盘面应该是**斑驳的表面**，色球层只在日缘一圈显形。
    //    拱的窗口要放宽（它本来就拱在日面之上，弧身该看得见）。
    vec3 vd = normalize(cameraPosition - vWorld);
    vec3 rd = normalize(vWorld);                 // 太阳在世界原点
    float graze = 1.0 - abs(dot(vd, rd));        // 日缘处 ≈ 1，盘心处 ≈ 0
    a *= mix(smoothstep(0.25, 0.75, graze) * (0.20 + 0.80 * graze),
             smoothstep(0.08, 0.55, graze) * (0.40 + 0.60 * graze),
             arcMix);

    if (a < 0.02) discard;

    // 颜色：根部亮橙、尖端深红（304Å 的日珥就是这个走向）；
    // 拱的"亮"在**两端**（两个脚扎进色球）、中段偏冷 —— 和须正好反过来。
    float heatT = mix(1.0 - t, abs(2.0 * t - 1.0), arcMix) + vStyle.w;
    vec3 col = mix(uColorCool, uColorHot, clamp(heatT * 0.80 + knots * 0.45, 0.0, 1.0));
    col *= (0.80 + 0.45 * knots) * (0.72 + 0.38 * vLife);

    // 加色混合：日珥是**发光体**（光学薄），叠加是对的；深度的遮挡由 depthTest 保证。
    gl_FragColor = vec4(col * uIntensity * a, a);
  }
#endif // PX_SUN_PROM_FRAG
