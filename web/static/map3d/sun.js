// 行星X WebUI — 太阳。
//
// 旧版是一个 fbm 斑块球 + 一张 canvas 径向渐变 sprite，两个问题：表面像**奶酪**（八度不够、
// 尺度太大，在球面上抽出大块斑驳），边缘是一条硬切口（没有 limb darkening，也没有色球/日冕），
// 所以整颗太阳读起来是「一个橙色贴纸」而不是一颗恒星。
//
// 现在三层叠加，从内到外：
//   ① 光球 photosphere —— 米粒组织(ridged 反相 = 亮米粒+暗沟) + 超米粒流 + 太阳黑子(本影/半影)
//      + **limb darkening**（临边昏暗：I(μ)=1-u1(1-μ)-u2(1-μ)²，同时压亮度并偏红）
//   ② 色球 / 日珥 —— **世界坐标的插片几何**（`prom.vert/frag`，见下面 buildPromCards）。
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

// --- ②b 日珥插片 --------------------------------------------------------------
// **世界空间的曲面片**：每一片是一条细长的带子，沿长度方向按 `aBend` 弯出去。
// 为什么是几何而不是在片元里算（三轮试错的结论，见 prom.vert 顶部与那条笔记）：
//   · 体积积分：掠射视线穿过三十多根针 ⇒ **平均成雾**（这是"怎么调都没须"的真正原因）
//   · 每像素解析求交：形状能出来，但是 2.5D，没有真遮挡/真视差
//   · 插片：真三维、真遮挡、剪影天然正确，代价只有几千个 12 顶点的小网格
const PROM_VERT = INC('px/sun/prom.vert');
const PROM_FRAG = INC('px/sun/prom.frag');

// 上限（按档位只改 `instanceCount`，不重建几何）。日珥只在**日缘一圈**可见，
// 均匀铺满球面时大约只有 1/10 落在可见的日缘带上，所以总量要给得足。
const PROM_MAX = 12000;

// 确定性 PRNG（截图要可复现；**不要**用 Math.random）
function mulberry32(a) {
  return function () {
    a |= 0; a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// 单位插片：宽 1、长 1，沿长度 4 段（段数决定"曲面片"弯得顺不顺）。
// ⚠ uv.y = 0 是**根部**、1 是**尖端**（three 的 PlaneGeometry 就是这样）。
function buildPromCards(sunR) {
  const base = new THREE.PlaneGeometry(1, 1, 1, 4);
  const geo = new THREE.InstancedBufferGeometry();
  geo.index = base.index;
  geo.setAttribute('position', base.attributes.position);
  geo.setAttribute('uv', base.attributes.uv);
  geo.instanceCount = PROM_MAX;

  const rnd = mulberry32(0x5eed1234);
  const dir = new Float32Array(PROM_MAX * 3);
  const side = new Float32Array(PROM_MAX * 3);
  const bend = new Float32Array(PROM_MAX * 3);
  const par = new Float32Array(PROM_MAX * 4);
  const d = new THREE.Vector3(), sv = new THREE.Vector3(), bv = new THREE.Vector3();
  const up = new THREE.Vector3(0, 1, 0), ex = new THREE.Vector3(1, 0, 0);

  for (let i = 0; i < PROM_MAX; i++) {
    // 球面均匀
    const z = rnd() * 2 - 1;
    const phi = rnd() * Math.PI * 2;
    const r = Math.sqrt(Math.max(0, 1 - z * z));
    d.set(r * Math.cos(phi), z, r * Math.sin(phi));
    // 让针**朝各个方向斜**（不要全是笔直的法线方向）：把方向轻轻推一下
    sv.copy(Math.abs(d.y) > 0.9 ? ex : up).cross(d).normalize();
    d.addScaledVector(sv, (rnd() * 2 - 1) * 0.30).normalize();
    bv.copy(d).cross(sv).normalize();

    // 长度：**长尾**（多数是短须、少数窜得很高）；宽度：细（世界单位）
    const len = sunR * (0.020 + 0.190 * Math.pow(rnd(), 2.3));
    const wid = sunR * (0.0035 + 0.0095 * rnd());
    const curve = (rnd() * 2 - 1) * 0.55;
    const seed = rnd();

    dir[i * 3] = d.x; dir[i * 3 + 1] = d.y; dir[i * 3 + 2] = d.z;
    side[i * 3] = sv.x; side[i * 3 + 1] = sv.y; side[i * 3 + 2] = sv.z;
    bend[i * 3] = bv.x; bend[i * 3 + 1] = bv.y; bend[i * 3 + 2] = bv.z;
    par[i * 4] = len; par[i * 4 + 1] = wid; par[i * 4 + 2] = curve; par[i * 4 + 3] = seed;
  }
  geo.setAttribute('aDir', new THREE.InstancedBufferAttribute(dir, 3));
  geo.setAttribute('aSide', new THREE.InstancedBufferAttribute(side, 3));
  geo.setAttribute('aBend', new THREE.InstancedBufferAttribute(bend, 3));
  geo.setAttribute('aParam', new THREE.InstancedBufferAttribute(par, 4));
  // 插片铺满整个球面，three 从 position 算出来的包围球不含实例属性 ⇒ 关掉视锥剔除，
  // 否则相机一靠近就会被整批剔掉（现象是"日珥时有时无"）。
  geo.boundingSphere = new THREE.Sphere(new THREE.Vector3(0, 0, 0), sunR * 1.5);
  return geo;
}

export function createSun(tier) {
  const g = new THREE.Group();

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
  const promMat = new THREE.ShaderMaterial({
    uniforms: {
      uSunR: { value: R },
      uTime: { value: 0 },
      uGrow: { value: 1 },
      uIntensity: { value: 1.45 },
      // 304Å 的日珥：根部亮橙、尖端深红（对照参考图）
      uColorHot: { value: new THREE.Vector3(1.35, 0.42, 0.10) },
      uColorCool: { value: new THREE.Vector3(0.85, 0.10, 0.02) },
    },
    vertexShader: PROM_VERT,
    fragmentShader: PROM_FRAG,
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,     // 发光体：不写深度（互相叠加），但要**读**深度 ⇒ 会被行星挡住
    depthTest: true,
    side: THREE.DoubleSide,
  });
  const promGeo = buildPromCards(R);
  const prom = new THREE.Mesh(promGeo, promMat);
  prom.frustumCulled = false;
  prom.renderOrder = 6;    // 在日冕之后
  g.add(prom);

  const parts = [photosphere, corona];
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
    promGeo.instanceCount = prom.visible ? (t.prom || 0) : 0;
  };
  setTier(tier);

  return {
    group: g,
    photosphere,
    update(t, camera, depth) {
      photosphereMat.uniforms.uTime.value = t;
      coronaMat.uniforms.uTime.value = t;
      promMat.uniforms.uTime.value = t;
      // 「喷发」感：整体高度做一次很慢的呼吸（振幅小，别让它读成"整片在缩放"）
      promMat.uniforms.uGrow.value = 1.0 + 0.16 * Math.sin(t * 0.23);
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
    dispose() {
      g.traverse((o) => { if (o.geometry) o.geometry.dispose(); if (o.material) o.material.dispose(); });
    },
  };
}
