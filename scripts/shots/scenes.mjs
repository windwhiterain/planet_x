// 场景定义：**命名场景 = 一组固定 URL 参数 + 固定相机 + 固定时间轴**。
//
// 场景测试要能回答的是「这个物体的画面变了没有」，所以每个场景都要**隔离**被测对象：
// 质量档钉死（`q`）、自适应分辨率关掉（`tune=adaptive:0`）、UI 标记关掉（`hide`）、
// 时间轴冻结（`t`）、行星撤走（`hide=bodies`，否则一颗行星飘到日面前面，判据就不可复现）。
//
// 相机几何**解析算出来**，不靠手调：太阳在原点、半径 `TUNING.sunRadius`、相机 fov 50°，
// 于是「日面半径多少像素」「日缘落在哪一行」都是闭式的 —— 判据的采样窗口才稳。
import { TUNING } from '../../web/static/map3d/tuning.js';

export const FOV_DEG = 50;
export const SUN_R = TUNING.sunRadius;

const rad = (d) => (d * Math.PI) / 180;
const norm = (v) => {
  const l = Math.hypot(...v) || 1;
  return v.map((x) => x / l);
};

// 球在屏幕上的角半径 → 像素半径。vp = [w,h]；d = 相机到球心距离。
export function diskRadiusPx(d, vp) {
  return (Math.tan(Math.asin(Math.min(1, SUN_R / d))) / Math.tan(rad(FOV_DEG) / 2)) * (vp[1] / 2);
}

// 「看整颗太阳」：相机在 +z 方向抬一点（OrbitControls 的 maxPolarAngle=0.47π 会夹位置，
// 抬不够就会被 update() 推走 → 相机不在你以为的地方 → 判据全错）。
export function diskView(d) {
  const tilt = rad(14);                       // 离黄道面的仰角：> tan(5.4°) 即可不被夹
  return { pos: [0, Math.sin(tilt) * d, Math.cos(tilt) * d], target: [0, 0, 0] };
}

// 「看日缘」：目标点取**从相机看过去的轮廓点** L（满足 (L−P)·L = 0 ⇒ P·L = R²），
// 于是日缘正好横穿画面中心，一半日面一半虚空 —— 日珥就是在这条线上量的。
// phiDeg：相机离黄道面的仰角（90° = 正上方俯视极区）。
export function limbView(d, phiDeg = 74) {
  const u = [0, Math.sin(rad(phiDeg)), Math.cos(rad(phiDeg))];
  const s = SUN_R / d;
  const c = Math.sqrt(Math.max(0, 1 - s * s));
  const target = [SUN_R * c, SUN_R * s * u[1], SUN_R * s * u[2]];
  const pos = u.map((x) => x * d);
  return { pos, target };
}

// 相机到轮廓点的距离（用来算「画面里 1 世界单位 = 多少像素」）。
export function limbScale(d, phiDeg = 74) {
  const v = limbView(d, phiDeg);
  const dist = Math.hypot(v.target[0] - v.pos[0], v.target[1] - v.pos[1], v.target[2] - v.pos[2]);
  return { dist, pxPerUnit: (vp) => (vp[1] / 2) / (Math.tan(rad(FOV_DEG) / 2) * dist) };
}

const BASE_TUNE = ['adaptive:0'];             // 自适应分辨率会让同一场景的采样率飘
const BASE_HIDE = ['labels', 'markers', 'bodies'];

export const SCENES = [
  {
    name: 'sun-disk',
    desc: '整个日面：配色/暗条/白饼比例（对照 SDO 304Å 参考图）',
    viewport: [1280, 800],
    query: { q: 'ultra', tune: BASE_TUNE, hide: BASE_HIDE, t: 12 },
    view: diskView(20.1),
    geom: { kind: 'disk', d: 20.1 },
    hold: 900,
    limits: {
      // 背景必须是**黑的**：日冕/后处理糊满屏就会顶穿这条。
      // ⚠ 下限不能取 0：天上有**游戏自己的星云**（四角均值 ≈ 20–22，
      // 见 sun-wide 场景的 19.3）。真正要卡的是「雾」，那由 ringFarLum 看。
      skyMeanLum: [0, 30],
      // 1.6–2.2 R 处的环带：日冕可以亮，但不能把整幅画面抬起来（曾经是 48）
      ringFarLum: [0, 60],
      // 日面是暖色（R>B），参考图远不止 40
      diskWarmth: [40, 260],
      // ACES 压成白饼的比例：参考图几乎没有纯白
      diskWhiteFrac: [0, 0.30],
      // 日面要有结构（米粒/暗条），不是一块平色
      diskP90minusP50: [3, 400],
    },
  },
  {
    name: 'sun-limb',
    desc: '日缘特写：针状体/日珥的参差度（一半日面一半虚空）',
    viewport: [1280, 800],
    query: { q: 'ultra', tune: BASE_TUNE, hide: BASE_HIDE, t: 12 },
    view: limbView(9.0, 74),
    geom: { kind: 'limb', d: 9.0, phi: 74, silThr: 55, silMaxR: 1.25 },
    hold: 900,
    limits: {
      // 日缘必须有伸出日面的结构：轮廓半径的分布太窄 = 一圈硬环、没有日珥。
      // 参考图（SDO 304Å，135 px 半径）的 spread 是 5 px ≈ 3.7% R —— 按比例换到本场景
      // （R≈864 px）约 30 px。下限取 12 px：有针，但不能是"一圈毛边"。
      edgeSpread: [12, 900],
      // 贴日面那一圈（色球/针状体）必须比更外圈亮 —— 否则「没有日珥」
      promContrast: [1.15, 1000],
    },
  },
  {
    name: 'sun-wide',
    desc: '整系统取景：日冕可以亮，但**不许把行星糊掉**（背景要黑）',
    viewport: [1280, 800],
    query: { q: 'ultra', tune: BASE_TUNE, hide: ['labels', 'markers'], t: 12 },
    view: null,                                // 用页面自己的 fitCamera 取景
    geom: { kind: 'disk', d: 20.1 },           // 只作参考：这里的日面不是画面中心
    hold: 900,
    limits: {
      cornerMeanLum: [0, 40],
    },
  },
  {
    name: 'earth',
    desc: '回归场景：地球取景（确认没把别的天体改坏）',
    viewport: [1280, 800],
    query: { q: 'ultra', tune: BASE_TUNE, hide: ['labels', 'markers'], t: 12, focus: '地球', dist: 18 },
    view: null,
    geom: { kind: 'none' },
    hold: 900,
    limits: {
      skyMeanLum: [0, 30],
      centerMeanLum: [8, 250],
    },
  },
];

export const sceneByName = (n) => SCENES.find((s) => s.name === n);

// --- 相机解析 ---------------------------------------------------------------
// 判据的采样窗口必须落在**我知道的那个位置**上，所以相机自己算一遍：给 pos/target/fov/视口，
// 就能把任意世界点投到像素坐标（日面圆心、日缘、地球……），不必去页面里问 three。
export function makeCamera(pos, target, vp, fovDeg = FOV_DEG) {
  const fwd = norm([target[0] - pos[0], target[1] - pos[1], target[2] - pos[2]]);
  const worldUp = [0, 1, 0];
  const right = norm([
    fwd[1] * worldUp[2] - fwd[2] * worldUp[1],
    fwd[2] * worldUp[0] - fwd[0] * worldUp[2],
    fwd[0] * worldUp[1] - fwd[1] * worldUp[0],
  ]);
  const up = [
    right[1] * fwd[2] - right[2] * fwd[1],
    right[2] * fwd[0] - right[0] * fwd[2],
    right[0] * fwd[1] - right[1] * fwd[0],
  ];
  const tanHalf = Math.tan(rad(fovDeg) / 2);
  const aspect = vp[0] / vp[1];
  const project = (p) => {
    const v = [p[0] - pos[0], p[1] - pos[1], p[2] - pos[2]];
    const z = v[0] * fwd[0] + v[1] * fwd[1] + v[2] * fwd[2];
    if (z <= 1e-6) return null;
    const x = v[0] * right[0] + v[1] * right[1] + v[2] * right[2];
    const y = v[0] * up[0] + v[1] * up[1] + v[2] * up[2];
    const ndcX = x / z / (tanHalf * aspect);
    const ndcY = y / z / tanHalf;
    return [(ndcX * 0.5 + 0.5) * vp[0], (1 - (ndcY * 0.5 + 0.5)) * vp[1]];
  };
  return { pos, target, fwd, right, up, project, aspect, tanHalf };
}

export function sceneUrl(base, scene) {
  const q = new URLSearchParams();
  const { query = {} } = scene;
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined || v === null || v === false) continue;
    q.set(k, Array.isArray(v) ? v.join(',') : String(v));
  }
  return `${base}/?${q.toString()}`;
}

export { norm };
