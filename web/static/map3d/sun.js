// 行星X WebUI — 太阳。
//
// 旧版是一个 fbm 斑块球 + 一张 canvas 径向渐变 sprite，两个问题：表面像**奶酪**（八度不够、
// 尺度太大，在球面上抽出大块斑驳），边缘是一条硬切口（没有 limb darkening，也没有色球/日冕），
// 所以整颗太阳读起来是「一个橙色贴纸」而不是一颗恒星。
//
// 现在三层叠加，从内到外：
//   ① 光球 photosphere —— 米粒组织(ridged 反相 = 亮米粒+暗沟) + 超米粒流 + 太阳黑子(本影/半影)
//      + **limb darkening**（临边昏暗：I(μ)=1-u1(1-μ)-u2(1-μ)²，同时压亮度并偏红）
//   ② 色球 chromosphere —— 极薄的自发光壳，只在 μ 很小（临边）处出现，深红
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
const SUN_PHOTO_FRAG = INC('px/sun/sun-photo.frag');

// --- ② 色球（薄壳，只在临边出现）---------------------------------------------
const CHROMO_FRAG = INC('px/sun/chromo.frag');

// --- ③ 日冕 / 日珥 / ④ 光晕：一张 billboard，在顶点里手动做面向相机 -------------
// 为什么用 billboard 而不是「大一号的球」：日冕是**光学薄**的发射体，看到的是沿视线积分的
// 结果，视觉上就是一团以太阳为中心、向外辐射的辉光。球壳做不出「延伸到很远处的冕流」，
// 而 billboard 上一段 2D 极坐标噪声就能做出来，而且永远正对镜头、没有背面剔除问题。
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
    uIntensity: { value: 0.055 },
    uFalloff: { value: 2.6 },
    // 步数随档位走：这是每像素最贵的一项，弱机必须能降下来。
    uSteps: { value: Math.max(6, Math.min(18, 4 + tier.oct * 2)) },
  },
  vertexShader: CORONA_VERT,
  fragmentShader: CORONA_FRAG,
  transparent: true,
  blending: THREE.AdditiveBlending,
  depthWrite: false,
  depthTest: true,
  // **背面**：相机在体积外时渲染远表面、在体积内时渲染的还是远表面 —— 两种情况都有片元，
  // 不用按位置切换 side（切 side 会改 define ⇒ 触发重编译）。
  side: THREE.BackSide,
});

export function createSun(tier) {
  const g = new THREE.Group();

  const R = TUNING.sunRadius;
  const oct = tier.oct;

  const photosphereMat = new THREE.ShaderMaterial({
    uniforms: { uFbmOct: fbmOct(oct), uTime: { value: 0 }, uIntensity: { value: TUNING.sunIntensity } },
    vertexShader: SUN_VERT,
    fragmentShader: SUN_PHOTO_FRAG,
  });
  const photosphere = new THREE.Mesh(new THREE.SphereGeometry(R, tier.seg[0], tier.seg[1]), photosphereMat);
  g.add(photosphere);

  const chromoMat = new THREE.ShaderMaterial({
    uniforms: { uFbmOct: fbmOct(Math.max(2, oct - 1)), uTime: { value: 0 } },
    vertexShader: SUN_VERT,
    fragmentShader: CHROMO_FRAG,
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    side: THREE.FrontSide,
  });
  const chromosphere = new THREE.Mesh(new THREE.SphereGeometry(R * 1.012, tier.seg[0], tier.seg[1]), chromoMat);
  g.add(chromosphere);

  // 日冕：**世界坐标的体积**（射线步进），不是 billboard。见 CORONA_FRAG 顶部的推导。
  const coronaMat = CORONA_MAT(R, tier);
  const corona = new THREE.Mesh(
    new THREE.SphereGeometry(R * CORONA_OUTER_RATIO, 48, 32), coronaMat);
  corona.renderOrder = 5;
  g.add(corona);

  const parts = [photosphere, chromosphere, corona];
  const setTier = (t) => {
    // 八度数现在是 **uniform**（见 util.js 里 NOISE_GLSL 那段），所以换档只改一个数值、
    // 不再触发 `needsUpdate` 重编译。步数同理。
    const oct = Math.max(1, t.oct);
    parts.forEach((m) => { if (m.material.uniforms.uFbmOct) m.material.uniforms.uFbmOct.value = oct; });
    coronaMat.uniforms.uSteps.value = Math.max(6, Math.min(18, 4 + oct * 2));
    corona.visible = !!t.corona;
    chromosphere.visible = !!t.corona;
  };
  setTier(tier);

  return {
    group: g,
    photosphere,
    update(t) {
      photosphereMat.uniforms.uTime.value = t;
      chromoMat.uniforms.uTime.value = t;
      coronaMat.uniforms.uTime.value = t;
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
