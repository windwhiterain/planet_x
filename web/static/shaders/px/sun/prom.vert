#ifndef PX_SUN_PROM_VERT
#define PX_SUN_PROM_VERT
#include <px/sun/flow.glsl>
  // 日珥插片（**世界空间的大片曲面条带**）。
  //
  // 演变（都在 `.agents/notes/sun-prominence.md`）：等半径球壳 ✗ → 并入日冕体积积分 ✗
  // （掠射视线把几十根针平均成雾）→ 每像素解析求交 ✗（2.5D、没有真遮挡）→
  // 几千根细小插片 ✓（真三维、真遮挡）→ **少数大片曲面条带 + 片元里的世界空间纹理**（现在）。
  //
  // 为什么从"几千根细针"改成"少数大片带子"（用户裁决）：细针版有两个治不好的毛病：
  //   ① 实例数是画面密度的上限 —— 要"密"就得几万根，而每根还是**光板一块**，
  //      于是"密"和"细腻"只能二选一；
  //   ② 每根的朝向是各自 random 的 ⇒ 一片**等距、等倾角的刷子**，形态上完全没有结构
  //      （用户原话：「带子的群面形态太均匀，连朝向都是一样的」）。
  // 现在：**带子只负责"在哪里、多大、往哪倒"**（几千片，足够大，覆盖住整个日缘带），
  // **细腻的须由片元着色器在一个世界空间噪声场里切出来**（见 prom.frag）。
  // 密度不再受实例数限制，纹理也不再受每片的 uv 限制。
  //
  // 朝向由 `promFlow` 这个**平缓的世界空间矢量场**定（见 px/sun/flow.glsl）：
  // 相邻根部的场几乎相同 ⇒ 带子成片地朝同一侧倒，而场本身在球面上缓慢转 ⇒
  // 不同区域倒的方向不同。**这不是"加了随机"，是"去掉了均匀"**。
  attribute vec3 aDir;     // 根部方向（球面上的单位向量）
  attribute vec3 aSide;    // 根部切平面基 1（宽度方向）
  attribute vec3 aBend;    // 根部切平面基 2（侧弯方向）
  attribute vec4 aParam;   // x=高度 y=宽度(弧长) z=侧弯量 w=种子
  attribute vec3 aKind;    // x=拱度(0..1) y=顺场倾倒的抖动 z=宽度倍率

  uniform float uSunR;
  uniform float uTime;
  uniform float uGrow;     // 整体生长/脉动（>1 会短暂变高，做"喷发"感）
  uniform float uTilt;     // 顺场倾倒的总幅度
  uniform float uTwist;    // 顺场弯（中轴沿长度侧移）的总幅度：**方向来自共享场**

  varying vec2 vUv;        // x: 横向 0..1，y: 沿长度 0(根)..1(尖)
  varying vec3 vWorld;
  varying vec2 vSize;      // 这条带子的世界尺寸（x=宽、y=高）：本地 uv ↔ 世界单位 的换算
  varying float vSeed;
  varying float vLift;
  varying float vArc;
  varying float vMask;
  varying float vEdge;     // 每顶点的高度调制（上缘/侧缘的起伏，见下面 e1）

  void main(){
    float uvT = uv.y;
    float x = uv.x - 0.5;

    float arc = aKind.x;
    // ⚠ **参数化必须均匀** —— 这一条是被纠正过的，别再"优化"回去：
    //   带子的弯曲/扭曲完全由 `promFlow` 那个**噪声场**决定 ⇒ 曲率在哪**不固定**。
    //   一度想按位置挪顶点密度（直须密在根部、拱密在拱顶），那是**错**的：
    //   噪声把拐弯挪到哪儿，那个"固定密集的地方"就正好落在直线段上，
    //   而真正拐弯的地方依然只有两三段 ⇒ 折角照旧，还白白浪费了一半顶点。
    //   真正跟得住曲率的只有"按弧长重参数化"，那要在顶点着色器里做积分（不值得）；
    //   所以正确做法就是**均匀密一点 + 按档位 LOD**（见 sun.js 的 PROM_LOD）。
    float t = uvT;

    vec3 root = aDir * uSunR;
    // 「哪里有日珥」：平缓场，成片。用它同时压**高度**和**不透明度**
    // （只压高度不行：高度归零的带子还是一片贴在日面上的宽 patch，会露成亮斑）。
    float mask = promMask(root, uTime);

    // ---- 顺世界空间场倾倒 + **顺场扭** --------------------------------------
    // 这就是用户说的"同一个噪声空间"：**带子的扭曲**取自共用的 `promFlow`。
    //   · 根部朝向 = 根处的场在切平面里的方向；
    //   · 沿长度再取**上方一点**的场，两者按 `t` 混合 ⇒ 中轴顺场**缓慢扭过去**
    //     （相邻带子扭向同一侧 ⇒ 成片，而场在球面上缓慢转 ⇒ 不同区域不同）。
    vec3 F0 = promFlow(root, uTime);
    vec3 Ft = promFlow(root + aDir * (uSunR * 0.30), uTime);
    float fm = length(F0);
    vec3 k0 = promTangential(aDir, F0, aSide);
    // 倒多少：场越强倒得越狠，再叠一点每片的抖动（**保留个体差异**，只是不再是
    // "每片各自 random 一个方向" —— 那是均匀，不是结构）。
    float tilt = uTilt * (0.25 + 1.5 * fm) * (0.55 + 0.90 * aKind.y);
    vec3 up = normalize(aDir + k0 * tilt);          // 生长方向 = 根处的场
    vec3 side = normalize(aSide - up * dot(aSide, up));
    vec3 bend = normalize(cross(up, side));
    // **扭曲**：中轴沿"上方一点的场"平滑扫过去（`t²`）。
    // ⚠ 不要在顶点里让**横截面随 t 转**（`side` 每步重正交化那种做法是"真扭转"，
    //   但只有 8 段顶点 ⇒ 方向一摆就成了**折线**，画面上是一堆带尖角的折板，
    //   实测非常难看）。扭转要的是"整条带子缓缓歪过去"，用中轴的平滑侧移表达就够了，
    //   而且它同样来自那个共享的世界空间场 ⇒ 相邻带子歪向同一侧。
    // **整条带子只有一条平滑的弯曲**：方向由共享场（`sweep`）定 ⇒ 相邻带子弯向同一侧
    // （"整体曲线平滑一致"）；每片只在**强度**上抖（`aKind.y`），方向只抖一点点
    // （`aParam.z`）。早先 `curve` 是每片一个**随符号**的弯曲量 ⇒ 相邻带子朝相反方向弯，
    // 整片日缘读起来是"毛躁、末端还多拐一下"。
    vec3 sweep = promTangential(up, Ft, bend);
    vec3 bendDir = normalize(sweep + bend * aParam.z);
    float bendMag = uTwist * (0.45 + 0.75 * aKind.y);

    float h = aParam.x * uGrow * (0.30 + 0.70 * mask);
    float w = aParam.y * aKind.z * (0.45 + 0.55 * mask);
    float seed = aParam.w;

    // 沿长度的高度剖面：直须是 `t`；拱是 `t(1-arc·t)`（arc→1 时尖端回到日面高度）。
    // ⚠ 用 `t·(1-arc·t)` 而不是 `sin(π·t)`：sin 会让**根部的斜率不为 0**，
    // 拱脚是斜插进日面的（像被风吹歪的草），而真实日珥环的脚是**垂直扎进**色球的。
    // `rise` 归一化到峰值 1，于是 aParam.x 在任意 arc 下都是同一个意思（顶点高度）。
    float riseRaw = t * (1.0 - arc * t);
    float riseMax = arc < 0.001 ? 1.0 : (arc <= 0.5 ? (1.0 - arc) : 1.0 / (4.0 * arc));
    float rise = riseRaw / riseMax;

    // **上缘要起伏**：一块矩形板最大的破绽就是它的四条直边（放大后一眼就是几何体）。
    // 沿**宽度**方向采世界空间噪声去调制高度与宽度 ⇒ 每条带子的顶边和侧边都是波浪形的，
    // 相邻带子共用同一个场，所以起伏也是连续的（不是每片各自抖）。
    // ⚠ 必须放在顶点着色器（几何真的被改了）；只在片元里做 alpha 衰减是"淡出"，
    //    剪影仍然是直的 —— 而这几条边正是"看出是插片"的地方。
    // ⚠ 采样位置用**稳定的** `aDir/aSide`，不要用随 `t` 转的 `side`：否则同一片带子的
    //    上缘噪声会沿长度自己变，起伏就变成了"抖"。
    vec3 wq = normalize(aDir + aSide * (x * w / uSunR)) * (uSunR * 1.9);
    float e1 = fbmP(wq + vec3(0.0, 0.0, uTime * 0.020));
    float e2 = fbmP(wq + vec3(17.0, 5.0, uTime * 0.020));
    // ⚠ 幅度**刻意做小**（±19% / ±12%）：早先给到 ±50% 时，上缘沿宽度大幅起伏，
    //   放大后读成"毛躁"（用户原话），而带子**整体曲线**才是要看的东西。
    //   这里要的只是"边界别是一条数学直线"，不是"把轮廓锯开"。
    float hh = h * (0.78 + 0.38 * e1);
    float ww = w * (0.88 + 0.24 * e2);

    // **曲面**（不是平板）：横向偏移换算成角度，于是整片带子贴着球面弯
    // （`rad` 处的弧长 `s` 对应 `s/rad` 弧度）。这一条就是"曲面片"里的那个"曲面"。
    float rad = uSunR - 0.015 + hh * rise;
    vec3 dv = up
            + side * ((x * ww) / rad)
            + bendDir * (bendMag * hh * t * t / rad);
    vec3 p = normalize(dv) * rad;

    vUv = vec2(uv.x, t);
    // 片上纹理是**本地空间**的（见 prom.frag）：把它换算到"世界单位"需要这条带子自己的
    // 宽/高，一并传过去 —— 这样纹理的频率对**任何尺寸**的带子都是同一套世界尺度，
    // 而参数化仍然长在带子自己身上（不是世界坐标）。
    vSize = vec2(ww, hh);
    vSeed = seed;
    vLift = t;
    vArc = arc;
    vMask = mask;
    vEdge = 0.45 + 0.55 * e1;
    vec4 wp = modelMatrix * vec4(p, 1.0);
    vWorld = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
#endif // PX_SUN_PROM_VERT
