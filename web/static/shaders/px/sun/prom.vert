#ifndef PX_SUN_PROM_VERT
#define PX_SUN_PROM_VERT
#include <px/noise/perlin.glsl>
  // 日珥插片（**世界空间的大片曲面条带**）。
  //
  // 演变（见 `.agents/notes/sun-prominence.md`）：等半径球壳 ✗ → 并入日冕体积积分 ✗
  // （掠射视线把几十根针平均成雾）→ 每像素解析求交 ✗（2.5D、没有真遮挡）→
  // 几千根细小插片 ✗（实例数就是密度上限，每根又是光板一块）→
  // **少数大片曲面条带 + 片元里的本地空间纹理**（现在）。
  //
  // 分工：
  //   · **每实例属性**（CPU 烘）决定"在哪里、多大、朝哪边倒、往哪边扭、什么时候喷"
  //     —— 流场见 `map3d/sunfield.js`（**curl noise，无散度**）。
  //   · **顶点**把横向偏移换算成角度（**曲面**）、做上缘/侧缘的轻微起伏、
  //     并把**喷发生命期**加到高度上。
  //   · **片元**在**本地空间**把带子切成十几~几十根细长日珥（见 prom.frag）。
  //
  // ⚠ 流场为什么在 CPU 侧烘（这一点是本文件从上一版改过来的原因）：带子的朝向/扭曲
  //   **本来就是每片一个常数**（同一片的所有顶点用同一个根部方向），而上一版让每个
  //   **顶点**都去算 2 次 `promFlow` + 1 次 `promMask`（约 24 次 `pnoise`）——
  //   250 顶点 × 5000 片 = 每帧 3000 万次噪声，实测那正是 LOD 差价（3.5 ms）的主因。
  //   烘成属性之后**顶点着色器里一次噪声都不用算**（只剩上缘起伏那两次，它是真的逐顶点）。
  attribute vec3 aDir;     // 根部方向（球面上的单位向量）
  attribute vec3 aSide;    // 根部切平面基 1（宽度方向）
  attribute vec3 aBend;    // 根部切平面基 2（侧弯 / 扭转方向的另一个分量）
  attribute vec4 aParam;   // x=高度 y=宽度(弧长) z=弯曲方向抖动 w=种子
  attribute vec3 aKind;    // x=拱度(0..1) y=顺场倾倒抖动 z=宽度倍率
  attribute vec4 aFlow;    // x,y=切平面里的"朝哪边倒" z=场强(0..1) w=「哪里有日珥」
  attribute vec4 aFlow2;   // x,y=切平面里的"往哪边扫" z=喷发相位 w=周期倍率
  attribute vec4 aTwist;   // x=总扭角(rad，ω·t̂ 沿路径的积分) y=扭率抖动 z=截面锐度 w=丝尖参差
  attribute vec4 aStyle;   // x=丝距倍率 y=丝长倍率 z=宽度剖面(0=收细的锥 1=张开的扇) w=色温偏移

  uniform float uSunR;
  uniform float uTime;
  uniform float uGrow;     // 全局呼吸（很小；主运动已经交给每片的生命期）
  uniform float uTilt;     // 顺场倾倒的总幅度
  uniform float uSweep;    // **弯**：∇×F 的垂直分量（顺场扫出去）
  uniform float uTwist;    // **扭**：∇×F 的场向分量 α 的径向积分（aTwist.x）
  uniform float uPeriod;   // 喷发周期的基准（秒）

  varying vec2 vUv;        // x: 横向 0..1，y: 沿长度 0(根)..1(尖)
  varying vec3 vWorld;
  varying vec2 vSize;      // 这条带子的世界尺寸（x=宽、y=高）：本地 uv ↔ 世界单位
  varying float vSeed;
  varying float vLift;
  varying float vArc;
  varying float vMask;
  varying float vEdge;
  varying float vLife;     // 生命期包络 0..1（片元用它调亮度）
  varying vec4 vStyle;     // 形态参数（片元用它把"每片长不一样"做出来）
  varying vec2 vShape;     // x=截面锐度 y=丝尖参差度

  void main(){
    float uvT = uv.y;
    float x = uv.x - 0.5;

    float arc = aKind.x;
    // ⚠ **参数化必须均匀**：带子的弯曲/扭曲完全由流场决定 ⇒ 曲率在哪**不固定**，
    //   按位置加密只会密在直段上。要更顺只能均匀多加段数（见 sun.js 的 PROM_LOD）。
    float t = uvT;

    // ---- 喷发**生命期**：每片自己的相位/周期，但相位取自**世界空间场** ----
    // 关键在"相干"：相位来自 `aFlow2.z`（一个平缓的三维场）⇒ 相邻带子相位接近，
    // 于是**一片一片地喷**；如果相位直接用每片的随机 seed，观感会是满屏噪点闪烁。
    // 形状是**快升 → 悬停 → 慢落 → 休息**（真日珥就是这个节奏）。
    float period = uPeriod * aFlow2.w;
    float ph = fract(uTime / period + aFlow2.z);
    float env = smoothstep(0.00, 0.16, ph) * (1.0 - smoothstep(0.42, 0.86, ph));

    // 切平面里的两个方向（CPU 已经把它们表示在 aSide/aBend 这组基上）
    vec3 k0 = normalize(aSide * aFlow.x + aBend * aFlow.y);
    vec3 k1 = normalize(aSide * aFlow2.x + aBend * aFlow2.y);
    float mask = aFlow.w;

    // 倒多少：场越强倒得越狠，再叠一点每片的抖动（保留个体差异，但不是"各自 random"）
    float tilt = uTilt * (0.25 + 1.5 * aFlow.z) * (0.55 + 0.90 * aKind.y);
    vec3 up = normalize(aDir + k0 * tilt);
    vec3 side = normalize(aSide - up * dot(aSide, up));
    vec3 bend = normalize(cross(up, side));
    // **整条带子只有一条平滑的弯**：方向由共享场（k1）定 ⇒ 相邻带子弯向同一侧；
    // 每片只抖强度（aKind.y）和 ±0.45 的方向（aParam.z）。
    vec3 bendDir = normalize(k1 + bend * aParam.z);
    float bendMag = uSweep * (0.45 + 0.75 * aKind.y);

    // ⚠ 底**不能**压到 0：生命期只是"在常驻的色球/针状体上**加戏**"，
    //   取 (0.18+0.82·env) 时休息段整条日缘会秃掉（用户看到的第一版就是这个）。
    //   真实太阳的日缘永远有一圈针状体；喷发是它上面长出来的东西。
    float h = aParam.x * uGrow * (0.30 + 0.70 * mask) * (0.45 + 0.55 * env);
    float w = aParam.y * aKind.z * (0.45 + 0.55 * mask);

    // 沿长度的高度剖面：直须是 `t`；拱是 `t(1-arc·t)`（arc→1 时尖端回到日面高度）。
    // ⚠ 用 `t·(1-arc·t)` 而不是 `sin(π·t)`：sin 让**根部斜率不为 0**，拱脚会斜插进日面。
    // `rise` 归一化到峰值 1，于是 aParam.x 在任意 arc 下都是同一个意思（顶点高度）。
    float riseRaw = t * (1.0 - arc * t);
    float riseMax = arc < 0.001 ? 1.0 : (arc <= 0.5 ? (1.0 - arc) : 1.0 / (4.0 * arc));
    float rise = riseRaw / riseMax;

    // **上缘/侧缘要起伏**（一块矩形板最大的破绽就是四条直边）。⚠ 必须放在顶点着色器
    // （几何真的被改）；只在片元里做 alpha 衰减是"淡出"，剪影仍然是直的。
    // ⚠ 幅度**刻意做小**（±19% / ±12%）：给到 ±50% 时放大后读成"毛躁"。
    // ⚠ 采样位置用**稳定的** aDir/aSide，不要用随 t 转的 side，否则上缘噪声沿长度自己变。
    vec3 wq = normalize(aDir + aSide * (x * w / uSunR)) * (uSunR * 1.9);
    float e1 = fbmP(wq + vec3(0.0, 0.0, uTime * 0.020));
    float e2 = fbmP(wq + vec3(17.0, 5.0, uTime * 0.020));
    float hh = h * (0.78 + 0.38 * e1);
    // **沿长度的宽度剖面**：这是"草地"和"有灵性"差得最明显的一处剪影参数 ——
    // `vStyle.z=0` 向尖端收细（细喷流/锥），`=1` 向尖端张开（面纱/扇）。
    // 两种都要在**最尖端收口**（不收就会是一个方块）。
    float prof = mix(pow(max(0.0, 1.0 - 0.90 * t), 1.35), 1.0 + 1.10 * t, vStyle.z)
               * (1.0 - smoothstep(0.93, 1.00, t));
    float ww = w * prof * (0.88 + 0.24 * e2);

    // ---- **扭**：横截面绕带子自己的轴转（真正的 corkscrew）------------------
    // 扭角的**来源**是 force-free 参数 α 沿径向的积分（每片烘在 aTwist.x，见 sunfield.js）。
    // 这里只做几何：把横截面基绕**局部切线**用 Rodrigues 转 θ。
    // ⚠ 上一版"扭"是让 `side` 对每步的 `up` 重正交化 —— 那等价于让整片跟着弯曲方向转，
    //   曲率一大就成了**折板**（用户看到的第一版：一堆尖角）。真正的扭是**绕切线转**，
    //   中轴不折，只是横截面转 ⇒ 带宽在投影里时宽时窄（转到侧面时几乎成一条线），
    //   这正是"扭"在静止画面里的样子。
    vec3 tang = normalize(up + bendDir * (2.0 * bendMag * t));   // 中轴切线（含扫出去的项）
    float th = uTwist * aTwist.x * (0.60 + 0.80 * aTwist.y) * rise;
    float ct = cos(th), st = sin(th);
    vec3 sideT = side * ct + cross(tang, side) * st + tang * dot(tang, side) * (1.0 - ct);
    // Rodrigues 保长度但不保证与切线垂直（`side ⊥ up` 而切线 ≠ up）⇒ 再正交化一次
    sideT = normalize(sideT - tang * dot(sideT, tang));

    // **曲面**（不是平板）：横向偏移换算成角度，于是整片带子贴着球面弯
    // （`rad` 处的弧长 `s` 对应 `s/rad` 弧度）。
    float rad = uSunR - 0.015 + hh * rise;
    vec3 dv = up
            + sideT * ((x * ww) / rad)
            + bendDir * (bendMag * hh * t * t / rad);
    vec3 p = normalize(dv) * rad;

    vUv = vec2(uv.x, t);
    // 片上纹理是**本地空间**的（见 prom.frag）：用这条带子自己的宽/高把 uv 换算到
    // 世界尺度 ⇒ 丝的粗细对任何尺寸的带子都一致，而纹理跟着带子走。
    vSize = vec2(ww, hh);
    vSeed = aParam.w;
    vLift = t;
    vArc = arc;
    vMask = mask;
    vEdge = 0.45 + 0.55 * e1;
    vLife = env;
    vStyle = aStyle;
    vShape = aTwist.zw;
    vec4 wp = modelMatrix * vec4(p, 1.0);
    vWorld = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
#endif // PX_SUN_PROM_VERT
