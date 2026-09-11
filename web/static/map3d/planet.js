// 行星X WebUI — 行星：表面着色器 / 云层 / 大气散射壳 / 星环。
//
// 旧版是一颗「平涂噪声球」：fbm 直出色、没有法线细节、没有云层、没有海洋高光、没有夜面、
// 没有大气。远看还行，一旦 `?focus=地球&dist=7` 拉近就露馅——大陆是色块，球面是塑料。
//
// 这一版的做法：
//   * **表面**：域扰动 fbm 造大陆 + ridged 造山系，再用**有限差分**求高度场梯度做**解析法线**
//     （真地形起伏，不是贴一张法线图）。岩石类再加一层程序化陨石坑场（环脊 + 碗）。
//   * **自转**：不改 `mesh.rotation`，而是把 `uSpin` 交给着色器去旋转**采样方向**。
//     这样「地表的纹理在转」与「城市钉在固定的球面方向上」可以同时成立——夜面城市灯才有意义。
//   * **夜面**：`uCityDirs` 是这颗星上**真实城市**的对象空间方向（由 state 里的城算出），
//     只有背光面亮起来，暖色、带一圈更宽的晕。
//   * **大气**：`DoubleSide` 的自发光壳，正面（贴地那半）给薄雾、背面（绕到球后那半）给
//     临边辉光；颜色沿晨昏线从白蓝过渡到落日橙红（Rayleigh 的廉价近似）。
//   * **星环**：径向密度用噪声而非等距条纹，带卡西尼缝；**环在行星上的投影**（表面着色器里
//     对每个像素反解「该点朝太阳的射线是否穿过环平面」）与**行星在环上的投影**（环着色器里
//     解析求「该点到太阳的射线是否穿过行星球」）双向都有——这两个影子是「土星感」的一半。

import * as THREE from 'three';
import { NOISE_GLSL, fbmOct, hex2rgb, lighten } from './util.js';
import { classIndex } from './kinds.js';
import { TUNING } from './tuning.js';

const PLANET_VERT = /* glsl */`
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vRel;      // 相对**本体中心**的世界偏移（星环投影必须用它，不能用 vWorldPos）
  varying vec3 vNormal;
  // 对象→世界的 3x3。**必须**从顶点着色器传下来：modelMatrix 是顶点着色器才有的
  // 内置量，片元里引用它是「undeclared identifier」——整颗行星会编译失败、什么都不显示。
  varying mat3 vObjToWorld;
  void main(){
    vObjToWorld = mat3(modelMatrix);
    vObjPos = position;
    // 世界空间法线：normalMatrix 是「对象→视图空间」的。而片元里的光方向是世界空间的
    // （太阳在原点）。两者混用会让 dot(n, lightDir) 失去意义——旧版踩过的坑。
    vNormal = normalize(mat3(modelMatrix) * normal);
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    vRel = mat3(modelMatrix) * position;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
`;

// 行星/卫星/矮行星都能用的公共小块：绕轴旋转、切空间、海洋/岩石交界。
const COMMON_FRAG = /* glsl */`
  vec3 rotAxis(vec3 v, vec3 a, float ang){
    float c = cos(ang), s = sin(ang);
    return v * c + cross(a, v) * s + a * dot(a, v) * (1.0 - c);
  }
  vec3 hash3t(vec3 p){
    p = vec3(dot(p, vec3(127.1, 311.7, 74.7)),
             dot(p, vec3(269.5, 183.3, 246.1)),
             dot(p, vec3(113.5, 271.9, 124.6)));
    return fract(sin(p) * 43758.5453123);
  }
  // 固定八度的 fbm（比 NOISE_GLSL 里那个由 uFbmOct 控制的版本便宜，用于求法线时的差分）。
  float fbmFast(vec3 p){
    return 0.5 * vnoise(p) + 0.25 * vnoise(p * 2.02) + 0.125 * vnoise(p * 4.08);
  }
  // 陨石坑场：每格最多一个坑，坑心被限制在格子内部（±0.16，影响半径 ≲0.34）⇒ 不需要查
  // 邻格，也不会有格边界的硬切。返回 = 碗(负) + 环脊(正)。
  float craterField(vec3 p, float S, float density){
    vec3 sp = p * S;
    vec3 cell = floor(sp);
    vec3 r = hash3t(cell);
    if (r.z > density) return 0.0;
    vec3 c = cell + 0.5 + (r - 0.5) * 0.32;   // 必须整体用 vec3：vec3 + vec2 不合法
    float d = length(sp - c);
    float rad = 0.14 + 0.10 * hash13(cell + 3.7);
    float t = d / rad;
    if (t > 1.7) return 0.0;
    float bowl = -smoothstep(0.0, 0.92, t);
    float rim = exp(-pow((t - 0.98) / 0.17, 2.0));
    return bowl * 0.55 + rim * 0.45;
  }
`;

const PLANET_FRAG = /* glsl */`
  precision highp float;
  ${NOISE_GLSL}
  ${COMMON_FRAG}
  uniform vec3  uBase;
  uniform vec3  uAccent;
  uniform vec3  uAtmo;
  uniform float uAtmoAmt;
  uniform float uBanded;
  uniform float uEmissive;
  uniform float uRough;
  uniform float uMetal;
  uniform float uAmbient;
  uniform float uSpin;
  uniform float uRelief;
  uniform float uCloudAmt;
  uniform float uCityLights;
  uniform int   uClass;
  // 星环在该行星上的投影（环平面 = 过球心的平面，法线 uRingN，半径区间 [uRingIn, uRingOut]）。
  uniform float uHasRing;
  uniform vec3  uRingN;
  uniform float uRingIn;
  uniform float uRingOut;
  uniform vec3  uCityDirs[CITY_MAX];
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vRel;
  varying vec3 vNormal;
  varying mat3 vObjToWorld;

  // --- 高度场 ---------------------------------------------------------------
  // 域扰动：先在大陆尺度上扰动一次，让大陆边缘是「流动」的而不是圆滚滚的噪声团；
  // 再叠一层 ridged 做山系。返回值大致在 [-0.2, 1.0]。
  float terrain(vec3 p){
    vec3 q = vec3(fbm(p * 1.55), fbm(p * 1.55 + 17.3), fbm(p * 1.55 + 43.7));
    vec3 w = p * 1.85 + (q - 0.5) * 1.45;
    float cont = fbmFast(w) + fbmFast(w * 2.1) * 0.35;
    float mount = ridged(w * 2.9 + 3.0);
    float h = cont * 0.74 + mount * 0.26;
    // 岩石类加陨石坑（类地/金星不加：那两类的「坑」早被大气/地质抹平了）。
    if (uClass == 0 || uClass == 3 || uClass == 4 || uClass == 8) {
      h += craterField(p, 6.5, 0.5) * 0.30;
      h += craterField(p * 2.3 + 11.0, 6.5, 0.4) * 0.13;
    }
    return h;
  }

  // 海平面：类地/泰坦有液态表面，岩石类没有。
  float seaLevel(){
    if (uClass == 1) return 0.585;
    if (uClass == 7) return 0.98;      // 泰坦：液态甲烷湖，很少
    return -10.0;                       // 其余类没有海 ⇒ 永远在「陆地」分支
  }

  // --- 各类的表面反照率 ------------------------------------------------------
  vec3 terranColor(vec3 ds, float h){
    float lat = abs(ds.y);
    float sea = seaLevel();
    // 大陆架过渡要**窄**：宽过渡会把整片洋面染成浅蓝，而真实地球的洋是接近黑的深蓝，
    // 只有贴着岸的一圈才是青绿（对照 scratch/ref/earth-blue-marble.jpg）。
    float shelf = smoothstep(sea - 0.045, sea + 0.008, h);      // 0=深洋 … 1=陆
    vec3 deep = vec3(0.004, 0.020, 0.075);
    vec3 mid = vec3(0.012, 0.055, 0.200);
    vec3 shallow = vec3(0.050, 0.200, 0.260);
    vec3 ocean = mix(deep, mid, smoothstep(0.0, 0.55, shelf));
    ocean = mix(ocean, shallow, smoothstep(0.62, 1.0, shelf));
    // 陆地：滩 → 干草原 → 森林 → 岩 → 雪（按高度爬）。整体是**橄榄/土黄**系，
    // 不是「糖绿」——真实地球的植被在太空里读起来接近暗橄榄褐。
    float land = smoothstep(sea, sea + 0.02, h);
    float alt = clamp((h - sea) / 0.34, 0.0, 1.0);
    vec3 beach = vec3(0.46, 0.38, 0.21);
    vec3 grass = vec3(0.20, 0.19, 0.07);
    vec3 forest = vec3(0.075, 0.115, 0.035);
    vec3 rockC = vec3(0.22, 0.19, 0.15);
    vec3 snow = vec3(0.86, 0.90, 0.96);
    vec3 landC = mix(beach, grass, smoothstep(0.0, 0.14, alt));
    landC = mix(landC, forest, smoothstep(0.12, 0.38, alt));
    landC = mix(landC, rockC, smoothstep(0.46, 0.72, alt));
    landC = mix(landC, snow, smoothstep(0.70, 0.92, alt));
    // 干旱带：低纬 → 沙漠化（撒哈拉/阿拉伯/澳洲那种大片土黄，是地球最容易认的特征）。
    // 覆盖率给得比「合理」更高一点：太空视角下沙漠是显眼的。
    float arid = (1.0 - smoothstep(0.22, 0.55, lat)) * (1.0 - smoothstep(0.02, 0.30, alt));
    arid *= smoothstep(0.28, 0.58, fbmFast(ds * 3.2 + 7.0) + 0.32);
    landC = mix(landC, vec3(0.58, 0.44, 0.22), arid * 0.72);
    // 高频细节：没有它，大陆就是一整块均匀的绿（「绿色大理石」）。同时按第二层噪声
    // 做生物群系扰动，让同一纬度上也有深浅差别。
    float detail = fbm(ds * 16.0 + 2.2);
    float biome = fbm(ds * 6.5 + 19.0);
    landC *= 0.78 + 0.34 * detail;
    landC = mix(landC, landC * vec3(1.18, 1.05, 0.82), smoothstep(0.40, 0.72, biome) * 0.35);
    // 极冠：噪声破边，别是一条平滑的纬度线。
    float capNoise = fbm(ds * 5.0 + 3.3) * 0.14;
    float pole = smoothstep(0.80 - capNoise, 0.93 - capNoise, lat);
    vec3 col = mix(ocean, landC, land);
    col = mix(col, vec3(0.93, 0.96, 0.99), pole * (0.55 + 0.45 * land));
    return col;
  }

  vec3 gasColor(vec3 ds, float lat){
    // 纬向急流：把纬度当主变量，用域扰动让每条带自己是湍流的。
    // ⚠ 剪切量（shear 乘在纬向上）要**小**：原来乘 5.0/9.0，相位被扰动到 ±5 rad，
    // 纬向条带被撕成一坨坨斑块，看起来不像气巨。真实的气巨带纹是**强纬向、弱经向**的，
    // 湍流只把边界揉皱，不该把带揉没。这里把 warp 幅度也降到 0.85。
    vec3 w = warp(ds * 3.0, 0.85, 0.0);
    float shear = fbm(w * 2.2) * 0.5;
    // 三层不同频率的纬向带 + 少量剪切，叠出「宽带里套细纹」的层次。
    float b1 = sin(lat * 12.0 + shear * 1.7);
    float b2 = sin(lat * 27.0 + shear * 3.1);
    float b3 = sin(lat * 52.0 + shear * 5.0);
    float band = 0.5 + 0.5 * (0.50 * b1 + 0.32 * b2 + 0.18 * b3);
    band = smoothstep(0.26, 0.80, band);
    // 两端各自再推开：config 给的 base/accent 往往只差一点点（土星的驼 vs 暗驼），
    // 直接 mix 出来是一条没有对比的色带。
    vec3 c = mix(uAccent * 0.58, uBase * 1.16, band);
    // 带内细流：真实气巨的每一条带自己都是湍流的，不是一条均匀色条。
    c *= 0.90 + 0.20 * fbm(w * 7.0 + 2.0);
    // 极区涡旋：高纬处收紧、变暗（木星/土星的极地都是暗的）。
    float polar = smoothstep(0.72, 1.0, abs(lat));
    c = mix(c, uAccent * 0.55, polar * 0.7);
    // 风暴：几颗定点的椭圆涡旋（大红斑是其中最大的一颗）。
    const vec3 STORM[3] = vec3[3](vec3(0.55, -0.18, 0.62), vec3(-0.72, 0.26, 0.42), vec3(0.12, 0.44, -0.88));
    for (int i = 0; i < 3; i++) {
      vec3 sd = normalize(STORM[i]);
      float dd = length(ds - sd);
      float spot = exp(-pow(dd / (0.13 + 0.05 * float(i)), 2.0));
      // 涡旋内部有环流纹理，不是一块纯色补丁。
      float swirl = 0.75 + 0.25 * sin(dd * 60.0 + fbm(ds * 8.0) * 6.0);
      vec3 sc = (i == 0) ? vec3(0.66, 0.30, 0.17) : mix(uAccent, uBase, 0.35) * 0.8;
      c = mix(c, sc, spot * swirl * 0.85);
    }
    return c;
  }

  vec3 icyColor(vec3 ds, float lat){
    if (uBanded > 0.5) {
      // 冰巨星：带极淡、极细，整体偏青蓝。
      vec3 w = warp(ds * 3.0, 1.1, 0.0);
      float band = 0.5 + 0.5 * sin(lat * 13.0 + fbm(w * 1.8) * 2.2);
      band = smoothstep(0.34, 0.82, band);
      return mix(uAccent, uBase, band * 0.75 + 0.12);
    }
    // 冰封卫星：光滑冰面 + 细裂纹 + 少量坑。
    float var = fbm(ds * 2.4);
    vec3 c = mix(uBase, uAccent, var * 0.32);
    float crack = abs(fbm(ds * 7.0) - 0.5);
    c = mix(c, vec3(0.86, 0.94, 1.02), smoothstep(0.14, 0.06, crack) * 0.55);
    c += craterField(ds, 5.0, 0.35) * 0.10;
    return c;
  }

  vec3 rockColor(vec3 ds, float lat, float h){
    float var = fbm(ds * 3.0);
    vec3 c = mix(uBase, uAccent, var * 0.7);
    if (uClass == 3) {
      // 火星：红荒漠 + 暗反照率区（Syrtis Major 那种）+ 极冠。
      c = mix(c, uAccent * 1.05, smoothstep(0.42, 0.72, fbm(ds * 2.1 + 5.0)) * 0.75);
      float capN = fbm(ds * 4.5) * 0.10;
      float pole = smoothstep(0.84 - capN, 0.95 - capN, lat);
      c = mix(c, vec3(0.94, 0.95, 0.96), pole * 0.85);
    } else {
      float capN = fbm(ds * 5.0) * 0.10;
      float pole = smoothstep(0.87 - capN, 0.96 - capN, lat);
      c = mix(c, vec3(0.88, 0.90, 0.94), pole * 0.6);
    }
    // 高度分层：高处长亮、洼地压暗（月海）。
    c *= 0.82 + 0.36 * smoothstep(-0.1, 0.7, h);
    return c;
  }

  vec3 titanColor(vec3 ds, float lat){
    float latT = clamp((lat + 1.0) * 0.5, 0.0, 1.0);
    vec3 c = mix(uAccent, uBase, latT);
    // 甲烷湖：极区少量暗斑。
    float lake = smoothstep(0.72, 0.9, fbm(ds * 3.4 + 9.0)) * smoothstep(0.55, 0.9, abs(ds.y));
    return mix(c, vec3(0.08, 0.10, 0.12), lake * 0.7);
  }

  vec3 venusColor(vec3 ds, float lat){
    // 浓厚硫磺云：**高速**纬向涡旋（金星大气 4 天绕一圈，是最有辨识度的特征）。
    // 用强域扰动把纬向条纹撕成 Y 形/涡卷，而不是一张均匀的驼色纸。
    vec3 w = warp(ds * 6.0 + vec3(0.0, lat * 2.0, 0.0), 3.4, 0.0);
    float s = fbm(w * 2.6);
    float swirl = 0.5 + 0.5 * sin(lat * 11.0 + s * 9.0);
    float streak = fbm(vec3(ds.x * 3.0, ds.y * 16.0, ds.z * 3.0));
    vec3 c = mix(uAccent, uBase, smoothstep(0.15, 0.85, s * 0.62 + swirl * 0.38));
    c *= 0.86 + 0.28 * streak;
    // 极区偶极涡旋（金星南极的暖区）。
    c = mix(c, uAccent * 1.15, smoothstep(0.75, 1.0, abs(lat)) * 0.45);
    return c;
  }

  // 星环在表面上的投影：从表面点朝太阳的射线，是否在环平面上穿过 [内缘,外缘]。
  // P 必须是**相对球心**的世界偏移（vRel），环平面过球心。
  float ringShadow(vec3 P, vec3 L){
    if (uHasRing < 0.5) return 0.0;
    float dn = dot(L, uRingN);
    if (abs(dn) < 1e-4) return 0.0;
    float t = -dot(P, uRingN) / dn;
    if (t <= 0.0) return 0.0;
    vec3 q = P + L * t;
    float r = length(q);
    float inside = smoothstep(uRingIn - 0.02, uRingIn + 0.06, r) * (1.0 - smoothstep(uRingOut - 0.06, uRingOut + 0.02, r));
    // 环不是实心圆盘：按同一个径向密度函数打洞，影子也要有缝。
    float tt = (r - uRingIn) / max(uRingOut - uRingIn, 1e-4);
    float dens = 0.62 + 0.24 * sin(tt * 118.0) * (1.0 - smoothstep(0.40, 0.50, tt) * 0.9);
    return inside * clamp(dens, 0.15, 1.0);
  }

  void main(){
    vec3 d = normalize(vObjPos);                    // 真实球面法线（对象空间）
    // 自转：把**采样方向**反向转，而不是转模型——城市方向因此可以钉死在对象空间里。
    vec3 ds = rotAxis(d, vec3(0.0, 1.0, 0.0), -uSpin);
    vec3 n = normalize(vNormal);
    vec3 lightDir = normalize(-vWorldPos);          // 太阳在原点
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float lat = ds.y;

    // --- 表面颜色 + 解析法线 ------------------------------------------------
    float h = terrain(ds);
    vec3 albedo;
    bool ocean = false;
    if (uClass == 1)      { albedo = terranColor(ds, h); ocean = h < seaLevel(); }
    else if (uClass == 5) { albedo = gasColor(ds, lat); }
    else if (uClass == 6) { albedo = icyColor(ds, lat); }
    else if (uClass == 2) { albedo = venusColor(ds, lat); }
    else if (uClass == 7) { albedo = titanColor(ds, lat); ocean = h < seaLevel(); }
    else                  { albedo = rockColor(ds, lat, h); }

    // 起伏法线：在 ds 的切平面里做前向差分。气巨/金星没有固体地表，跳过（省一半开销）。
    vec3 nSurface = d;
    if (uRelief > 0.0 && uClass != 5 && uClass != 2) {
      vec3 t1 = normalize(cross(ds, vec3(0.0, 1.0, 0.0)) + vec3(1e-5, 0.0, 1e-5));
      vec3 t2 = cross(ds, t1);
      float e = 0.011;
      float h1 = terrain(normalize(ds + t1 * e));
      float h2 = terrain(normalize(ds + t2 * e));
      vec3 bump = ((h1 - h) / e) * t1 + ((h2 - h) / e) * t2;
      // ocean 处把起伏压平（水面是平的）。
      float flat_ = ocean ? 0.12 : 1.0;
      vec3 nS = normalize(ds - bump * uRelief * 0.055 * flat_);
      nSurface = rotAxis(nS, vec3(0.0, 1.0, 0.0), uSpin);   // 转回对象空间
      n = normalize(vObjToWorld * nSurface);
    } else if (uRelief > 0.0 && (uClass == 5)) {
      // 气巨：用低幅度的湍流给云顶一点起伏（不是地形，是云顶高度）。
      vec3 t1 = normalize(cross(ds, vec3(0.0, 1.0, 0.0)) + vec3(1e-5, 0.0, 1e-5));
      vec3 t2 = cross(ds, t1);
      float e = 0.02;
      float c0 = fbmFast(ds * 3.0);
      float c1 = fbmFast(normalize(ds + t1 * e) * 3.0);
      float c2 = fbmFast(normalize(ds + t2 * e) * 3.0);
      vec3 bump = ((c1 - c0) / e) * t1 + ((c2 - c0) / e) * t2;
      nSurface = normalize(ds - bump * uRelief * 0.05);
      n = normalize(vObjToWorld * nSurface);
    }

    // --- 光照 --------------------------------------------------------------
    float ndl = dot(n, lightDir);
    float diff = max(ndl, 0.0);
    // 晨昏线柔化：真实大气的散射让明暗交界是一条过渡带，不是硬切。
    float lit = smoothstep(-0.06, 0.16, ndl);

    vec3 col = albedo * (uAmbient + (1.0 - uAmbient) * lit);

    // 星环投影。
    float rsh = ringShadow(vRel, lightDir);
    col *= (1.0 - rsh * 0.78 * lit);

    // 自发光（config 的 emissive，给类地一点点「大气自身的亮度」）。
    col += albedo * uEmissive * 0.35;

    // --- 镜面：海洋是唯一有明显高光的表面 ------------------------------------
    if (uClass == 1 || uClass == 7) {
      float water = (1.0 - smoothstep(seaLevel() - 0.01, seaLevel() + 0.02, h));
      vec3 hv = normalize(lightDir + viewDir);
      float spec = pow(max(dot(n, hv), 0.0), 180.0) * water * lit * 1.2;
      col += vec3(1.0, 0.97, 0.90) * spec;
      // 菲涅尔：掠射角下水变得像镜子（这是「地球照片」最标志性的一条）。
      float fres = pow(1.0 - max(dot(n, viewDir), 0.0), 5.0);
      col += uAtmo * fres * water * lit * 0.55;
    } else {
      // 非海洋：一点点粗糙高光，避免完全 lambert 的塑料感。
      vec3 hv = normalize(lightDir + viewDir);
      col += vec3(1.0) * pow(max(dot(n, hv), 0.0), 34.0) * (1.0 - uRough) * 0.16 * lit;
    }

    // --- 夜面城市灯 --------------------------------------------------------
    // 只有背光面亮；灯的**位置**来自 state 里这颗星上真实的城。
    if (uCityLights > 0.0) {
      float night = smoothstep(0.10, -0.22, ndl);
      if (night > 0.001) {
        float lights = 0.0;
        for (int i = 0; i < CITY_MAX; i++) {
          vec3 cd = uCityDirs[i];
          if (dot(cd, cd) < 0.5) continue;              // 空槽（未使用的槽写 0）
          float ang = dot(d, cd);
          // 两层：紧致的市区亮核 + 更宽的一圈城市群/郊区辉光。
          // 角半径由 ang > cos θ 反推：0.99980 对应约 1.1°，0.9980 对应约 3.6°。
          float core = smoothstep(0.99980, 0.99995, ang);
          float glow = smoothstep(0.9980, 0.99980, ang) * 0.45;
          lights += core + glow;
        }
        // 暖色（钠灯/城市辉光的色温），并且被云层/大气稍微晕开。
        col += vec3(1.0, 0.72, 0.36) * lights * night * uCityLights * 2.2;
      }
    }

    // --- 大气边缘辉光（贴在球面上的那一层；更大范围的那圈由大气壳负责）--------
    float rim = pow(1.0 - max(dot(n, viewDir), 0.0), 3.2);
    col += uAtmo * rim * uAtmoAmt * (0.05 + 0.95 * lit) * 0.85;

    gl_FragColor = vec4(col, 1.0);
  }
`;

// --- 大气壳 ------------------------------------------------------------------
// 用 `DoubleSide` + `gl_FrontFacing` 把「贴着球的那半」和「绕到球后的那半」分开处理：
// 背面 = 临边辉光（视线穿过最厚的大气），正面 = 覆盖在星球上的薄雾。
const ATMO_VERT = /* glsl */`
  varying vec3 vWorldPos;
  varying vec3 vRel;
  varying vec3 vNormal;
  void main(){
    vNormal = normalize(mat3(modelMatrix) * normal);
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    // 大气壳的「高度」必须相对**本体中心**算，不能拿 vWorldPos 的模（那是到太阳的距离）。
    vRel = mat3(modelMatrix) * position;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
`;

const ATMO_FRAG = /* glsl */`
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

    // 照明用**切点**的法线：大气是被视线最接近球心那一小段上的太阳高度角照亮的，
    // 而不是被壳面上这个点的法线（后者会让盘面中央和边缘平分亮度，丢掉临边亮环）。
    vec3 T = O + D * tc;
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
`;

// --- 云层 -------------------------------------------------------------------
const CLOUD_VERT = /* glsl */`
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;
  void main(){
    vObjPos = position;
    vNormal = normalize(mat3(modelMatrix) * normal);
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
`;

const CLOUD_FRAG = /* glsl */`
  precision highp float;
  ${NOISE_GLSL}
  uniform float uTime;
  uniform float uSpin;
  uniform float uAmount;
  uniform vec3  uTint;
  uniform float uOpacity;
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;
  void main(){
    vec3 d = normalize(vObjPos);
    // 云的自转与地表**不同步**（真实大气有风），所以这里用自己的一套旋转。
    vec3 ds = d;
    float ang = -uSpin;
    float c = cos(ang), s = sin(ang);
    ds = vec3(ds.x * c - ds.z * s, ds.y, ds.x * s + ds.z * c);

    // 云团：纬度带状的环流 + 域扰动，做出「涡旋/带状」而不是均匀的花花。
    float latB = 0.55 + 0.45 * sin(ds.y * 7.0 + fbm(ds * 3.0) * 2.2);
    vec3 w = warp(ds * 5.0, 1.7, uTime * 0.02);
    float cov = fbm(w * 3.0 + vec3(0.0, 0.0, uTime * 0.01));
    // 叠一层更细的云絮：只有大尺度的云看起来像一团团棉花糖。
    cov = cov * 0.72 + fbm(ds * 11.0 + vec3(4.0, 0.0, uTime * 0.03)) * 0.34;
    float cover = smoothstep(0.52, 0.74, cov * (0.55 + 0.65 * latB));
    cover = clamp(cover * uAmount, 0.0, 1.0);
    if (cover < 0.006) discard;

    vec3 n = normalize(vNormal);
    vec3 lightDir = normalize(-vWorldPos);
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float ndl = dot(n, lightDir);
    float lit = smoothstep(-0.10, 0.22, ndl);
    // 云顶是白的，但边缘/厚度大处偏灰（自遮挡）。
    float thick = smoothstep(0.15, 0.85, cover);
    vec3 col = mix(uTint * 0.72, uTint, thick) * (0.10 + 0.90 * lit);
    // 散射：云在逆光时边缘透光。
    float fwd = pow(max(dot(-lightDir, viewDir), 0.0), 3.0);
    col += uTint * fwd * 0.25 * lit;
    // 临边处视线穿过更厚的一层云 → 稍微不透明一点。
    float rim = pow(1.0 - max(dot(n, viewDir), 0.0), 2.0);
    float alpha = clamp(cover * (0.82 + 0.18 * rim) * uOpacity, 0.0, 1.0);
    gl_FragColor = vec4(col, alpha);
  }
`;

// --- 星环 -------------------------------------------------------------------
const RING_VERT = /* glsl */`
  varying vec3 vWorldPos;
  varying vec3 vLocal;
  void main(){
    vLocal = position;
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
`;

const RING_FRAG = /* glsl */`
  precision highp float;
  ${NOISE_GLSL}
  uniform vec3  uCol;
  uniform vec3  uCol2;
  uniform float uInner;
  uniform float uOuter;
  uniform vec3  uCenter;      // 行星中心（世界）
  uniform float uPlanetR;
  varying vec3 vWorldPos;
  varying vec3 vLocal;

  float hash1(float n){ return fract(sin(n * 91.3458) * 47453.5453); }

  // 径向密度：真实环不是等距条纹——用噪声调制疏密，再挖出卡西尼缝/恩克缝。
  float density(float t){
    float ringlets = 0.5 + 0.5 * sin(t * 118.0 + hash1(floor(t * 30.0)) * 6.283);
    float a = 0.58 + 0.42 * ringlets;
    // 内圈 C 环：稀薄。
    a *= 0.42 + 0.58 * smoothstep(0.05, 0.30, t);
    // 卡西尼缝。
    a *= 1.0 - 0.88 * smoothstep(0.415, 0.445, t) * (1.0 - smoothstep(0.478, 0.508, t));
    // 恩克缝（外侧细缝）。
    a *= 1.0 - 0.55 * smoothstep(0.855, 0.872, t) * (1.0 - smoothstep(0.888, 0.905, t));
    // 一条更细的次级缝。
    a *= 1.0 - 0.35 * smoothstep(0.66, 0.672, t) * (1.0 - smoothstep(0.682, 0.695, t));
    return clamp(a, 0.0, 1.0);
  }

  void main(){
    float rad = length(vLocal.xy);
    float t = (rad - uInner) / max(uOuter - uInner, 1e-4);
    if (t < 0.0 || t > 1.0) discard;
    float dens = density(t);

    // 前后向散射：相位角接近 0（太阳在相机背后）时环变亮——「冲日效应」，真实土星环最
    // 引人注目的一条。用 HG 相函数的一个廉价近似。
    vec3 toSun = normalize(-vWorldPos);
    vec3 toCam = normalize(cameraPosition - vWorldPos);
    float cosPhase = dot(toSun, toCam);
    float phase = 0.55 + 0.75 * pow(clamp(1.0 + cosPhase, 0.0, 2.0) * 0.5, 2.2);

    // 行星在环上的投影：从环上这点朝太阳的射线，是否穿过行星球。
    vec3 w = uCenter - vWorldPos;
    float tc = dot(w, toSun);
    float shadow = 0.0;
    if (tc > 0.0) {
      float d2 = dot(w, w) - tc * tc;
      float rr = uPlanetR;
      // 半影：太阳是有限大的，影子边缘有一段过渡。
      float pen = max(rr * 0.10, 1e-3);
      shadow = 1.0 - smoothstep(rr * rr, (rr + pen) * (rr + pen), d2);
    }
    // 环自身的厚度：掠射角下变亮（视线穿过更多粒子）。
    float graze = 1.0 - abs(dot(normalize(vec3(0.0, 1.0, 0.0)), toCam));

    vec3 col = mix(uCol, uCol2, smoothstep(0.35, 0.85, t) * 0.6);
    float alpha = dens * (1.0 - shadow * 0.86) * (0.55 + 0.45 * phase);
    alpha *= 0.75 + 0.55 * graze;
    gl_FragColor = vec4(col * (0.55 + 0.75 * phase) * (1.0 - shadow * 0.72), clamp(alpha, 0.0, 1.0));
  }
`;

export const CITY_MAX = 12;

// ---------------------------------------------------------------------------
export function planetMaterial(spec, tier, opts = {}) {
  const [base, accent, atmo] = [hex2rgb(spec.color), hex2rgb(spec.accent), hex2rgb(spec.atmosphere)];
  const dirs = [];
  for (let i = 0; i < CITY_MAX; i++) dirs.push(new THREE.Vector3(0, 0, 0));
  const cls = classIndex(spec.class);
  const isGas = cls === 5;
  const hasAtmo = (spec.atmosphere && spec.atmosphere !== '#000000') || cls === 1 || cls === 2 || cls === 5 || cls === 6 || cls === 7;

  const mat = new THREE.ShaderMaterial({
    vertexShader: PLANET_VERT,
    fragmentShader: PLANET_FRAG,
    defines: { CITY_MAX: CITY_MAX },
    uniforms: {
      uFbmOct: fbmOct(tier.oct),
      uBase: { value: new THREE.Vector3(...base) },
      uAccent: { value: new THREE.Vector3(...accent) },
      uAtmo: { value: new THREE.Vector3(...atmo) },
      uAtmoAmt: { value: hasAtmo ? (isGas ? 0.85 : 1.30) : 0.0 },
      uBanded: { value: spec.banded ? 1.0 : 0.0 },
      uEmissive: { value: spec.emissive || 0.0 },
      uRough: { value: spec.roughness != null ? spec.roughness : 0.8 },
      uMetal: { value: spec.metalness || 0.0 },
      uAmbient: { value: TUNING.shaderAmbient },
      uSpin: { value: 0 },
      uRelief: { value: tier.relief ? TUNING.relief : 0.0 },
      uCloudAmt: { value: TUNING.cloudAmount },
      uCityLights: { value: tier.nightLights ? TUNING.cityLights : 0.0 },
      uClass: { value: cls },
      uHasRing: { value: opts.ring ? 1.0 : 0.0 },
      uRingN: { value: opts.ring ? opts.ring.normal.clone() : new THREE.Vector3(0, 1, 0) },
      uRingIn: { value: opts.ring ? opts.ring.inner : 1 },
      uRingOut: { value: opts.ring ? opts.ring.outer : 1 },
      uCityDirs: { value: dirs },
    },
  });
  mat.userData.hasAtmo = hasAtmo;
  return mat;
}

export function createAtmosphere(radius, spec, tier) {
  const [atmo, sunset] = [hex2rgb(spec.atmosphere), hex2rgb(spec.accent)];
  const shellR = radius * 1.035;
  const mat = new THREE.ShaderMaterial({
    vertexShader: ATMO_VERT,
    fragmentShader: ATMO_FRAG,
    uniforms: {
      uAtmo: { value: new THREE.Vector3(...atmo).multiplyScalar(0.4).add(new THREE.Vector3(0.22, 0.34, 0.62)) },
      uSunset: { value: new THREE.Vector3(...sunset).lerp(new THREE.Vector3(0.95, 0.38, 0.16), 0.55) },
      uDensity: { value: 1.0 },
      uPlanetR: { value: radius },
      uShellR: { value: shellR },
      // 标高：真实大气 ~8 km vs 地球 6371 km ⇒ 约 1/800。这里为了看得见放宽到壳厚的 45%。
      uScaleH: { value: (shellR - radius) * 0.26 },
    },
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    // 只要相机侧那半个壳：b>Rp 时前后两个壳面投到同一批像素，双面会让亮度翻倍。
    side: THREE.FrontSide,
  });
  const mesh = new THREE.Mesh(new THREE.SphereGeometry(shellR, tier.seg[0], tier.seg[1]), mat);
  mesh.visible = !!tier.atmo;
  return mesh;
}

export function createClouds(radius, spec, tier) {
  const cls = classIndex(spec.class);
  const mat = new THREE.ShaderMaterial({
    vertexShader: CLOUD_VERT,
    fragmentShader: CLOUD_FRAG,
    uniforms: {
      uFbmOct: fbmOct(Math.max(3, tier.oct - 1)),
      uTime: { value: 0 },
      uSpin: { value: 0 },
      uAmount: { value: cls === 2 ? 0.92 : 0.40 },
      uTint: { value: new THREE.Vector3(0.96, 0.97, 1.0) },
      uOpacity: { value: 0.97 },
    },
    transparent: true,
    depthWrite: false,
    side: THREE.FrontSide,
  });
  const mesh = new THREE.Mesh(new THREE.SphereGeometry(radius * 1.008, tier.seg[0], tier.seg[1]), mat);
  mesh.visible = !!tier.clouds;
  mesh.renderOrder = 2;
  return mesh;
}

export function createRing(radius, spec, tier, planeQuat) {
  const inner = radius * TUNING.ringInner;
  const outer = radius * TUNING.ringOuter;
  const [r, g, b] = lighten(spec.accent || '#c9b08a', 0.22);
  const [r2, g2, b2] = lighten(spec.color || '#e6d9c0', 0.30);
  const mat = new THREE.ShaderMaterial({
    vertexShader: RING_VERT,
    fragmentShader: RING_FRAG,
    uniforms: {
      uFbmOct: fbmOct(Math.max(2, tier.oct - 2)),
      uCol: { value: new THREE.Vector3(r, g, b) },
      uCol2: { value: new THREE.Vector3(r2, g2, b2) },
      uInner: { value: inner },
      uOuter: { value: outer },
      uCenter: { value: new THREE.Vector3() },
      uPlanetR: { value: radius },
    },
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  const geo = new THREE.RingGeometry(inner, outer, 256, 1);
  const mesh = new THREE.Mesh(geo, mat);
  // RingGeometry 躺在局部 XY 平面；转到 XZ（地图平面），再乘上该天体的轨道平面朝向。
  mesh.rotation.x = -Math.PI / 2;
  if (planeQuat) mesh.quaternion.premultiply(planeQuat);
  mesh.renderOrder = 3;
  return mesh;
}

// 由 `{inc, node}` 造出该天体轨道平面在世界里的朝向（星环/自转轴都挂它）。
export function planeQuaternion(plane) {
  const q = new THREE.Quaternion();
  if (!plane || !plane.inc) return q;
  // 平面法线：把 +Y 绕交点轴转 inc。
  const axis = new THREE.Vector3(Math.cos(plane.node), 0, Math.sin(plane.node));
  // 交点轴在平面内 → 绕它转 inc 得到倾斜的平面。
  const perp = new THREE.Vector3(-Math.sin(plane.node), 0, Math.cos(plane.node));
  const n = perp.clone().multiplyScalar(Math.sin(plane.inc)).add(new THREE.Vector3(0, Math.cos(plane.inc), 0));
  q.setFromUnitVectors(new THREE.Vector3(0, 1, 0), n.normalize());
  void axis;
  return q;
}
