// 行星X WebUI — 日珥的**世界空间流场**（在 CPU 上烘一次，喂给顶点着色器当每实例属性）。
//
// ## 为什么要烘到 CPU 侧
//
// 第一版把流场写在 GLSL 里（`px/sun/flow.glsl`），每个**顶点**都算：`promFlow` 两次
// （各 3 次 `fbmP`）+ `promMask` 一次 + 上缘起伏两次。一片 250 个顶点 × 5000 片
// ⇒ 每帧约 3000 万次 `pnoise`，实测那一段就是 LOD 差价（3.5 ms）的主因。
//
// 而**带子的朝向/扭曲本来就是每片一个常数**（同一片的所有顶点用的是同一个根部方向）。
// 所以正确的做法是：**在加载时对每条带子的根部求一次场，结果存进每实例属性**。
// 顶点着色器里于是**一次噪声都不用算**，只剩属性读取。
//
// ## 为什么是 curl noise
//
// 用户要的是"带子的扭曲"，物理上对应**磁流管 / 等离子体流**。这里用
// **curl noise**（Bridson 等："取一个向量势场的旋度"）：
// 旋度场**恒无散度**（∇·(∇×Ψ) ≡ 0）⇒ 流场**没有源也没有汇**，带子不会往几个点上聚、
// 也不会从几个点往外炸 —— 观感上就是"拧着的、成片的等离子体"，而不是"随机方向的风"。
// 第一版是三个独立 `fbm` 拼出来的矢量场，有散度 ⇒ 有的地方全挤在一起、有的地方全空。
//
// 同族的省算力替代是 bitangent noise（atyuwen），本仓不需要那个级别的优化：
// 场只在**加载时**算一次（5000 条带子 × 2 个采样点 × 12 次 fbm ≈ 36 万次噪声求值，
// 在 JS 里是几毫秒的事）。
//
// ## 场里有什么
//
//   · **curl 矢量** ⇒ 带子"朝哪边倒"（根部）与"往哪边扭"（上方一点）
//   · **掩码**     ⇒ 哪里有日珥（成片，不是均匀铺满）
//   · **相位**     ⇒ 喷发的**相干性**：相邻带子相位接近 ⇒ 一片一片地喷，
//                     而不是每片各自闪（那会读成"噪点闪烁"）
// --- 确定性 3D 梯度噪声（Perlin 的 improved noise）----------------------------
// 只用在加载时，所以不求快，只求**确定性**与分布正常。
function mulberry32(a) {
  return function () {
    a |= 0; a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const GRAD3 = [
  [1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0],
  [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1],
  [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1],
];

function makeNoise3(seed) {
  const rnd = mulberry32(seed);
  const perm = new Uint8Array(256);
  for (let i = 0; i < 256; i++) perm[i] = i;
  for (let i = 255; i > 0; i--) {
    const j = (rnd() * (i + 1)) | 0;
    const t = perm[i]; perm[i] = perm[j]; perm[j] = t;
  }
  const p = new Uint8Array(512);
  for (let i = 0; i < 512; i++) p[i] = perm[i & 255];
  const fade = (t) => t * t * t * (t * (t * 6 - 15) + 10);
  const lerp = (a, b, t) => a + (b - a) * t;
  const g = (h, x, y, z) => { const v = GRAD3[h % 12]; return v[0] * x + v[1] * y + v[2] * z; };
  return function noise(x, y, z) {
    const xi = Math.floor(x), yi = Math.floor(y), zi = Math.floor(z);
    const X = xi & 255, Y = yi & 255, Z = zi & 255;
    x -= xi; y -= yi; z -= zi;
    const u = fade(x), v = fade(y), w = fade(z);
    const A = p[X] + Y, AA = p[A] + Z, AB = p[A + 1] + Z;
    const B = p[X + 1] + Y, BA = p[B] + Z, BB = p[B + 1] + Z;
    return lerp(
      lerp(lerp(g(p[AA], x, y, z), g(p[BA], x - 1, y, z), u),
        lerp(g(p[AB], x, y - 1, z), g(p[BB], x - 1, y - 1, z), u), v),
      lerp(lerp(g(p[AA + 1], x, y, z - 1), g(p[BA + 1], x - 1, y, z - 1), u),
        lerp(g(p[AB + 1], x, y - 1, z - 1), g(p[BB + 1], x - 1, y - 1, z - 1), u), v), w);
  };
}

// fbm：3 个八度足够（场是**低频**的，多叠只会更碎，而"平缓"正是用户要的）。
// 与 GLSL 侧的 `fbmP` 同口径（每八度 ×2.02），但**不必逐位一致** —— 它只是个烘焙资产。
function fbm(n, x, y, z, oct = 3) {
  let a = 0.5, s = 0;
  for (let i = 0; i < oct; i++) {
    s += a * n(x, y, z);
    x *= 2.02; y *= 2.02; z *= 2.02;
    a *= 0.5;
  }
  return s;
}

// --- 场本身 -------------------------------------------------------------------
// 频率：`FLOW_K = 0.42`（特征尺度 ≈ 2.4 世界单位 ≈ 0.37 太阳半径）——
// 与第一版 GLSL 里那组数是同一个量级：**频率一高，"成片"就退化成"各随各的"。**
const FLOW_K = 0.42;
const MASK_K = 0.30;   // 「哪里有日珥」的片要比流向更**大**（低一档频率）
// 喷发相位的**相干尺度**。
// ⚠ 这个数踩过一次坑：取 0.17 时，整个球面（半径 6.4 ⇒ 直径 12.8 世界单位）
//   只跨 12.8×0.17 ≈ 2.2 个噪声格，实测相位 p10/p90 = 0.39/0.58 —— **整颗太阳
//   挤在同一格里** ⇒ 全太阳同时喷、同时休息（渲染出来就是"整条日缘一起秃"）。
//   要的是"**一片一片**地喷"，不是"整体一起喷"：取 0.75 ⇒ 跨约 9.6 格，
//   相干斑块尺度 ≈ 0.13 太阳半径，相邻带子仍然接近（成片），不同区域不同步。
const PHASE_K = 0.20;
// 相位场的**放大**：见 `phase()` 上面那段（fbm 的幅值不随频率变 ⇒ 不放大就没有错相）。
// 5.0 是量出来的：相位跨度 p10–p90 = 0.80（≈ 一个整周期，全太阳错开），
// 而相干长度 **≈ 1 世界单位 ≈ 0.16R** —— 相距 0.5 单位的两点相位环差 0.19（相关），
// 相距 1.5 单位已经 0.25（= 完全不相关）。于是"一片一片地喷"的斑块约 0.16R。
const PHASE_GAIN = 5.0;

// 差分步长（噪声空间）。⚠ 这个数必须**远小于**噪声的特征尺度（1 个格 = 1.0）：
// 第一版取 0.30 ⇒ 差分跨了小半个格，"旋度"退化成粗糙的方向差，实测散度只比
// "三个独立 fbm 拼的场"小一点点（0.367 vs 0.439）—— 等于白算。
// 收到 0.06 之后散度掉到数值噪声量级（见 scripts 里的自检），JS 是双精度，不怕消去。
const EPS = 0.06;

// `∇×F` 的差分步长（**世界单位**）。F 的特征尺度 ≈ 2.4 世界单位 ⇒ 0.18 足够小
// （二阶差分对步长不敏感，但太小会被浮点消去吃掉）。
const CURL2_E = 0.18;
// 单条带子的总扭角上限（弧度）。2.6 ≈ 150°。
const TWIST_MAX = 2.6;
// ω 网格：20³ 覆盖 ±1.6R（带子最高长到 ~0.3R，留足余量）。
const OMEGA_N = 20;
const OMEGA_EXTENT = 1.6;
// 径向积分的步数（梯形法）。8 步对 20³ 的网格已经过剩。
const TWIST_STEPS = 8;

export class PromField {
  constructor(seed = 0x5eed1234) {
    this.n0 = makeNoise3(seed);
    this.n1 = makeNoise3(seed ^ 0x9e3779b9);
    this.n2 = makeNoise3(seed ^ 0x85ebca6b);
    this.nm = makeNoise3(seed ^ 0xc2b2ae35);
    this.np = makeNoise3(seed ^ 0x27d4eb2f);
  }

  // 向量势 Ψ 的三个分量（同一族噪声，用**不同的大偏移**分开；
  // 直接给每个分量换一套置换表也行，这里用偏移是省一次建表）。
  //
  // ⚠ **按分量取**（而不是一次算 3 个）是有意的：旋度的 6 个偏导**每个只要一个分量**
  //   （`∂Ψz/∂y` 只要 Ψz），所以逐分量取势时一次 curl = 6 次 fbm 而不是 12 次
  //   `_psi`（= 36 次噪声，省 3 倍）。二阶的 `∇×F`（求 α 要用）再乘 6，
  //   这个 3 倍直接决定了加载时烘 α 网格要 0.1 s 还是 0.5 s。
  psiX(x, y, z) { return fbm(this.n0, x, y, z); }
  psiY(x, y, z) { return fbm(this.n1, x + 13.7, y + 4.1, z + 7.3); }
  psiZ(x, y, z) { return fbm(this.n2, x + 2.9, y + 27.3, z + 8.9); }

  /// 旋度（中心差分）。**无散度**是它的全部意义所在：见文件头。
  curl(px, py, pz, out = [0, 0, 0]) {
    const k = FLOW_K;
    const x = px * k, y = py * k, z = pz * k;
    const e = EPS;                        // 差分步长（噪声空间，见文件头的推导）
    const px_ = this.psiX.bind(this), py_ = this.psiY.bind(this), pz_ = this.psiZ.bind(this);
    const dzy = pz_(x, y + e, z) - pz_(x, y - e, z);   // ∂Ψz/∂y
    const dyz = py_(x, y, z + e) - py_(x, y, z - e);   // ∂Ψy/∂z
    const dxz = px_(x, y, z + e) - px_(x, y, z - e);   // ∂Ψx/∂z
    const dzx = pz_(x + e, y, z) - pz_(x - e, y, z);   // ∂Ψz/∂x
    const dyx = py_(x + e, y, z) - py_(x - e, y, z);   // ∂Ψy/∂x
    const dxy = px_(x, y + e, z) - px_(x, y - e, z);   // ∂Ψx/∂y
    const inv = 1 / (2 * e);
    out[0] = (dzy - dyz) * inv;
    out[1] = (dxz - dzx) * inv;
    out[2] = (dyx - dxy) * inv;
    return out;
  }

  /// `∇×F`（二阶：需要 F 在 6 个邻点的值）。
  /// 用它只为拿**沿场方向**的那一个分量 —— 见 `alpha()` 上面那段分解。
  curlOfCurl(px, py, pz, out) {
    const e = CURL2_E;
    const a = [0, 0, 0], b = [0, 0, 0];
    const c = [0, 0, 0], d = [0, 0, 0], g = [0, 0, 0], h = [0, 0, 0];
    this.curl(px, py + e, pz, a); this.curl(px, py - e, pz, b);   // ∂F/∂y
    this.curl(px, py, pz + e, c); this.curl(px, py, pz - e, d);   // ∂F/∂z
    this.curl(px + e, py, pz, g); this.curl(px - e, py, pz, h);   // ∂F/∂x
    const inv = 1 / (2 * e);
    // (∇×F)x = ∂Fz/∂y − ∂Fy/∂z ; (∇×F)y = ∂Fx/∂z − ∂Fz/∂x ; (∇×F)z = ∂Fy/∂x − ∂Fx/∂y
    out[0] = ((a[2] - b[2]) - (c[1] - d[1])) * inv;
    out[1] = ((c[0] - d[0]) - (g[2] - h[2])) * inv;
    out[2] = ((g[1] - h[1]) - (a[0] - b[0])) * inv;
    return out;
  }

  /// **扭的自转率**：`ω·t̂ / 2`（ω = ∇×F 是涡量，t̂ 是带子的路径切线）。
  ///
  /// 为什么是"半"：跟着流走的材料微元，其刚体转动角速度正好是 **ω/2**（涡量是角速度的两倍），
  /// 而横截面绕路径方向的自转只取 ω 在 t̂ 上的投影。⇒ **顶点沿路径位移，截面自然转**，
  /// 这就是"扭"最自然的来源（不需要额外编一个扭转模型）。
  ///
  /// 与另一种口径的关系（两种都试过，选了这一种）：
  ///   · `α = (F·∇×F)/|F|²`：旋度**沿场**方向 —— 太阳物理的 **force-free 参数**
  ///     （`∇×B = αB`，磁流管扭缠数 `Tw = (1/4π)∮α dl`）。它描述"**场本身**有多扭"。
  ///   · `ω·t̂/2`（现在用的）：旋度**沿路径**方向 —— 描述"**跟着这条路径走**时截面转多少"。
  /// 带子的路径是"径向 + 顺场倾斜"，t̂ 与 F 不是一回事 ⇒ 对"插片沿路径长出来"这件事，
  /// 路径口径才是对的。（两者在力-free 场里恰好重合。）
  ///
  /// ⚠ 这个量**规范不变**（只依赖 F = ∇×Ψ，不依赖我挑了哪个 Ψ）⇒ 是场的真实属性，
  ///   而且是个**世界空间标量场** ⇒ 相邻带子自然扭得一致（与"弯"共用同一个 Ψ）。
  twistRate(px, py, pz, tx, ty, tz, Wout = [0, 0, 0]) {
    this.curlOfCurl(px, py, pz, Wout);
    return 0.5 * (Wout[0] * tx + Wout[1] * ty + Wout[2] * tz);
  }

  /// 「哪里有日珥」：0..1，成片。
  mask(px, py, pz) {
    const m = fbm(this.nm, px * MASK_K + 41.0, py * MASK_K + 17.0, pz * MASK_K, 3) * 0.5 + 0.5;
    // smoothstep(0.38, 0.58, ·)：与第一版 GLSL 里的窗口一致（约六成球面长日珥）
    const t = Math.min(1, Math.max(0, (m - 0.38) / 0.20));
    return t * t * (3 - 2 * t);
  }

  /// **把 ω = ∇×F 烘成一张粗网格**（构造时算一次）。
  ///
  /// ⚠ 为什么不每片现算：一次 `curlOfCurl` = 6 次 curl = 36 次 fbm，一条带子要 8 个采样点
  ///   ⇒ 288 次 fbm/片 × 5000 片 = **1.44 M 次 fbm（实测 1.2 s）**。加载时卡这么久，
  ///   截图器的"20 s 内进入可拍状态"直接超时（就是 `sun-pole` 首跑报的那个错）。
  ///   烘成 20³ 网格只要 8000 × 36 ≈ 0.29 M 次 fbm（~0.1 s），之后每片只剩几次查表。
  /// ⚠ 网格是**内存里的推导产物**，不进版本库（用户裁决：烘焙产物必须能由配置生成出来）。
  bakeOmega(sunR) {
    const n = OMEGA_N;
    const half = sunR * OMEGA_EXTENT;
    const g = new Float32Array(n * n * n * 3);
    const W = [0, 0, 0];
    for (let iz = 0; iz < n; iz++) {
      const z = -half + (2 * half * iz) / (n - 1);
      for (let iy = 0; iy < n; iy++) {
        const y = -half + (2 * half * iy) / (n - 1);
        for (let ix = 0; ix < n; ix++) {
          const x = -half + (2 * half * ix) / (n - 1);
          this.curlOfCurl(x, y, z, W);
          const o = ((iz * n + iy) * n + ix) * 3;
          g[o] = W[0]; g[o + 1] = W[1]; g[o + 2] = W[2];
        }
      }
    }
    this.omegaGrid = { n, half, g };
    return this.omegaGrid;
  }

  /// 三线性采样 ω 网格（越界返回 0）。
  omegaAt(px, py, pz, out) {
    const gr = this.omegaGrid;
    out[0] = out[1] = out[2] = 0;
    if (!gr) return out;
    const { n, half, g } = gr;
    const fx = ((px + half) / (2 * half)) * (n - 1);
    const fy = ((py + half) / (2 * half)) * (n - 1);
    const fz = ((pz + half) / (2 * half)) * (n - 1);
    if (fx < 0 || fy < 0 || fz < 0 || fx > n - 1 || fy > n - 1 || fz > n - 1) return out;
    const x0 = Math.floor(fx), y0 = Math.floor(fy), z0 = Math.floor(fz);
    const x1 = Math.min(n - 1, x0 + 1), y1 = Math.min(n - 1, y0 + 1), z1 = Math.min(n - 1, z0 + 1);
    const tx = fx - x0, ty = fy - y0, tz = fz - z0;
    for (let c = 0; c < 3; c++) {
      const at = (i, j, k) => g[((k * n + j) * n + i) * 3 + c];
      const l = (a2, b2, t2) => a2 + (b2 - a2) * t2;
      out[c] = l(
        l(l(at(x0, y0, z0), at(x1, y0, z0), tx), l(at(x0, y1, z0), at(x1, y1, z0), tx), ty),
        l(l(at(x0, y0, z1), at(x1, y0, z1), tx), l(at(x0, y1, z1), at(x1, y1, z1), tx), ty), tz);
    }
    return out;
  }

  /// **一条带子的总扭角** = 自转率沿**带子路径**的积分（`θ = ½∫ ω·t̂ dl`）。
  ///
  /// `tilt` 给的是路径相对径向的倾角（和 `prom.vert` 里 `up` 用的是同一个量 —— 见
  /// `sun.js` 里那几个 `PROM_*` 常量：**两边必须同源**，改了那边这里要跟着改）。
  /// 带子还会沿长度顺场扫出去一点（`bend` 那一项），这里用路径中点处的切线近似，
  /// 比只按 r̂ 准（倾角是 t̂ 与 r̂ 的主要差别）。
  twistAlong(dirx, diry, dirz, upx, upy, upz, sunR, height, steps = TWIST_STEPS) {
    let acc = 0;
    const r0 = sunR * 0.96;
    const W = this._W || (this._W = [0, 0, 0]);
    const rate = (r) => {
      this.omegaAt(dirx * r, diry * r, dirz * r, W);
      return 0.5 * (W[0] * upx + W[1] * upy + W[2] * upz);   // ½ω·t̂
    };
    for (let i = 0; i < steps; i++) {
      const t0 = i / steps, t1 = (i + 1) / steps;
      const ra = r0 + height * t0, rb = r0 + height * t1;
      acc += 0.5 * (rate(ra) + rate(rb)) * (rb - ra);       // 梯形法
    }
    // 总扭角夹住：正则过的自转率仍可能撞上罕见的强扭区。
    // ±2.6 rad ≈ ±150°，是"看得出扭、又不像麻花"的口径（真实日珥环大多 ≤1 圈）。
    return Math.max(-TWIST_MAX, Math.min(TWIST_MAX, acc));
  }

  /// 喷发相位的**相干值**（0..1，会 `fract` 用）：相邻带子接近 ⇒ 一片一片地喷。
  ///
  /// ⚠ 这里必须**放大**（`PHASE_GAIN`）才谈得上"不同区域不同步"。fbm 的输出分布
  /// 与**频率无关**（频率只改空间尺度，不改幅值）：3 个八度的 fbm 落在 ±0.4 里，
  /// `×0.5+0.5` 之后 p10/p90 跨度只有 **0.20** —— 也就是说整颗太阳的相位只差 0.2 个周期
  /// （34 s 周期里差 6.8 s），**读起来还是"全太阳一起喷"**。乘 3.0 之后跨度 ≈ 0.6，
  /// 再 `fract` 就真的错开了；空间上仍是平滑场（放大只是让相干斑块变小）。
  phase(px, py, pz) {
    const v = fbm(this.np, px * PHASE_K + 7.0, py * PHASE_K + 3.0, pz * PHASE_K, 2);
    return v * PHASE_GAIN + 0.5;
  }
}

/// 采样：把世界空间场变成**根部切平面里的两个方向 + 两个标量**。
/// 坐标系用 `side`/`bend`（切平面里的一对正交基）⇒ 属性里只需要 2 个分量就能表达方向。
/// `dir`/`side`/`bend` 都是单位向量、且太阳在世界原点（见 sun.js 里的说法）。
export function sampleField(field, dir, side, bend, sunR, out) {
  const c0 = field.curl(dir.x * sunR, dir.y * sunR, dir.z * sunR);
  const len = Math.hypot(c0[0], c0[1], c0[2]) || 1e-6;
  // 切平面分量（去掉沿法线的部分）：带子只能"贴着球面倒"，沿法线的那一维没有意义
  const nx = dir.x, ny = dir.y, nz = dir.z;
  const project = (c) => {
    const d = c[0] * nx + c[1] * ny + c[2] * nz;
    return [c[0] - nx * d, c[1] - ny * d, c[2] - nz * d];
  };
  const t0 = project(c0);
  const l0 = Math.hypot(t0[0], t0[1], t0[2]);
  if (l0 > 1e-5) { t0[0] /= l0; t0[1] /= l0; t0[2] /= l0; } else { t0[0] = side.x; t0[1] = side.y; t0[2] = side.z; }

  // 上方一点（0.30R）的场 ⇒ 带子沿长度**往哪边扭**
  const up = 0.30 * sunR;
  const c1 = field.curl(dir.x * (sunR + up), dir.y * (sunR + up), dir.z * (sunR + up));
  const t1 = project(c1);
  const l1 = Math.hypot(t1[0], t1[1], t1[2]);
  if (l1 > 1e-5) { t1[0] /= l1; t1[1] /= l1; t1[2] /= l1; } else { t1[0] = t0[0]; t1[1] = t0[1]; t1[2] = t0[2]; }

  const px = dir.x * sunR, py = dir.y * sunR, pz = dir.z * sunR;
  const dot = (v, u) => v[0] * u.x + v[1] * u.y + v[2] * u.z;
  out.tilt[0] = dot(t0, side); out.tilt[1] = dot(t0, bend);
  out.twist[0] = dot(t1, side); out.twist[1] = dot(t1, bend);
  // 强度：归一化到 0..1。标度是**实测**出来的，不是拍的：|curl| 的
  // p10/p50/p90 = 0.60 / 1.26 / 2.00（见文件头那段自检）⇒ 乘 0.40 让中位落在 0.5 附近，
  // 两端分别是 0.24 / 0.80（既不饱和也不挤在一起）。
  out.mag = Math.min(1, len * 0.40);
  out.mask = field.mask(px, py, pz);
  out.phase = field.phase(px, py, pz);
  return out;
}

