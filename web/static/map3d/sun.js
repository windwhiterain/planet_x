// 行星X WebUI — 太阳。
//
// 旧版是一个 fbm 斑块球 + 一张 canvas 径向渐变 sprite，两个问题：表面像**奶酪**（八度不够、
// 尺度太大，在球面上抽出大块斑驳），边缘是一条硬切口（没有 limb darkening，也没有色球/日冕），
// 所以整颗太阳读起来是「一个橙色贴纸」而不是一颗恒星。
//
// 现在四层叠加，从内到外：
//   ① 光球 photosphere —— 米粒组织(ridged 反相 = 亮米粒+暗沟) + 超米粒流 + 太阳黑子(本影/半影)
//      + **limb darkening**（临边昏暗：I(μ)=1-u1(1-μ)-u2(1-μ)²，同时压亮度并偏红）
//   ② 色球 chromosphere —— 极薄的自发光壳，只在 μ 很小（临边）处出现，深红
//   ③ 日冕 corona —— **billboard** 上的程序化径向流苏（针状体/冕流）+ 临边日珥弧
//   ④ 光晕 halo —— 一层很淡的大范围 additive 辉光，负责在远景/缩小后「太阳还在发光」
//
// 全部 HDR（光球输出 ~9、日冕 ~1），超出 1 的部分交给 UnrealBloom 变成辉光——真实感来自
// 「亮的东西真的比白更亮」，而不是在球外画一圈半透明橙色。

import * as THREE from 'three';
import { NOISE_GLSL, fbmOct } from './util.js';
import { TUNING } from './tuning.js';

const SUN_VERT = /* glsl */`
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;
  void main(){
    vObjPos = position;
    // 世界空间法线：normalMatrix 是「对象→视图」空间的，而光照方向是世界空间的，
    // 两者混用会让 dot(n, lightDir) 失去意义（旧版踩过的坑，见 notes/webui-3d-rendering）。
    vNormal = normalize(mat3(modelMatrix) * normal);
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
`;

// --- ① 光球 -----------------------------------------------------------------
const SUN_PHOTO_FRAG = /* glsl */`
  precision highp float;
  ${NOISE_GLSL}
  uniform float uTime;
  uniform float uIntensity;
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;

  void main(){
    vec3 p = normalize(vObjPos);
    vec3 n = normalize(vNormal);
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float mu = clamp(dot(n, viewDir), 0.0, 1.0);

    // 米粒组织：两套不同尺度/相位对流的噪声交叉淡入淡出，做出「沸腾」而不是「飘动」。
    float t = uTime;
    float ph = 0.5 + 0.5 * sin(t * 0.55);
    float SC = 95.0;                       // 米粒的基准空间频率（见上，26 时像海绵）
    float g1 = 1.0 - ridged(p * SC + vec3(0.0, 0.0, t * 0.35));
    float g2 = 1.0 - ridged(p * SC + vec3(37.0, 11.0, t * 0.35 + 41.0));
    float gran = mix(g1, g2, ph);
    // 超米粒：更大尺度的对流胞，给米粒组织一个「群」的结构（约 10 倍于米粒）。
    float superG = fbm(p * 9.0 + vec3(0.0, 0.0, t * 0.06));
    // 磁网络：米粒边界上的亮环（真实太阳临边附近的 faculae）。
    float network = smoothstep(0.55, 0.95, ridged(p * SC + vec3(0.0, 0.0, t * 0.35)));

    float heat = gran * 0.62 + superG * 0.38;
    heat = heat * 0.82 + network * 0.22;
    // 对比度重映射：米粒组织的自然动态范围只有 ±20%，在 ACES 之后几乎看不出结构。
    // 以 0.50 为中心拉开 1.9 倍，让颗粒/暗沟真的读得出来。
    heat = clamp((heat - 0.50) * 1.55 + 0.50, 0.0, 1.4);

    // 太阳黑子：大尺度的暗区，本影更黑、半影有丝状结构。
    float spotField = fbm(p * 2.6 + vec3(5.0, 9.0, t * 0.02));
    float penumbra = smoothstep(0.60, 0.74, spotField);
    float umbra = smoothstep(0.70, 0.80, spotField);
    float spot = penumbra * 0.55 + umbra * 0.65;
    // 黑子只在低纬带出现（真实黑子集中在 ±5°..±30°），别让极区也长斑。
    float latBand = exp(-pow(p.y / 0.55, 2.0));
    spot *= mix(0.25, 1.0, latBand);
    heat *= (1.0 - spot * 0.72);

    // 色带：冷 → 暖 → 白热。
    vec3 cool = vec3(0.95, 0.42, 0.10);
    vec3 mid  = vec3(1.00, 0.74, 0.36);
    vec3 hot  = vec3(1.00, 0.96, 0.88);
    vec3 col = mix(cool, mid, smoothstep(0.28, 0.66, heat));
    col = mix(col, hot, smoothstep(0.62, 1.02, heat));

    // 临边昏暗：太阳最重要的「是颗球」的证据。亮度按 I(μ) 压，颜色同时偏红。
    float limb = 1.0 - 0.62 * (1.0 - mu) - 0.18 * (1.0 - mu) * (1.0 - mu);
    col *= clamp(limb, 0.06, 1.0);
    col = mix(col, col * vec3(1.25, 0.55, 0.20), pow(1.0 - mu, 3.0) * 0.85);
    // 黑子区域再压一点蓝，读起来是「暗红的斑」而不是「灰色的洞」。
    col = mix(col, col * vec3(1.15, 0.6, 0.35), spot * 0.5);

    gl_FragColor = vec4(col * uIntensity, 1.0);
  }
`;

// --- ② 色球（薄壳，只在临边出现）---------------------------------------------
const CHROMO_FRAG = /* glsl */`
  precision highp float;
  ${NOISE_GLSL}
  uniform float uTime;
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;
  void main(){
    vec3 p = normalize(vObjPos);
    vec3 n = normalize(vNormal);
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float mu = abs(dot(n, viewDir));
    // 针状体（spicules）：沿径向拉长的细丝，随时间抖动。
    float spic = ridged(p * vec3(38.0, 38.0, 9.0) + vec3(0.0, 0.0, uTime * 0.9));
    float rim = pow(1.0 - mu, 3.4);
    vec3 col = vec3(1.35, 0.30, 0.14) * rim * (0.35 + 0.9 * spic);
    col += vec3(1.6, 0.55, 0.22) * pow(1.0 - mu, 7.0) * 0.7;
    gl_FragColor = vec4(col, 1.0);
  }
`;

// --- ③ 日冕 / 日珥 / ④ 光晕：一张 billboard，在顶点里手动做面向相机 -------------
// 为什么用 billboard 而不是「大一号的球」：日冕是**光学薄**的发射体，看到的是沿视线积分的
// 结果，视觉上就是一团以太阳为中心、向外辐射的辉光。球壳做不出「延伸到很远处的冕流」，
// 而 billboard 上一段 2D 极坐标噪声就能做出来，而且永远正对镜头、没有背面剔除问题。
const CORONA_VERT = /* glsl */`
  uniform float uSize;
  varying vec2 vP;
  void main(){
    vP = position.xy * 2.0;                 // -1..1 的盘面坐标
    // 手写 billboard：把平面的局部 xy 直接加到「太阳中心在视图空间的位置」上。
    // 这样永远正对镜头（日冕是光学薄发射体，这就是对的近似），也不吃背面剔除。
    // 半边长 = uSize ⇒ vP=1 处正好离日心 uSize，于是 uCore 可以直接写成 R/uSize。
    vec4 mv = modelViewMatrix * vec4(0.0, 0.0, 0.0, 1.0);
    mv.xy += position.xy * uSize * 2.0;
    gl_Position = projectionMatrix * mv;
  }
`;

const CORONA_FRAG = /* glsl */`
  precision highp float;
  ${NOISE_GLSL}
  uniform float uTime;
  uniform float uSize;
  uniform float uIntensity;
  uniform float uCore;      // 被光球盖住的半径（以盘面坐标为单位）
  uniform float uFalloff;   // 径向幂律指数
  varying vec2 vP;

  void main(){
    float r = length(vP);
    if (r < uCore * 0.92) discard;          // 被光球挡住的区域直接丢，省填充率

    // 先算**便宜**的径向包络，再决定要不要跑昂贵的噪声。
    // 这一条很关键：日冕 billboard 的半径是 4 个太阳半径，相机贴近行星时它会铺满整个
    // 屏幕——不早退的话，每帧要对全屏跑一次 fbm+ridged（集显上直接吃掉一半帧时间）。
    float rc0 = max(uCore, 0.04);
    float fall = pow(rc0 / max(r, rc0), uFalloff) * smoothstep(1.0, 0.72, r);
    if (fall * uIntensity < 0.0025) discard;

    float a = atan(vP.y, vP.x);
    float t = uTime;

    // 冕流：沿角向拉长的噪声，在径向缓慢流动。极坐标 → 三维噪声坐标（用 (cos,sin,r) 保角向周期）。
    vec3 q = vec3(cos(a), sin(a), 0.0) * 3.2 + vec3(0.0, 0.0, r * 1.6 - t * 0.10);
    float streamer = fbm(q);
    float fine = ridged(vec3(cos(a), sin(a), 0.0) * 9.0 + vec3(0.0, 0.0, r * 3.0 - t * 0.22));
    // 径向衰减在 main() 开头已经算好（fall）。**必须是幂律**（K-日冕在天空平面上的
    // 投影大致 ~r^-2.6），不能用 pow(1-r,k) 那种「到边缘才归零」的浅包络——后者在相机
    // 贴近太阳时会让整个画面蒙上一层灰。
    // 冕流要有**对比**：有底噪的日冕只是一层灰雾。基底压到 0.02，让「流苏之间」真的接近
    // 全黑——真实日冕照片里最抓人的就是那些放射状亮条之间的暗。
    float shape = 0.02 + 2.30 * pow(smoothstep(0.22, 0.80, streamer), 1.8) + 0.30 * fine;

    // 日珥：紧贴光球的一圈弧状等离子体，随时间涨落。
    float band = exp(-pow((r - uCore * 1.14) / (uCore * 0.16), 2.0));
    float prom = smoothstep(0.45, 0.95, fbm(vec3(cos(a), sin(a), 0.0) * 5.5 + vec3(0.0, 0.0, t * 0.13)));
    vec3 promCol = vec3(1.5, 0.30, 0.18);

    // 日冕的色：内圈偏白黄（自由电子散射的近白），外圈偏冷的淡蓝紫（F-corona / 尘埃散射）。
    vec3 inner = vec3(1.15, 0.90, 0.62);
    vec3 outer = vec3(0.42, 0.52, 0.95);
#ifdef CORONA_SIMPLE
    // 远处的弥散光晕只需要一条幂律：跳过全部噪声（全屏 fbm 是这个 pass 的成本大头）。
    vec3 col = mix(inner, outer, smoothstep(uCore, 1.0, r)) * fall;
#else
    vec3 col = mix(inner, outer, smoothstep(uCore, 1.0, r)) * fall * shape;
#endif
    col += promCol * band * prom * 1.9;

    // 靠近核心处补一层紧致辉光，让「日冕→光球」没有断层。**必须带窗口**：r^-3 的尾巴
    // 拖到 1.5 个太阳半径之外就成了一层盖住行星的灰。窗口让它到 0.45 就消失。
    col += vec3(1.2, 0.85, 0.55) * pow(max(0.0, uCore / max(r, 1e-3)), 3.0) * 0.42
         * smoothstep(0.52, 0.24, r);
    // 远处归零（避免 billboard 的方形边界露出来）。
    col *= smoothstep(1.0, 0.55, r);

    gl_FragColor = vec4(col * uIntensity, 1.0);
  }
`;

const CORONA_MAT = (size, core, intensity, oct, falloff = 2.6, simple = false) => new THREE.ShaderMaterial({
  defines: simple ? { CORONA_SIMPLE: '' } : {},
  uniforms: {
    uFbmOct: fbmOct(oct),
    uTime: { value: 0 },
    uSize: { value: size },
    uIntensity: { value: intensity },
    uCore: { value: core },
    uFalloff: { value: falloff },
  },
  vertexShader: CORONA_VERT,
  fragmentShader: CORONA_FRAG,
  transparent: true,
  blending: THREE.AdditiveBlending,
  depthWrite: false,
  depthTest: true,
  side: THREE.DoubleSide,
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

  // 日冕 billboard：`uSize` 是盘面半径（世界单位），`uCore` = R/uSize。
  const coronaSize = R * 4.0;
  const coronaMat = CORONA_MAT(coronaSize, R / coronaSize, 0.40, oct);
  const corona = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), coronaMat);
  corona.frustumCulled = false;
  corona.renderOrder = 5;
  g.add(corona);

  // 远景光晕：一片更大的、极淡的 additive，保证太阳在缩小到几个像素时仍然「在发光」。
  const haloMat = CORONA_MAT(R * 9.0, 0.11, 0.05, Math.max(2, oct - 2), 2.2, true);
  const halo = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), haloMat);
  halo.frustumCulled = false;
  halo.renderOrder = 4;
  g.add(halo);

  const parts = [photosphere, chromosphere, corona, halo];
  const setTier = (t) => {
    // 八度数现在是 **uniform**（见 util.js 里 NOISE_GLSL 那段），所以换档只改一个数值、
    // 不再触发 `needsUpdate` 重编译 —— 顺带把「每换一次档就重编一遍太阳 shader」也省掉了。
    parts.forEach((m) => { if (m.material.uniforms.uFbmOct) m.material.uniforms.uFbmOct.value = Math.max(1, t.oct); });
    corona.visible = !!t.corona;
    halo.visible = !!t.corona;
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
      haloMat.uniforms.uTime.value = t * 0.4;
    },
    setIntensity(v) {
      photosphereMat.uniforms.uIntensity.value = v;
      coronaMat.uniforms.uIntensity.value = 0.40;
    },
    setTier,
    dispose() {
      g.traverse((o) => { if (o.geometry) o.geometry.dispose(); if (o.material) o.material.dispose(); });
    },
  };
}
