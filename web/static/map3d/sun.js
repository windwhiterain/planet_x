// 行星X WebUI — 太阳。
//
// 旧版是一个 fbm 斑块球 + 一张 canvas 径向渐变 sprite，两个问题：表面像**奶酪**（八度不够、
// 尺度太大，在球面上抽出大块斑驳），边缘是一条硬切口（没有 limb darkening，也没有色球/日冕），
// 所以整颗太阳读起来是「一个橙色贴纸」而不是一颗恒星。
//
// 现在三层叠加，从内到外：
//   ① 光球 photosphere —— 米粒组织(ridged 反相 = 亮米粒+暗沟) + 超米粒流 + 太阳黑子(本影/半影)
//      + **limb darkening**（临边昏暗：I(μ)=1-u1(1-μ)-u2(1-μ)²，同时压亮度并偏红）
//   ② 色球 / 日珥 —— **世界坐标的插片几何**（`prom.vert/frag`，见下面 buildPromAttrs）。
//      走过的弯路（三轮，都写进 `.agents/notes/sun-prominence.md` 了）：
//      先是等半径球壳（没有径向厚度 ⇒ 永远是一圈硬红环），再是"并入日冕的体积积分"
//      —— 但掠射时视线几乎与日面**平行**，穿过三十多根针的间距，积分把针**平均成一层雾**，
//      所以加频率/改强度全都无效；再试"每像素解析求交"，形状有了却是 2.5D、没有真遮挡。
//      **插片才是对的**：几千条细长的**曲面片**（弯的带子）真的长在日面上，世界空间、
//      真遮挡、真视差，剪影天然正确。
//   ③ 日冕 corona —— **世界坐标的体积渲染**：几何是一个球，片元里沿视线做射线步进，
//      密度场在世界坐标里采样。**不是 billboard** —— 平面被行星一挡就是「薄膜破了个洞」，
//      而那把大范围的柔光交给 bloom（摄影上的效果）去做，不用几何去假装。
//
// 全部 HDR（光球输出 ~9、日冕 ~1），超出 1 的部分交给 UnrealBloom 变成辉光——真实感来自
// 「亮的东西真的比白更亮」，而不是在球外画一圈半透明橙色。

import * as THREE from 'three';
import { INC } from './glsl.js';
import { NOISE_GLSL, fbmOct } from './util.js';
import { TUNING } from './tuning.js';
import { PromField, sampleField } from './sunfield.js';

const SUN_VERT = INC('px/sun/sun.vert');

// --- ① 光球 -----------------------------------------------------------------
const SUN_PHOTO_FRAG = INC('px/sun/photo.frag');

// --- ③ 日冕 / 日珥 -----------------------------------------------------------
// ⚠ 这段注释曾经写着「④ 光晕：一张 billboard」—— 那是**体渲染改造之前的化石**，
// 而它真的把一次排查带偏了（用户据此以为黑边是 billboard 的接缝）。已删除。
// 现在的太阳是**三个网格**：光球球体 + 日冕球体（半径 `R*CORONA_OUTER_RATIO`）+ 日珥插片。
// 项目里仅存的 Sprite 全在 markers.js（UI 标记层），与本模块无关。
const CORONA_VERT = INC('px/sun/corona.vert');

// 日冕 = **体渲染**（沿视线对三维体积积分），几何是**世界坐标的球体**，不再是 billboard。
//
// 为什么必须换掉 billboard（用户裁决）：「只要还在用 billboard 这种问题就一直会出现」——
// 一张平面被行星挡住就是「薄膜破了个洞」，这是**几何决定**的，调参救不了；而且它的图案
// 只能锁在屏幕空间（转镜头整片跟着转，实测过）或做近似世界方向，怎么调都别扭。
//
// 现在：片元里从相机出发对日冕体积做射线步进，密度场在**世界坐标**里采样 ⇒
//   ① 转镜头时冕流待在世界里不动；
//   ② 遮挡是真三维遮挡（几何体有真实深度，被行星挡住的层就是不见了，不是被抠了个洞）；
//   ③ 远近两侧沿视线自然累积出厚度 —— 读起来是**体**，不是一张纸。
//
// 步数上限是 uniform（见 util.js 里 NOISE_GLSL 那段：常量上界会被完全展开）。
const CORONA_FRAG = INC('px/sun/corona.frag');

// 体积外半径 = 几倍太阳半径。密度在 0.70×该值处就归零（见 coronaDensity 的外缘窗口），
// 所以球体的硬剪影落在密度为 0 的地方。
const CORONA_OUTER_RATIO = 4.0;

const CORONA_MAT = (sunR, tier) => new THREE.ShaderMaterial({
  uniforms: {
    uFbmOct: fbmOct(Math.max(2, tier.oct - 1)),
    uTime: { value: 0 },
    uSunR: { value: sunR },
    uOuter: { value: sunR * CORONA_OUTER_RATIO },
    uOuterRatio: { value: CORONA_OUTER_RATIO },
    // 日冕是「沿视线累积」的量纲，和光球的 HDR 亮度不是一回事（见 setIntensity）。
    uIntensity: { value: 0.05 },
    // 片元里拿不到 projectionMatrix（three 只在顶点前缀给），自己传
    uProj: { value: new THREE.Matrix4() },
    // 场景深度：体积积分必须在**最近的实体表面**处停下（见 CORONA_FRAG 里的夹断），
    // 否则遮挡物**背后**的介质也会被累加进来。这张纹理来自 postfx 的深度预趟。
    uDepth: { value: null },
    uResolution: { value: new THREE.Vector2(1, 1) },
    uNearFar: { value: new THREE.Vector2(0.1, 1000) },
    uHasDepth: { value: 0 },
    // 这个指数决定「日冕能伸多远」。三个极端都试过，前两个都用错了：
    //   · **2.6**：2.5 R 处仍有 rr^-2.6 = 9% ⇒ 一路糊到行星轨道上（实测：日面之外
    //     整幅画面的角落被抬到 48/255，而天空本身只有 21 ⇒ **一半的亮度是雾**）。
    //   · **9.0**：rr=2 处就低于 2e-3 的早退阈值 ⇒ **日冕本身被裁掉了**
    //     （用户：「你把日晕给删了」）。为消一层雾而把主体删掉，是我搞反了因果。
    //   · **4.2**（现在）：1.5 R ⇒ 0.18（近处的日晕还在），2 R ⇒ 0.054，3 R ⇒ 0.010
    //     ⇒ 远景有晕、近景不糊。这不是"取舍"：2.6 那版的雾**另有主因**（色球壳太厚
    //     + 粗积分里的厚弧道，两条都已单独修掉），只是当时没找出来。
    uFalloff: { value: 4.2 },
    // 步数随档位走：这是每像素最贵的一项，弱机必须能降下来。
    uSteps: { value: Math.max(6, Math.min(18, 4 + tier.oct * 2)) },
  },
  vertexShader: CORONA_VERT,
  fragmentShader: CORONA_FRAG,
  transparent: true,
  blending: THREE.AdditiveBlending,
  depthWrite: false,
  // **不要深度测试**：遮挡判据在着色器里用**解析夹断**做（`t1 = min(t1, tScene)` + discard），
  // 它不依赖深度缓冲，相机在体积内/外/被部分遮挡全都成立。交给深度测试反而会错 ——
  // BackSide 的天然深度是**远壁**，球内一切都会把体积挡掉（「裸体太阳」）。
  depthTest: false,
  // **背面**：相机在体积外时渲染远表面、在体积内时渲染的还是远表面 —— 两种情况都有片元，
  // 不用按位置切换 side（切 side 会改 define ⇒ 触发重编译）。
  side: THREE.BackSide,
});

// --- ②b 日珥插片（大片曲面条带 + 片元里的世界空间纹理）-------------------------
// 分工：**顶点**决定"在哪里、多大、朝哪边倒"（几千片曲面条带），
//      **片元**在一个世界空间噪声场里把带子切成**细腻的须**。
// 为什么不是"几千根细针"（上一版，见 prom.vert 顶部）：实例数就是密度的上限，
// 每根又是光板一块 ⇒ 密和细腻只能二选一；而且每根各自 random 倾角 ⇒ 一片等距刷子。
// 为什么不能用体积积分做（更早的三轮试错）：掠射视线穿过几十根针会把它**平均成雾**；
// "每像素解析求交"能出形状却是 2.5D、没有真遮挡与视差。
const PROM_VERT = INC('px/sun/prom.vert');
const PROM_FRAG = INC('px/sun/prom.frag');

// 上限（按档位只改 `instanceCount`，不重建几何）。
// ⚠ 这个数**从 30000 降到了 5000**：细针版靠实例数堆密度，而每条只有 8x4 个像素那么大，
// "密"永远追不上真实日面；现在每条带子是**大片曲面**（宽度 0.045R~0.16R），
// 密度来自片元里的世界空间纹理 ⇒ 再多的实例也只是白白重叠。
const PROM_MAX = 5000;

// 确定性 PRNG（截图要可复现；**不要**用 Math.random）
function mulberry32(a) {
  return function () {
    a |= 0; a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// 条带的 **LOD 表**：`[横向段, 长度段]`。段数只影响**剪影/曲面有多顺**，
// 纹理的细腻程度是片元的事（§prom.frag），所以低档降段数不会把质感降掉。
//
// ⚠ 为什么这里能谈 LOD、而"硬件曲面细分"不能（用户问过）：WebGL2 = ES 3.0，
//   GLSL ES 3.00 里**没有** tessellation control/evaluation 这两个 stage；
//   WebGPU/WGSL 也没有（core spec 只有 vertex/fragment/compute）。three 的
//   `TessellateModifier`/`ParametricGeometry` 都是 **CPU 侧的一次性预剖分**。
//   而我们的带子曲面是**解析**的（顶点着色器里算 `p(u,v)`）⇒ "细分级别"就等于
//   "一片喂多少顶点"，与硬件特性无关，按档位给就行。
//   `[段数] → 顶点数/片`： 9×24→250、7×16→136、5×10→66（实例数另算）。
// ⚠ 段数只在**长度方向**堆：带子的弯/扭由噪声决定，**曲率在哪不固定**，
//   所以任何"按位置加密"的做法都是白费（会密在直段上）；只能均匀给足。
const PROM_LOD = [[9, 24], [7, 16], [5, 10]];

// ⚠ **这些常量是 JS 与 GLSL 的耦合点**：`prom.vert` 里 `up` / `bendMag` 的算法必须与
//   下面 `buildPromAttrs` 里烘 `up` 的那几行**逐项一致** —— 因为"扭角"是在 CPU 上沿
//   **带子的路径**积分的，而路径方向 `up` 是顶点着色器算的。两边一旦不一致，
//   症状是"扭的方向和带子实际的倾斜对不上"（不报错、只是看着别扭）。
//   所以这两个值只在这里定义一次，uniform 也用它们。
const PROM_TILT = 0.75;    // 顺场倾倒的总幅度
const PROM_SWEEP = 0.34;   // 顺场扫出去（∇×F 的垂直分量 = 弯）
const PROM_TWIST = 2.0;    // 扭的倍率（∇×F 的场向/路径分量沿路径的积分）

// 单位插片：宽 1、长 1。
// ⚠ uv.y = 0 是**根部**、1 是**尖端**（three 的 PlaneGeometry 就是这样）。
// 这里只生成**每实例属性**（与 LOD 无关，5000 条带子的数据不该按 LOD 复制一份）；
// --- 日珥的**形态种群** ------------------------------------------------------
// 用户裁决：*「现在就像种草一样，每个都差不多，没有灵性」*。
// 根因不是"纹理不够细"，而是**整个种群只有一个形态**：每条都是同一个锥形长条 +
// 同一套丝距/丝长，于是读起来是一片草地。
//
// 真太阳上的日珥是**几种形态混着**的（304Å 一眼就能分辨）：
//   spike 细喷流 / bush 篱笆状灌木丛 / sheet 宽面纱 / loop 大弧 / knot 亮结。
// 每种有自己的 (高度, 宽度, 拱度) 分布，**以及自己的一套表面参数**
// （丝距、丝长、宽度剖面、截面锐度、色温偏移、丝尖参差度）⇒ 逐个不同。
//
// 字段含义（都是**乘数/插值权重**，不是绝对量 —— 全局旋钮仍是 TUNING 里那些）：
//   w     该形态占种群的比例
//   h/wid 高度、宽度（单位 = 太阳半径）
//   arc   拱度 0..1（1 = 尖端落回日面）
//   fil   丝距倍率（乘 uFilFreq）  along 丝长倍率（乘 uFilAlong，越小丝越长）
//   prof  宽度剖面 0=向尖端收细(锥) 1=向尖端张开(扇)
//   cross 截面 0=平铺面纱 1=中间一道脊
//   heat  色温偏移   len 丝尖参差度（每根丝末端散开的程度）
//   span  **U 形（马鞍）的两条腿分开多少**（单位 = 高度）。0 = 不收回来（直须）。
//         ⚠ 这是**初始形态**，不是靠场弯出来的（用户裁决："马鞍形我指的是初始形态
//         就是一个 U 形状的，然后在此基础上再受到场影响"）。
const PROM_KINDS = [
  { n: 'spike', w: 0.30, h: [0.030, 0.280], wid: [0.030, 0.120], arc: [0.00, 0.00], span: [0.00, 0.00], fil: [1.30, 1.90], along: [0.50, 0.95], prof: [0.00, 0.15], cross: [0.55, 0.95], heat: [0.05, 0.25], len: [0.30, 0.45] },
  { n: 'bush', w: 0.24, h: [0.012, 0.110], wid: [0.070, 0.260], arc: [0.00, 0.35], span: [0.20, 0.70], fil: [1.50, 2.20], along: [1.10, 2.10], prof: [0.35, 0.65], cross: [0.20, 0.55], heat: [-0.10, 0.10], len: [0.45, 0.62] },
  { n: 'sheet', w: 0.20, h: [0.040, 0.340], wid: [0.110, 0.380], arc: [0.00, 0.45], span: [0.30, 1.10], fil: [0.50, 0.80], along: [0.35, 0.80], prof: [0.10, 0.40], cross: [0.05, 0.30], heat: [-0.05, 0.15], len: [0.25, 0.40] },
  { n: 'loop', w: 0.16, h: [0.090, 0.520], wid: [0.050, 0.190], arc: [0.65, 1.00], span: [0.90, 2.10], fil: [0.60, 1.00], along: [0.65, 1.20], prof: [0.00, 0.22], cross: [0.35, 0.70], heat: [0.00, 0.20], len: [0.30, 0.48] },
  { n: 'knot', w: 0.10, h: [0.010, 0.070], wid: [0.045, 0.150], arc: [0.00, 0.25], span: [0.10, 0.45], fil: [1.80, 2.50], along: [1.60, 3.00], prof: [0.40, 0.75], cross: [0.45, 0.85], heat: [0.15, 0.40], len: [0.50, 0.70] },
];
const PROM_KIND_SUM = PROM_KINDS.reduce((a2, k) => a2 + k.w, 0);
// 按权重抽一个形态（**概率分布而不是阈值**：权重就是它的出现频率）
function pickKind(u) {
  let x = u * PROM_KIND_SUM;
  for (const k of PROM_KINDS) { x -= k.w; if (x <= 0) return k; }
  return PROM_KINDS[PROM_KINDS.length - 1];
}
const lerpR = (ab, u) => ab[0] + (ab[1] - ab[0]) * u;
// **对数均匀**取尺寸：`线性均匀 + pow(·,1.9)` 会把绝大多数压到区间下限附近
// ⇒ 看上去"每条都一样大"（用户裁决："这些带子看起来都一样，你改改大小"）。
// 对数上均匀 ⇒ 每个尺度上都有差不多多的样本，大小差别才**看得出来**。
const lerpLog = (ab, u) => ab[0] * Math.pow(ab[1] / ab[0], u);

// 网格按 LOD 生成多份，见 makePromGeo。
function buildPromAttrs(sunR, field) {
  const rnd = mulberry32(0x5eed1234);
  const dir = new Float32Array(PROM_MAX * 3);
  const side = new Float32Array(PROM_MAX * 3);
  const bend = new Float32Array(PROM_MAX * 3);
  const par = new Float32Array(PROM_MAX * 4);
  const kind = new Float32Array(PROM_MAX * 4);
  // **世界空间流场**烘出来的每实例量（朝向 / 扭转 / 场强 / 掩码 / 喷发相位 / 周期）。
  // 为什么烘在 CPU：见 prom.vert 与 sunfield.js 顶部 —— 这些量**每片就是一个常数**，
  // 让 250 个顶点各算一遍（每顶点约 24 次 pnoise）纯属浪费，实测值 3.5 ms。
  const flowA = new Float32Array(PROM_MAX * 4);
  const flowB = new Float32Array(PROM_MAX * 4);
  const twist = new Float32Array(PROM_MAX * 4);
  // 每片自己的**形态参数**（见 PROM_KINDS）：(丝距倍率, 丝长倍率, 宽度剖面, 色温偏移)。
  // 这一条是"灵性"的关键 —— 以前这些是**全局 uniform**，所以每条带子的表面长得一样。
  const style = new Float32Array(PROM_MAX * 4);
  // 根部处的**原始场矢量**：顶点着色器要用"该顶点处的场 − 它"（差动才有扭/剪切）
  const f0v = new Float32Array(PROM_MAX * 3);
  // ω 网格烘一次（**内存里的推导产物**，不是版本库里的资产；20³ 实测 ~0.13 s）。
  // 只有 `uTwist != 0`（那份**显式积分**的扭）才需要它 —— 现在扭已经交给
  // 顶点自己采样场了（见 `bakeField`），这份留着是为了随时能 A/B。
  field.bakeOmega(sunR);
  const fs = { tilt: [0, 0], twist: [0, 0], mag: 0, mask: 0, phase: 0, f0: [0, 0, 0] };
  const d = new THREE.Vector3(), sv = new THREE.Vector3(), bv = new THREE.Vector3();
  const k0v = new THREE.Vector3(), upv = new THREE.Vector3();   // 复用的临时量
  const up = new THREE.Vector3(0, 1, 0), ex = new THREE.Vector3(1, 0, 0);

  for (let i = 0; i < PROM_MAX; i++) {
    // 根部：**均匀铺满球面**。结构不再是"根部聚类"做出来的 ——
    // 聚类是"局部密、别处也均匀"，而这里要的是**成片**（有整片空白），
    // 那件事交给顶点着色器里的 `promMask`（世界空间场），CPU 侧保持均匀采样最干净。
    const z = rnd() * 2 - 1;
    const phi = rnd() * Math.PI * 2;
    const r = Math.sqrt(Math.max(0, 1 - z * z));
    d.set(r * Math.cos(phi), z, r * Math.sin(phi));
    // 根部标架（aDir 是"法线"，aSide/aBend 是切平面里的一对基）。
    // ⚠ 这里**不再**随机扰动 d 的方向：带子的朝向由 `promFlow` 这个世界空间场决定
    //   （见 prom.vert）。CPU 侧多搅一点随机，就等于把好不容易建立起来的"场"搅没了。
    // ⚠ **根部横截面必须绕 d 随机滚一个角**。参考写法 `sv = up × d`（up 是世界 +Y）
    //   恒落在**水平面**里 ⇒ 每条带子绕自身法线的滚转角恒为 0 ⇒ **从极轴看下去所有
    //   片子的"正面"朝向完全一样**（用户报的"基本都是正的、没有斜着的"）。
    //   侧视的三种景天然看不出这个偏置，是加了 `sun-pole` 那一景才钉住的。
    //   滚转角用均匀随机：带子自己的初始朝向本来就该是各向同性的
    //   （"同一个噪声空间"管的是**场**，不是每片的初始滚转）。
    sv.copy(Math.abs(d.y) > 0.9 ? ex : up).cross(d).normalize();
    bv.copy(d).cross(sv).normalize();
    const roll = rnd() * Math.PI * 2;
    const cr = Math.cos(roll), sr = Math.sin(roll);
    const sx = sv.x, sy = sv.y, sz = sv.z;
    sv.set(sx * cr + bv.x * sr, sy * cr + bv.y * sr, sz * cr + bv.z * sr).normalize();
    bv.copy(d).cross(sv).normalize();

    // **抽一个形态**，然后这个形态的**全部**参数都从它那套分布里取 ——
    // 高度、宽度、拱度、丝距、丝长、宽度剖面、截面、色温、丝尖参差度。
    // （只抽"大小"是上一版的做法：大小有差别、形态没差别 ⇒ 还是草地。）
    const K = pickKind(rnd());
    // 高度用幂分布：多数偏矮、少数很高（长尾）；幂次按形态给（喷流尖、面纱平）
    const hgt = sunR * lerpLog(K.h, rnd());
    const wid = sunR * lerpLog(K.wid, rnd());
    const arc = lerpR(K.arc, rnd());
    const isArc = arc > 0.001;
    // `aParam.z` 现在只是**弯曲方向的抖动**（不是弯曲量）：方向的主导向量来自共享场
    // （见 prom.vert 的 bendDir），这里只让每片偏一点点 ⇒ 成片但不成复写纸。
    const curve = (rnd() * 2 - 1) * 0.45;
    const seed = rnd();

    dir[i * 3] = d.x; dir[i * 3 + 1] = d.y; dir[i * 3 + 2] = d.z;
    side[i * 3] = sv.x; side[i * 3 + 1] = sv.y; side[i * 3 + 2] = sv.z;
    bend[i * 3] = bv.x; bend[i * 3 + 1] = bv.y; bend[i * 3 + 2] = bv.z;
    par[i * 4] = hgt; par[i * 4 + 1] = wid; par[i * 4 + 2] = curve; par[i * 4 + 3] = seed;
    const tiltJit = rnd();                         // 顺场倾倒的抖动
    kind[i * 3] = arc;
    kind[i * 3 + 1] = tiltJit;
    kind[i * 3 + 2] = 0.70 + 0.70 * rnd();         // 宽度的每片抖动
    // U 形两条腿的分开量（**初始形态**，单位 = 高度）。arc=0 的直须用不到它。
    kind[i * 3 + 3] = K.span[1] > 0 ? lerpR(K.span, rnd()) : 0.0;
    style[i * 4] = lerpR(K.fil, rnd());
    style[i * 4 + 1] = lerpR(K.along, rnd());
    style[i * 4 + 2] = lerpR(K.prof, rnd());
    style[i * 4 + 3] = lerpR(K.heat, rnd());

    // 采样世界空间流场：根部定"朝哪边倒"，上方 0.30R 定"往哪边扭"，
    // 外加"哪里有日珥"的掩码与喷发相位（相位来自场 ⇒ 相邻带子相干）。
    sampleField(field, d, sv, bv, sunR, fs);
    flowA[i * 4] = fs.tilt[0]; flowA[i * 4 + 1] = fs.tilt[1];
    flowA[i * 4 + 2] = fs.mag; flowA[i * 4 + 3] = fs.mask;
    flowB[i * 4] = fs.twist[0]; flowB[i * 4 + 1] = fs.twist[1];
    // 相位 = 场（成片）+ 一点每片的抖动（否则同一片里几十条带子整齐划一，很假）
    flowB[i * 4 + 2] = fs.phase + 0.16 * (rnd() - 0.5);
    flowB[i * 4 + 3] = 0.60 + 1.10 * rnd();                                 // 周期倍率
    // **扭角** = 自转率 `½ω·t̂`（ω=∇×F 是涡量、t̂ 是带子路径切线）沿**路径**的积分。
    // 跟着流走的材料微元，其刚体转动角速度正好是 **ω/2**，横截面绕路径方向的自转只取
    // ω 在 t̂ 上的投影 ⇒ "顶点沿路径位移、截面自然转"，这是"扭"最自然的来源。
    // （另一种口径 α=(F·∇×F)/|F|² 是"旋度沿**场**方向"，即 force-free 参数；
    //   带子的 t̂ 与 F 不是一回事，所以对"沿路径长出来的插片"用路径口径才对。）
    // 带子的**路径方向** —— 必须与 `prom.vert` 里算 `up` 的那三行同源（见 PROM_* 常量）：
    //   tilt = uTilt·(0.25 + 1.5·场强)·(0.55 + 0.90·抖动);  up = normalize(aDir + k0·tilt)
    const tiltAmt = PROM_TILT * (0.25 + 1.5 * fs.mag) * (0.55 + 0.90 * tiltJit);
    upv.copy(d).addScaledVector(k0v, tiltAmt).normalize();
    twist[i * 2] = field.twistAlong(d.x, d.y, d.z, upv.x, upv.y, upv.z, sunR, hgt);
    twist[i * 2 + 1] = 0.55 + 0.90 * rnd();                                 // 每片的扭率抖动
    f0v[i * 3] = fs.f0[0]; f0v[i * 3 + 1] = fs.f0[1]; f0v[i * 3 + 2] = fs.f0[2];
    twist[i * 2 + 2] = lerpR(K.cross, rnd());                               // 截面：面纱 ↔ 一道脊
    twist[i * 2 + 3] = lerpR(K.len, rnd());                                 // 丝尖参差度
  }
  return {
    aDir: new THREE.InstancedBufferAttribute(dir, 3),
    aSide: new THREE.InstancedBufferAttribute(side, 3),
    aBend: new THREE.InstancedBufferAttribute(bend, 3),
    aParam: new THREE.InstancedBufferAttribute(par, 4),
    aKind: new THREE.InstancedBufferAttribute(kind, 4),
    aFlow: new THREE.InstancedBufferAttribute(flowA, 4),
    aFlow2: new THREE.InstancedBufferAttribute(flowB, 4),
    aTwist: new THREE.InstancedBufferAttribute(twist, 4),
    aStyle: new THREE.InstancedBufferAttribute(style, 4),
    aField0: new THREE.InstancedBufferAttribute(f0v, 3),
  };
}

// 一份 LOD 的网格：**实例属性是共享的**（同一个 BufferAttribute 挂到多个 geometry 上，
// 数据只有一份；各 LOD 的差别只在 `position/uv` 这两张每个 LOD 各自的小表）。
function makePromGeo(attrs, ws, hs, sunR) {
  const base = new THREE.PlaneGeometry(1, 1, ws, hs);
  const geo = new THREE.InstancedBufferGeometry();
  geo.index = base.index;
  geo.setAttribute('position', base.attributes.position);
  geo.setAttribute('uv', base.attributes.uv);
  for (const [k, v] of Object.entries(attrs)) geo.setAttribute(k, v);
  geo.instanceCount = PROM_MAX;
  // 插片铺满整个球面，three 从 position 算出来的包围球不含实例属性 ⇒ 关掉视锥剔除，
  // 否则相机一靠近就会被整批剔掉（现象是"日珥时有时无"）。
  geo.boundingSphere = new THREE.Sphere(new THREE.Vector3(0, 0, 0), sunR * 1.5);
  return geo;
}

export function createSun(tier) {
  const g = new THREE.Group();
  const sunPos = new THREE.Vector3();   // 复用的临时量（每帧算 LOD 用，别在循环里 new）

  const R = TUNING.sunRadius;
  const oct = tier.oct;

  const photosphereMat = new THREE.ShaderMaterial({
    // 光球的八度**封顶 4**：uFbmOct=7 时最细的一层是 105*2.13^6 ≈ 9800 周期/弧度
    // ⇒ 屏幕上约 0.1 px，纯属亚像素闪烁（观感是「砂纸」而不是米粒）。4 层的最细一级
    // ~1.5 px，正好落在像素尺度上，米粒才读得出来。日冕是另一个材质，不受影响。
    uniforms: { uFbmOct: fbmOct(Math.min(4, oct)), uTime: { value: 0 }, uIntensity: { value: TUNING.sunIntensity } },
    vertexShader: SUN_VERT,
    fragmentShader: SUN_PHOTO_FRAG,
  });
  const photosphere = new THREE.Mesh(new THREE.SphereGeometry(R, tier.seg[0], tier.seg[1]), photosphereMat);
  g.add(photosphere);

  // 日冕：**世界坐标的体积**（射线步进），不是 billboard。见 CORONA_FRAG 顶部的推导。
  const coronaMat = CORONA_MAT(R, tier);
  const corona = new THREE.Mesh(
    new THREE.SphereGeometry(R * CORONA_OUTER_RATIO, 48, 32), coronaMat);
  corona.renderOrder = 5;
  g.add(corona);

  // 日珥插片：**自己一套材质与几何**（世界空间、真遮挡、真视差）
  // **世界空间流场纹理**（顶点着色器按"每个顶点自己的位置"取它 ⇒ 弯/扭/剪切自然产生）。
  // ⚠ 它是**加载时算出来的推导产物**，不进版本库（用户裁决）。详见 sunfield.js::bakeField。
  const field = new PromField();
  const ft = field.bakeField(R);
  const fieldTex = new THREE.DataTexture(ft.data, ft.width, ft.height, THREE.RGBAFormat, THREE.FloatType);
  // NearestFilter 是**必须**的：图集里相邻格子是**不同深度的切片**，线性过滤会在切片间渗色
  // ⇒ 顶点着色器里手写三线性（8 次取纹理）。
  fieldTex.minFilter = THREE.NearestFilter;
  fieldTex.magFilter = THREE.NearestFilter;
  fieldTex.wrapS = THREE.ClampToEdgeWrapping;
  fieldTex.wrapT = THREE.ClampToEdgeWrapping;
  fieldTex.needsUpdate = true;

  const promMat = new THREE.ShaderMaterial({
    uniforms: {
      uSunR: { value: R },
      uTime: { value: 0 },
      uGrow: { value: 1 },
      uIntensity: { value: 0.85 },
      // 顶点和片元都要采噪声（顶点定"朝哪边倒"+上缘起伏、片元切"丝"），所以两件都挂。
      // ⚠ 每一片要采 ~7 次 `fbmP`（顶点）× 2 次 `ridgedP`（片元），**八度数是这里最贵的旋钮**：
      //   实测（RTX，特写视角，5000 片）oct 3→2 省 **~4 ms**，少一次 `ridgedP` 省 ~2.5 ms。
      //   旧的 `min(3, oct)` 对 oct≥3 的档**恒等于 3** ⇒ 等于没分档（medium/low 白花钱）。
      //   现在是 `oct-2` 且上下夹住：ultra 3、high 3、medium 2、low 1。
      uFbmOct: fbmOct(Math.max(1, Math.min(3, oct - 2))),
      // 顺场倾倒的总幅度（弧度尺度）：**"朝向不再一样"就是靠它**
      uTilt: { value: PROM_TILT },
      // 顺场扭的幅度（中轴沿长度往场的方向歪出去多少）
      // 「弯」的幅度：∇×F 的**垂直**分量掰弯场线（顺场扫出去）
      uSweep: { value: PROM_SWEEP },
      // **显式积分的那份扭**（`½∫ω·t̂ dl` 烘在 aTwist.x）。现在**关掉**：
      // 扭改由"每个顶点自己采样世界空间流场"自然产生（见 uDisp 与 prom.vert）。
      // 留着这个旋钮是为了随时能 A/B 回显式版本。
      uTwist: { value: 0.0 },
      // **顶点驱动的位移幅度**（世界单位）：每个顶点沿**自己位置上**的场位移这么多。
      // 为什么这就是"扭"：扭转是个**微分量** —— 左右两缘取到的场不同 ⇒ 截面转。
      // 定量（实测本仓的场）：转角 ≈ ½·|ω|·amp，amp=0.30 ⇒ **0.84 rad**，
      // 与手调出来的 aTwist.x×uTwist 中位 0.82 几乎一样；而且顺带得到**剪切**。
      uDisp: { value: TUNING.promDisp != null ? TUNING.promDisp : 0.30 },
      // 场纹理的取样参数（图集布局必须与 sunfield.js::bakeField 逐项一致）
      uFieldTex: { value: fieldTex },
      uFieldAtlas: { value: new THREE.Vector2(ft.width, ft.height) },
      uFieldGrid: { value: new THREE.Vector2(ft.tilesX, ft.tile) },
      uFieldN: { value: ft.n },
      uFieldHalf: { value: ft.half },
      // 喷发周期的基准（秒）；每条带子按自己的 `aFlow2.w` 在 0.6~1.7× 之间取
      uPeriod: { value: TUNING.promPeriod || 34 },
      // 带子**内部**纹理的频率。⚠ 纹理是**本地空间**的（沿带宽/带长的 uv），
      // 频率只用来把 uv 换算成世界尺度（`vSize`）—— 这样窄带子和宽带子上的丝一样粗。
      // 26 ⇒ 丝的间距 ≈ 0.038 世界单位 ≈ 0.006R ⇒ 一条带子里并排 **7~26 根丝**。
      // `uFilAlong` 是顺带长方向的频率比：0.14 ⇒ 丝被拉长 ~7 倍（是**长**丝，不是一粒粒）。
      uFilFreq: { value: 26.0 },
      uFilAlong: { value: 0.14 },
      // 304Å 的日珥：根部亮橙、尖端深红（对照参考图）
      uColorHot: { value: new THREE.Vector3(2.00, 0.66, 0.09) },
      uColorCool: { value: new THREE.Vector3(1.20, 0.20, 0.020) },
    },
    vertexShader: PROM_VERT,
    fragmentShader: PROM_FRAG,
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,     // 发光体：不写深度（互相叠加），但要**读**深度 ⇒ 会被行星挡住
    depthTest: true,
    side: THREE.DoubleSide,
  });
  // 三个 LOD 各一份网格（实例属性共享），按档位换 —— 换的是 `prom.geometry`，
  // **不触发着色器重编译**（材质没变），所以换档不会有卡顿。
  const promAttrs = buildPromAttrs(R, field);
  const promGeos = PROM_LOD.map(([ws, hs]) => makePromGeo(promAttrs, ws, hs, R));
  const prom = new THREE.Mesh(promGeos[0], promMat);
  prom.frustumCulled = false;
  prom.renderOrder = 6;    // 在日冕之后
  g.add(prom);

  const parts = [photosphere, corona];
  // 当前生效的 LOD 等级（档位给上限，屏幕尺寸再往下压，见 update 里的 applyLod）。
  let lodTier = 0;
  let lodNow = -1;
  const applyLod = (want) => {
    const lod = Math.max(0, Math.min(promGeos.length - 1, want));
    if (lod === lodNow) return;
    lodNow = lod;
    if (prom.geometry !== promGeos[lod]) prom.geometry = promGeos[lod];
  };
  const setTier = (t) => {
    // 八度数现在是 **uniform**（见 util.js 里 NOISE_GLSL 那段），所以换档只改一个数值、
    // 不再触发 `needsUpdate` 重编译。步数同理。
    const oct = Math.max(1, t.oct);
    parts.forEach((m) => { if (m.material.uniforms.uFbmOct) m.material.uniforms.uFbmOct.value = oct; });
    coronaMat.uniforms.uSteps.value = Math.max(6, Math.min(18, 4 + oct * 2));
    corona.visible = !!t.corona && TUNING.coronaOn !== 0;
    // 日珥插片跟着同一个开关（它们和色球是同一层东西），数量随档位缩 ——
    // 只改 `instanceCount`，几何一份、不重建（换档不该有卡顿）。
    prom.visible = !!t.corona && TUNING.coronaOn !== 0;
    // **LOD（档位这一维）**：段数上限走 `TUNING.TIERS[].promLod`，数量走 `instanceCount`。
    // 两个维度分开降是有意的：段数降的是**几何平滑度**，数量降的是**密度**，
    // 而质感（带子内部那些细丝）在片元里，两边都不动它。
    lodTier = Math.max(0, Math.min(promGeos.length - 1, t.promLod | 0));
    lodNow = -1;                       // 逼 applyLod 下一帧重新选
    promGeos.forEach((gg, k) => { gg.instanceCount = prom.visible ? (t.prom || 0) : 0; });
  };
  setTier(tier);

  return {
    group: g,
    photosphere,
    update(t, camera, depth) {
      // **LOD（屏幕尺寸这一维）**：日珥插片的顶点开销 = 实例数 × 每片段数，
      // 而每片段数只在"带子在屏幕上够大"时才有意义 —— 远景（`sun-wide`，日面才几十像素）
      // 用顶档纯属浪费。实测（RTX，1280×800，ultra 5000 片）：[9,24]≈1.25M 顶点 ⇒ 8.8~10.2 ms，
      // [5,10]≈0.33M ⇒ 6.1 ms ⇒ **顶档几何要 ~3.5 ms**，所以这一维值得按距离降。
      // 判据是**日面在屏幕上的半径**（像素），阈值取得很宽：只有明显变小才降，
      // 免得贴近/拉远时来回跳（`applyLod` 也只在等级真的变了才换 geometry）。
      if (camera) {
        const h = (depth && depth.height) || (typeof window !== 'undefined' ? window.innerHeight : 800);
        // ⚠ 用**太阳到相机**的距离，不是"相机到原点"：两者只是在太阳恰好位于世界原点时
        //   才相等（现在相等），但那是**巧合级**的依赖 —— 天体一动就悄悄错。
        //   取太阳组的**世界坐标**来算，语义才对得上"LOD 按物体到相机的距离决定"。
        g.getWorldPosition(sunPos);
        const dist = Math.max(1e-3, camera.position.distanceTo(sunPos));
        // proj[5] = 1/tan(fov/2) ⇒ 日面在屏幕上的半径（像素）
        const screenR = (R / dist) * camera.projectionMatrix.elements[5] * 0.5 * h;
        const bySize = screenR < 45 ? 2 : (screenR < 130 ? 1 : 0);
        applyLod(Math.max(lodTier, bySize));   // 取更粗的那个
      }
      photosphereMat.uniforms.uTime.value = t;
      coronaMat.uniforms.uTime.value = t;
      promMat.uniforms.uTime.value = t;
      // 全局呼吸只留一点点：**主运动已经交给每片自己的喷发周期**（见 prom.vert），
      // 这里再加一个大振幅的全局呼吸会和它打架（整片日缘一起涨落，很假）。
      promMat.uniforms.uGrow.value = 1.0 + 0.05 * Math.sin(t * 0.23);
      // 相机矩阵会变（变焦/改 FOV），所以每帧同步，不能只在创建时设一次
      if (camera) coronaMat.uniforms.uProj.value.copy(camera.projectionMatrix);
      // 深度预趟产物 + 相机参数：体积积分靠它们夹断（每帧都要更新，窗口会变）
      if (depth) {
        // 用**深度纹理**（24 位），而不是 RT 的颜色纹理（8 位，会分层）
        coronaMat.uniforms.uDepth.value = depth.depthTexture || depth.texture;
        coronaMat.uniforms.uResolution.value.set(depth.width, depth.height);
      }
      coronaMat.uniforms.uHasDepth.value = depth ? 1 : 0;
      if (camera) coronaMat.uniforms.uNearFar.value.set(camera.near, camera.far);
    },
    setIntensity(v) {
      photosphereMat.uniforms.uIntensity.value = v;
      // 日冕的强度是它自己的（跟光球强度不是一回事）：光球强度是 HDR 亮度，
      // 日冕是**沿视线积分后的累加值**，两者差一个 dt 量纲。别把它们写成一个数。
    },
    setTier,
    // 当前生效的日珥 LOD 等级（0=最细）。给 `debug()` 用：LOD 是"看不见的降级"，
    // 没有这个数就只能靠帧时间反推，判据里也没法断言"远景真的降了"。
    promLod() { return lodNow; },
    dispose() {
      g.traverse((o) => { if (o.geometry) o.geometry.dispose(); if (o.material) o.material.dispose(); });
    },
  };
}
