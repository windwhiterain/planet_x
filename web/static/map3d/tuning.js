// 行星X WebUI — 视觉调参与**质量分档**。
//
// 两件事分开：
//   * `TUNING`  —— 构图/尺度/观感旋钮。改这里就能整体改观感，配合截图迭代；
//                  `PlanetXMap.tune({...})` 在运行时改它（会重建世界）。
//   * `TIERS`   —— 性能档。**只**决定「编多少八度噪声、开几个 pass、多少分辨率」这类
//                  硬件吃得消吃不消的问题，不改变观感取向。
//
// 分档的动机：本机实测 GPU 是 Intel Iris Xe（集显），而无后处理的旧版空转 165fps。
// 加了 HDR 管线 + 多个全屏 pass 之后必须有一条能自动退到「还能看」的路。探测 + 自适应
// 在 index.js 里，这里只给静态表。

export const TUNING = {
  // --- 尺度 ---------------------------------------------------------------
  // 显示半径 = radiusScale × config 的 `radius`（相对类地行星）。保持 config 的相对比例
  // （气巨>冰巨>类地>卫星>矮行星，与真实太阳系次序一致），只把整体尺度压到「太阳明显最大」。
  radiusScale: 2.2,
  radiusMin: 0.26,
  radiusMax: 5.7,
  sunRadius: 6.4,

  // 径向压缩指数 r^orbitExp：越小内太阳系越舒展（外圈/柯伊伯带越挤）。
  orbitExp: 0.42,

  // **轨道倾角**（度）：全系统严格共面时地图是「平铺沙盘」，读图清楚但没有纵深。
  // 给每个天体一个 ≤`orbitIncMax` 的确定性小倾角，整个系统立刻读得出是三维的，
  // 而因为倾角小、又不改任何 2D 投影量，俯视时几乎不影响读图。
  // 设 0 可一键回到严格共面（`PlanetXMap.tune({orbitIncMax:0})`）。
  orbitIncMax: 7.0,
  // 卫星的倾角相对母星再小一档（真实卫星的轨道相对母星赤道面更平）。
  orbitIncMoon: 3.5,

  // 星环：内/外缘 = 相对天体显示半径的倍数（土星主环真实跨度约 1.2–2.3 Rs）。
  ringInner: 1.22,
  ringOuter: 2.28,
  // 环的「厚度」：真实环极薄（~10m），这里给一点点非零厚度纯粹是为了掠射角下不消失。
  ringThickness: 0.012,

  // 卫星与母星之间的最小净空（母星带星环时按环外缘算），不足则沿方位角把卫星推出去。
  moonGap: 1.2,

  // --- 光照 ---------------------------------------------------------------
  // 行星 shader：受光面之外的底光。真实太空里背光面几乎全黑，但全黑会让星球在战略图上
  // 「消失」（连城市都找不到），所以留一点点让暗面仍读得出轮廓。
  shaderAmbient: 0.055,
  // 太阳的 HDR 强度。**现在这个 1.0 是「标定过的一比一」**：photo.frag 的配色锚点是从
  // 参考图（SDO 304Å）经 ACES 反解出来的**绝对 HDR 值**（`run.mjs --solve`），所以这里
  // 就该是 1.0。以前是 1.25 而配色是"看着调"的相对值 ⇒ 日面整体进了 ACES 的平顶区：
  // 实测 (254,238,220)、盘面 p10..p90 只有 5 级 —— 一块白饼，颗粒与暗区全丢。
  sunIntensity: 1.0,
  // 体积层（日冕 + 色球/日珥）的总开关。`?q=<档>` 里的 `corona` 是**质量档**的一部分，
  // 而隔离被测层（只看光球 / 只看体积）是调试与场景测试的常规需求 ⇒ 和 `postfx` 一样
  // 给一条 tune 逃生门。默认 1。
  coronaOn: 1,
  // 曝光交给后期的 ACES：改变这里 = 整体明暗。
  exposure: 0.78,

  // 首帧取景：fitR>0 = 固定取景半径；否则按「当前天体最远距离 × fitMargin」自适应。
  fitR: 0,
  fitMargin: 1.22,
  // 取景时把倾角抬起来的高度也算进去（否则倾斜后的球会顶出画面）。
  fitLift: 0.06,

  // 遮挡淡出：depth∈[-0.10, 0.06] 内把标记淡出（depth=0 表示中心点正好落在天体轮廓上）。
  occlFadeLo: -0.10,
  occlFadeHi: 0.06,

  // --- UI 标记尺寸（屏幕像素） ---------------------------------------------
  reticlePx: 16,        // 阵营准星环（近景会跟着模型大小放大，见 fitWorld）
  farDotPx: 8,          // 远景 billboard（近景换成 3D 模型）
  farDotStationPx: 10,
  labelPx: 13,          // 天体标签 chip 高度
  labelGapPx: 7,        // 标签与天体轮廓之间的间距

  // --- 后处理强度 ---------------------------------------------------------
  bloomStrength: 0.42,
  bloomRadius: 0.55,
  bloomThreshold: 0.90,
  // 体积光（god rays）：从太阳屏幕位置做径向散射。
  // 参与散射的亮度门槛（线性 HDR）。只有真的比白更亮的东西（日面/日冕/羽流）才拉丝，
  // 否则整幅画面会被自己的行星和轨道线糊成一片雾。
  flareThreshold: 0.85,
  // 各向异性拉丝（变形宽银幕镜头那道横线）。
  streakStrength: 0.16,
  // 镜头鬼影（沿「太阳→屏幕中心」连线的几个彩色光圈）。
  // 0.14 时在虚空里读起来是「天上浮着一串靶子」——那是同心环图案的问题（已改成软光斑），
  // 强度也一起压下来：真实镜头反射是**要仔细看才发现**的东西。
  ghostStrength: 0.05,
  // 「相机伪影」三件套：色差 / 暗角 / 胶片颗粒。都刻意做得很轻。
  caStrength: 0.9,      // 以「像素」为单位
  vignette: 0.26,
  grain: 0.022,
  // 桶形畸变默认关掉（克制的电影感：不要鱼眼）。
  barrel: 0.0,

  // 后处理总开关。关掉 = 只留 RenderPass + OutputPass（用来量「场景本身」花了多少时间，
  // 也是弱机/截图调试的逃生门）。
  postfx: true,

  // 自适应分辨率：帧时间超了就降 DPR，回到 60fps 以上再慢慢加回来。
  // 关掉它 = 画面永远清晰但可能卡；`adaptiveMinScale` 是降到多低就不再降（0.6 = 剩下 36% 像素）。
  adaptive: true,
  adaptiveMinScale: 0.62,

  // --- 程序化内容 ---------------------------------------------------------
  // 小行星带 / 柯伊伯带：实例化碎岩。0 = 关。
  beltDensity: 1.0,
  // 程序化星空的整体亮度（乘在 scene.backgroundIntensity 上）。0 = 纯黑背景。
  skyIntensity: 1.0,
  // 天体表面法线的起伏强度（0 = 光滑球，1 = 夸张）。
  relief: 0.42,
  // 夜面城市灯强度（0 = 关）。
  cityLights: 1.0,
  // 云量（只对类地/金星/泰坦这类有大气的类生效）。
  cloudAmount: 1.0,
};

// ---------------------------------------------------------------------------
// 质量档
// ---------------------------------------------------------------------------
// `seg` = 天体球几何分段 [width, height]；`oct` = 着色器 fbm 八度；
// `skyRes` = 程序化星空的 cubemap 每面分辨率（一次生成，之后零成本）。
export const TIERS = {
  ultra: {
    label: 'Ultra',
    dprCap: 2.0,
    msaa: 4,
    seg: [128, 84],
    oct: 7,
    skyRes: 1024,
    stars: 16000,
    prom: 5000,
    promLod: 0,
    bloom: true,
    flare: true,
    grade: true,
    atmo: true,
    corona: true,
    clouds: true,
    relief: true,
    nightLights: true,
    belt: 1.5,
    adaptive: true,
  },
  high: {
    label: 'High',
    dprCap: 2.0,
    msaa: 4,
    seg: [72, 48],
    oct: 5,
    skyRes: 512,
    stars: 6500,
    prom: 3000,
    promLod: 1,
    bloom: true,
    flare: true,
    grade: true,
    atmo: true,
    corona: true,
    clouds: true,
    relief: true,
    nightLights: true,
    belt: 1.0,
    adaptive: true,
  },
  medium: {
    label: 'Medium',
    dprCap: 1.5,
    msaa: 2,
    seg: [56, 36],
    oct: 4,
    skyRes: 384,
    stars: 3800,
    prom: 1800,
    promLod: 1,
    bloom: true,
    flare: false,
    grade: true,
    atmo: true,
    corona: true,
    clouds: true,
    relief: true,
    nightLights: true,
    belt: 0.6,
    adaptive: true,
  },
  low: {
    label: 'Low',
    dprCap: 1.25,
    msaa: 0,
    seg: [40, 26],
    oct: 3,
    skyRes: 256,
    stars: 1800,
    prom: 800,
    promLod: 2,
    bloom: true,
    flare: false,
    grade: false,
    atmo: true,
    corona: false,
    clouds: false,
    relief: false,
    nightLights: false,
    belt: 0.0,
    adaptive: true,
  },
  minimal: {
    label: 'Minimal',
    dprCap: 1.0,
    msaa: 0,
    seg: [32, 20],
    oct: 2,
    skyRes: 128,
    stars: 700,
    prom: 0,
    promLod: 2,
    bloom: false,
    flare: false,
    grade: false,
    atmo: false,
    corona: false,
    clouds: false,
    relief: false,
    nightLights: false,
    belt: 0.0,
    adaptive: false,
  },
};

export const TIER_ORDER = ['minimal', 'low', 'medium', 'high', 'ultra'];

// 探测 GPU 档次。拿不到 unmasked renderer 时退回 medium（保守但还能看）。
// `dpr` 也参与：4K + DPR2 的集显比 1080p 的独显更吃力。
export function detectTier(renderer, dpr) {
  let name = '';
  try {
    const gl = renderer.getContext();
    const dbg = gl.getExtension('WEBGL_debug_renderer_info');
    name = dbg ? String(gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL)) : String(gl.getParameter(gl.RENDERER));
  } catch (e) { name = ''; }
  const s = name.toLowerCase();
  const pixels = (typeof window !== 'undefined' ? window.innerWidth * window.innerHeight : 1920 * 1080) * (dpr || 1);

  // 软件渲染 / 虚拟 GPU：直接最低档，别挣扎。
  if (/swiftshader|llvmpipe|software|basic render|mesa offscreen/.test(s)) return { tier: 'minimal', gpu: name };

  const isIntegrated = /intel|uhd|iris|radeon graphics|vega \d|apple m\d/.test(s);
  const isDiscrete = /nvidia|geforce|rtx|quadro|radeon rx|radeon pro|arc a\d|apple m\d (pro|max|ultra)/.test(s);

  if (isDiscrete && !/intel/.test(s)) return { tier: pixels > 1920 * 1080 * 2.2 ? 'high' : 'ultra', gpu: name };
  if (isIntegrated && !isDiscrete) {
    if (pixels > 2560 * 1440 * 1.4) return { tier: 'low', gpu: name };
    return { tier: pixels > 1920 * 1080 * 1.4 ? 'medium' : 'high', gpu: name };
  }
  return { tier: 'medium', gpu: name };
}

// 把 `?q=` 的值解析成档名（容忍 high/High/2 这种写法）。
export function parseTier(q) {
  if (!q) return null;
  const s = String(q).trim().toLowerCase();
  if (TIERS[s]) return s;
  const n = Number(s);
  if (Number.isFinite(n) && n >= 0 && n < TIER_ORDER.length) return TIER_ORDER[n];
  return null;
}
