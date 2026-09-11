// 行星X WebUI — 太阳。
//
// 旧版是一个 fbm 斑块球 + 一张 canvas 径向渐变 sprite，两个问题：表面像**奶酪**（八度不够、
// 尺度太大，在球面上抽出大块斑驳），边缘是一条硬切口（没有 limb darkening，也没有色球/日冕），
// 所以整颗太阳读起来是「一个橙色贴纸」而不是一颗恒星。
//
// 现在三层叠加，从内到外：
//   ① 光球 photosphere —— 米粒组织(ridged 反相 = 亮米粒+暗沟) + 超米粒流 + 太阳黑子(本影/半影)
//      + **limb darkening**（临边昏暗：I(μ)=1-u1(1-μ)-u2(1-μ)²，同时压亮度并偏红）
//   ② 色球 / 日珥 —— **并入日冕的体积**（见 corona.frag 的 ④ 项）。它原先是一层等半径的
//      球壳（`R*1.012`），而**壳在几何上没有径向厚度**：外缘永远等于轮廓线，于是只能读成
//      「一圈均匀硬红环」，怎么调噪声都救不回来（用户：「太阳的大气太耿直了，弄得像日珥
//      一点」）。日珥的本质是径向**长短不一**的针与弧 —— 那需要体积，不需要一层壳。
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
// 现在的太阳**只有两个网格**：光球球体 + 日冕球体（半径 `R*CORONA_OUTER_RATIO`）。
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
    uIntensity: { value: 0.055 },
    // 色球强度是**另一个量纲**（比日冕亮约 1e4 倍），别和 uIntensity 混为一谈
    uChromo: { value: 5.0 },
    // 片元里拿不到 projectionMatrix（three 只在顶点前缀给），自己传
    uProj: { value: new THREE.Matrix4() },
    // 场景深度：体积积分必须在**最近的实体表面**处停下（见 CORONA_FRAG 里的夹断），
    // 否则遮挡物**背后**的介质也会被累加进来。这张纹理来自 postfx 的深度预趟。
    uDepth: { value: null },
    uResolution: { value: new THREE.Vector2(1, 1) },
    uNearFar: { value: new THREE.Vector2(0.1, 1000) },
    uHasDepth: { value: 0 },
    // 这个指数决定「日冕能伸多远」。两个极端都试过，都用错了：
    //   · **2.6**（原来）：2.5 R 处仍有 rr^-2.6 = 9% ⇒ 一路糊到行星轨道上。
    //   · **9.0**（我改的）：rr=2 处就低于 2e-3 的早退阈值 ⇒ **日冕本身被裁掉了**
    //     （用户：「你把日晕给删了」）。为消一层雾而把主体删掉，是我搞反了因果 ——
    //     那层雾的两个真因（色球漏夹断、相机在体积内深度测试失效）都已经单独修好了。
    //   · **6.0**（现在）：1.5 R ⇒ 0.13，2.5 R ⇒ 6.4e-3（行星轨道处只剩 0.6%，雾基本不可见），
    //     而 3 R 处仍有 1.4e-3 —— 日冕保持可见、有体量，又不至于罩住行星。
    uFalloff: { value: 2.6 },
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

  // 日冕：**世界坐标的体积**（射线步进），不是 billboard。见 CORONA_FRAG 顶部的推导。
  const coronaMat = CORONA_MAT(R, tier);
  const corona = new THREE.Mesh(
    new THREE.SphereGeometry(R * CORONA_OUTER_RATIO, 48, 32), coronaMat);
  corona.renderOrder = 5;
  g.add(corona);

  const parts = [photosphere, corona];
  const setTier = (t) => {
    // 八度数现在是 **uniform**（见 util.js 里 NOISE_GLSL 那段），所以换档只改一个数值、
    // 不再触发 `needsUpdate` 重编译。步数同理。
    const oct = Math.max(1, t.oct);
    parts.forEach((m) => { if (m.material.uniforms.uFbmOct) m.material.uniforms.uFbmOct.value = oct; });
    coronaMat.uniforms.uSteps.value = Math.max(6, Math.min(18, 4 + oct * 2));
    corona.visible = !!t.corona;   // 色球/日珥现在住在日冕体积里，跟着同一个开关
  };
  setTier(tier);

  return {
    group: g,
    photosphere,
    update(t, camera, depth) {
      photosphereMat.uniforms.uTime.value = t;
      coronaMat.uniforms.uTime.value = t;
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
