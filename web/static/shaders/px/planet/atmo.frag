#ifndef PX_PLANET_ATMO_FRAG
#define PX_PLANET_ATMO_FRAG
  precision highp float;
  uniform vec3  uAtmo;
  uniform vec3  uSunset;
  uniform float uDensity;
  uniform float uPlanetR;
  uniform float uShellR;
  uniform float uScaleH;
  varying vec3 vWorldPos;
  varying vec3 vRel;
  varying vec3 vNormal;

  // 大气是**视线穿过的一层壳**，不是壳面上那个点的属性。所以这里做的是最经典的解析近似：
  //   ① 求视线到球心的垂距 b（碰撞参数）；
  //   ② 视线在壳内的弦长 = sqrt(Rs²-b²)，若 b < Rp 则后半被行星本体挡住，只剩前半；
  //   ③ 密度取**切点高度**上的指数分布。
  // 两者相乘（弦长 × 切点密度）自动给出想要的两件事：贴着实心边缘的一圈亮环（弦最长），
  // 以及盘面上很淡的一层雾（弦最短）。
  //
  // ⚠ 曾经的写法是「取壳面顶点到自己球心的高度 h，thickness = pow(1-h, 0.6)」——
  // 壳上**每个**顶点到球心的距离都正好等于 Rs，于是 h≡1、thickness≡0，整层大气**一点都
  // 没渲染**（现象是行星边缘一圈死黑，且怎么调 uDensity 都没反应）。
  void main(){
    vec3 O = cameraPosition;
    vec3 D = normalize(vWorldPos - O);
    vec3 C = vWorldPos - vRel;              // 天体中心（世界坐标）
    vec3 OC = C - O;
    float tc = dot(OC, D);
    float b2 = max(dot(OC, OC) - tc * tc, 0.0);
    float b = sqrt(b2);

    float half_ = sqrt(max(uShellR * uShellR - b2, 0.0));
    float inner = (b < uPlanetR) ? sqrt(max(uPlanetR * uPlanetR - b2, 0.0)) : 0.0;
    float path = max(half_ - inner, 0.0);
    float rho = (b < uPlanetR) ? 1.0 : exp(-(b - uPlanetR) / max(uScaleH, 1e-4));
    float od = path * rho;

    // 照明的锚点。**不能直接用切点**：b→0（盘面正中）时切点恰好穿过球心 C，
    // normalize(T-C) 就退化成 normalize(0) = NaN ⇒ 盘心冒出一个**跟着镜头走的暗色尖点**
    // （实测：木星/天王星/金星正中都有一道 V 形暗口；关掉大气整层就干净了 —— 这是
    // 「镜头相关的奇怪特效」的真正来源，与 warp 折叠无关）。
    // b < Rp 时改用**视线与行星球面的交点**当锚点：|anchor-C| = Rp，法线良定义；
    // 而且物理上更对 —— 被挡在行星前的那段大气，本来就是被该地表点的太阳高度角照亮的。
    // b ≥ Rp 时 inner = 0，式子退化成切点，与原来一致（那里 |T-C| = b ≥ Rp，本来就安全）。
    vec3 T = O + D * (tc - inner);
    vec3 nT = normalize(T - C);
    vec3 sunDir = normalize(-T);            // 太阳在世界原点
    float mu = dot(nT, sunDir);

    // 颜色沿太阳高度角走：正面顶光 → 青白；掠射 → 落日橙红（Rayleigh 散射的廉价近似）。
    float day = smoothstep(-0.14, 0.40, mu);
    vec3 tint = mix(uSunset, uAtmo, day);
    // 晨昏线上那一圈最亮（真实大气在明暗交界处有一道亮弧）。
    float twilight = exp(-pow(mu / 0.17, 2.0));
    tint = mix(tint, uSunset * 1.3, twilight * 0.55);
    // 背光面几乎不发光，但要留一点点（星光 / 行星反照）。
    float lit = 0.03 + 0.97 * smoothstep(-0.32, 0.14, mu);

    gl_FragColor = vec4(tint * uDensity * od * lit, 1.0);
  }
#endif // PX_PLANET_ATMO_FRAG
