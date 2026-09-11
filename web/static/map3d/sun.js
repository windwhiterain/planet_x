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
  varying vec3 vWorld;
  void main(){
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorld = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
`;

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
const CORONA_FRAG = /* glsl */`
  precision highp float;
  ${NOISE_GLSL}
  uniform float uTime;
  uniform float uSunR;        // 光球半径（世界单位）
  uniform float uOuter;       // 体积外半径（世界单位）
  uniform float uOuterRatio;  // uOuter / uSunR（外缘平滑归零用）
  uniform float uIntensity;
  uniform float uFalloff;     // 径向幂律（K-日冕投影大致 ~r^-2.6）
  uniform int   uSteps;
  varying vec3 vWorld;

  // 射线 × 球。球心在世界原点 —— 太阳就在原点（行星着色器里 lightDir 也是这么假设的）。
  vec2 hitSphere(vec3 ro, vec3 rd, float R){
    float b = dot(ro, rd);
    float c = dot(ro, ro) - R * R;
    float h = b * b - c;
    if (h < 0.0) return vec2(-1.0, -1.0);
    h = sqrt(h);
    return vec2(-b - h, -b + h);
  }

  // 密度场。**高对比**是「读起来像日珥实体」而不是「一团晕」的关键（用户明确要的）：
  // 平滑过渡会糊成灰雾，只有把低值区真的压到接近 0，丝状结构才立得起来。
  // 山脊项由**同一次** fbm 折出来，不再多跑一遍噪声 —— 省一半 ALU。
  float coronaDensity(vec3 p, float rr, float t){
    float fall = pow(max(1.0 / rr, 0.0), uFalloff);
    if (fall < 2.0e-3) return 0.0;                  // 便宜的先算：够小就别进噪声
    float n = fbm(p * (2.6 / uSunR) + vec3(0.0, 0.0, -t * 0.10));
    float ridge = 1.0 - abs(2.0 * n - 1.0);
    float shape = smoothstep(0.34, 0.90, n) * (0.40 + 1.05 * smoothstep(0.30, 0.92, ridge));
    // 外缘平滑归零：体积球本身有个硬轮廓，密度必须**在球面之前**就回到 0，
    // 否则那个球体的剪影会在天上切出一圈硬边（和之前 billboard 的方角是同一类错）。
    return fall * shape * smoothstep(uOuterRatio, uOuterRatio * 0.70, rr);
  }

  void main(){
    vec3 ro = cameraPosition;
    vec3 rd = normalize(vWorld - ro);

    vec2 to = hitSphere(ro, rd, uOuter);
    if (to.y <= 0.0) discard;                       // 这条射线根本不碰体积
    float t0 = max(to.x, 0.0);
    float t1 = to.y;
    // 日面之后不积分（日冕在日面背后不发光）。相机在体积内时 t0 = 0 也成立。
    vec2 ti = hitSphere(ro, rd, uSunR * 1.004);
    if (ti.x > 0.0) t1 = min(t1, ti.x);
    if (t1 <= t0) discard;

    // 步长由「这条射线穿过体积的长度」决定 ⇒ 不论掠射还是正穿都保证覆盖，不会漏采样。
    float dt = (t1 - t0) / float(max(uSteps, 1));
    float t = uTime;
    vec3 acc = vec3(0.0);
    float trans = 1.0;                              // 光学薄，但留一点自吸收，免得贴边糊成一片
    // 起点抖半个步长：固定步长会在球面上留下同心分层，抖一下就没有了。
    float tt = t0 + dt * 0.5;
    for (int i = 0; i < uSteps; i++) {
      if (tt > t1 || trans < 0.02) break;
      vec3 p = ro + rd * tt;
      float rr = length(p) / uSunR;
      float d = coronaDensity(p, rr, t);
      if (d > 0.0) {
        vec3 col = mix(vec3(1.15, 0.90, 0.62), vec3(0.48, 0.56, 0.95),
                       smoothstep(1.0, 3.0, rr));
        float a = d * dt;
        acc += col * a * trans;
        trans *= exp(-a * 0.5);
      }
      tt += dt;
    }

    gl_FragColor = vec4(acc * uIntensity, 1.0);
  }
`;

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
