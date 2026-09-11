#ifndef PX_SUN_PROM_VERT
#define PX_SUN_PROM_VERT
  // 日珥插片（**世界空间的曲面片**）。
  //
  // 为什么最后选了插片而不是体积积分或高度场求交：见 `.agents/notes/sun-prominence.md`。
  // 一句话——日缘附近的视线几乎与日面**平行**，任何"沿视线积分"的做法都会把几十根针
  // **平均成一层雾**；而"每像素解析求交"虽然能出形状，却是 2.5D 的、没有真正的三维遮挡
  // 与视差。插片是**真的几何**：世界坐标、真遮挡、真视差、剪影天然正确，而且便宜
  // （几千个 12 顶点的片，几万三角形）。
  //
  // 每一片 = 一条**弯的**带子（不是平 quad）：沿长度方向按 `curve` 弯出去 ⇒ 「很多曲面片」。
  // 形状完全在顶点着色器里由 per-instance 属性生成，几何只有一份单位网格。
  attribute vec3 aDir;     // 根部方向（球面上的一点，单位向量）= 片的"上"轴
  attribute vec3 aSide;    // 片宽方向（与 aDir 正交）
  attribute vec3 aBend;    // 弯曲方向（与 aDir、aSide 正交）：带子朝这边弯出去
  attribute vec4 aParam;   // x=长度 y=宽度 z=弯曲量 w=相位/种子

  uniform float uSunR;
  uniform float uTime;
  uniform float uGrow;     // 整体生长/脉动（>1 会短暂变高，做"喷发"感）

  varying vec2 vUv;        // x: 横向 0..1，y: 沿长度 0(根)..1(尖)
  varying vec3 vWorld;
  varying float vSeed;
  varying float vLift;

  void main(){
    float t = uv.y;
    float x = uv.x - 0.5;

    float h = aParam.x * uGrow;
    float w = aParam.y;
    float curve = aParam.z;
    float seed = aParam.w;

    // 横向收窄 ⇒ 尖头（`taper` 的指数决定"针"有多尖：2.2 大约在 60% 高度处就收掉一半）
    float taper = pow(max(1.0 - t, 0.0), 0.75);
    // 根部埋进日面一点：不然片与球的切点会露出一条缝（掠射时特别明显）
    vec3 base = aDir * (uSunR - 0.015);
    vec3 p = base
           + aDir  * (h * t)
           + aBend * (curve * h * t * t)
           + aSide * (x * w * taper);

    vUv = vec2(uv.x, t);
    vSeed = seed;
    vLift = t;
    vec4 wp = modelMatrix * vec4(p, 1.0);
    vWorld = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
#endif // PX_SUN_PROM_VERT
