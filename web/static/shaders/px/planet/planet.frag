#ifndef PX_PLANET_PLANET_FRAG
#define PX_PLANET_PLANET_FRAG
#include <px/planet/common.glsl>
#include <px/noise/fbm.glsl>
#include <px/noise/ridged.glsl>
#include <px/noise/warpT.glsl>

  precision highp float;


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
  // ---- 程序化表面参数（config 的 body_kinds[].params）--------------------------------
  // 名字由前端 paramUniforms() 从字段名自动派生（band_freq ⇒ uBandFreq），
  // 所以这里**必须**与 Rust SurfaceParams 各结构体的字段名逐字对应。
  // 每个类只用自己那几个；多余的声明是无害的（three.js 只上传程序里存在的 uniform）。
  uniform float uBandFreq;        // Gas / IceGiant
  uniform float uBandDetail;      // Gas
  uniform float uBandContrast;    // Gas / IceGiant
  uniform float uBandWeight;      // IceGiant
  uniform float uShear;           // Gas
  uniform float uTurbulence;      // Gas / IceGiant / Venus
  uniform float uPolar;           // Gas
  uniform int   uStormCount;      // Gas
  uniform float uStormSize;       // Gas
  uniform float uStormStrength;   // Gas
  uniform float uHaze;            // Gas / IceGiant / Titan
  uniform float uSpot;            // IceGiant
  uniform float uMottle;          // Rock / Lunar / Dwarf / IceWorld
  uniform float uPolarCap;        // Terran / Martian / Rock / Lunar / Dwarf
  uniform float uReliefShade;     // Rock / Lunar / Dwarf
  uniform float uCraterDensity;   // Rock / Lunar / Dwarf / IceWorld
  uniform float uArid;            // Terran
  uniform float uBiome;           // Terran
  uniform float uDetail;          // Terran
  uniform float uDarkRegion;      // Martian
  uniform float uSwirlFreq;       // Venus
  uniform float uSwirlAmt;        // Venus
  uniform float uStreak;          // Venus
  uniform float uCrackFreq;       // IceWorld
  uniform float uCrackWidth;      // IceWorld
  uniform float uCrackAmount;     // IceWorld
  uniform float uLake;            // Titan
  uniform float uLakePolar;       // Titan
  // 星环在该行星上的投影（环平面 = 过球心的平面，法线 uRingN，半径区间 [uRingIn, uRingOut]）。
  uniform float uHasRing;
  uniform vec3  uRingN;
  uniform float uRingIn;
  uniform float uRingOut;
  uniform vec3  uCityDirs[CITY_MAX];
  uniform int   uCityCount;      // 实际有城的槽数（循环上界；CITY_MAX 只是数组容量）
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
    landC = mix(landC, vec3(0.58, 0.44, 0.22), arid * uArid);
    // 高频细节：没有它，大陆就是一整块均匀的绿（「绿色大理石」）。同时按第二层噪声
    // 做生物群系扰动，让同一纬度上也有深浅差别。
    float detail = fbm(ds * 16.0 + 2.2);
    float biome = fbm(ds * 6.5 + 19.0);
    landC *= 0.78 + uDetail * detail;
    landC = mix(landC, landC * vec3(1.18, 1.05, 0.82), smoothstep(0.40, 0.72, biome) * uBiome);
    // 极冠：噪声破边，别是一条平滑的纬度线。polar_cap 越大极冠越往低纬长。
    float capNoise = fbm(ds * 5.0 + 3.3) * 0.14;
    float pole = smoothstep(uPolarCap - capNoise, uPolarCap + 0.13 - capNoise, lat);
    vec3 col = mix(ocean, landC, land);
    col = mix(col, vec3(0.93, 0.96, 0.99), pole * (0.55 + 0.45 * land));
    return col;
  }

  vec3 gasColor(vec3 ds, float lat){
    // 纬向急流：把纬度当主变量，用域扰动让每条带自己是湍流的。
    // ⚠ 剪切量（shear 乘在纬向上）要**小**：原来乘 5.0/9.0，相位被扰动到 ±5 rad，
    // 纬向条带被撕成一坨坨斑块，看起来不像气巨。真实的气巨带纹是**强纬向、弱经向**的，
    // 湍流只把边界揉皱，不该把带揉没。
    // turbulence 走 warpT：它自动保住 amt·warpK 的折叠安全乘积（见 util.js::warp），
    // 所以可以在 config 里自由调大而不会重新引入折痕。
    vec3 w = warpT(ds * 3.0, 0.36, 0.83, 0.0, uTurbulence);
    float shear = fbm(w * 2.2) * 0.5 * uShear;
    // 三层不同频率的纬向带 + 少量剪切，叠出「宽带里套细纹」的层次。
    // band_detail 是 2/3 层的权重：1.0 = 原口径 0.50/0.32/0.18，0 = 只剩基频
    // ⇒ 土星给 0.30 就是「带疏而柔」，木星给 1.0 是「宽带里套细纹」。
    float b1 = sin(lat * uBandFreq + shear * 1.0);
    float b2 = sin(lat * uBandFreq * 2.25 + shear * 1.8);
    float b3 = sin(lat * uBandFreq * 4.33 + shear * 2.9);
    float bd = clamp(uBandDetail, 0.0, 1.0);
    float band = 0.5 + 0.5 * mix(b1, 0.50 * b1 + 0.32 * b2 + 0.18 * b3, bd);
    // band_contrast：1.0 = 原窗口 (0.26, 0.80) 那种分明，0 = 近乎均匀的一颗球。
    float half_ = mix(0.44, 0.27, clamp(uBandContrast, 0.0, 1.0));
    band = smoothstep(0.53 - half_, 0.53 + half_, band);
    // 两端各自再推开：config 给的 base/accent 往往只差一点点（土星的驼 vs 暗驼），
    // 直接 mix 出来是一条没有对比的色带。
    vec3 c = mix(uAccent * 0.58, uBase * 1.16, band);
    // 带内细流：真实气巨的每一条带自己都是湍流的，不是一条均匀色条。
    c *= 0.90 + 0.20 * fbm(w * 7.0 + 2.0);
    // 极区涡旋：高纬处收紧、变暗（木星/土星的极地都是暗的）。polar 越大压得越宽。
    float polar = smoothstep(1.0 - clamp(uPolar, 0.0, 1.0) * 0.40, 1.0, abs(lat));
    c = mix(c, uAccent * 0.55, polar * 0.7);
    // 风暴：椭圆涡旋（第 0 颗是「大红斑」那种最大的）。storm_count = 0 就是一颗都没有
    // —— 土星本体确实没有大红斑那种显眼的风暴。
    const vec3 STORM[3] = vec3[3](vec3(0.55, -0.18, 0.62), vec3(-0.72, 0.26, 0.42), vec3(0.12, 0.44, -0.88));
    for (int i = 0; i < uStormCount; i++) {
      vec3 sd = normalize(STORM[i]);
      float dd = length(ds - sd);
      float spot = exp(-pow(dd / (uStormSize * (1.0 + 0.38 * float(i))), 2.0));
      // 涡旋内部有环流纹理，不是一块纯色补丁。
      float swirl = 0.75 + 0.25 * sin(dd * 60.0 + fbm(ds * 8.0) * 6.0);
      vec3 sc = (i == 0) ? vec3(0.66, 0.30, 0.17) : mix(uAccent, uBase, 0.35) * 0.8;
      c = mix(c, sc, spot * swirl * clamp(uStormStrength, 0.0, 1.0));
    }
    // 雾霾：往 base/accent 的中间色压，读起来更朦胧、对比更低（土星 > 木星）。
    c = mix(c, mix(uBase, uAccent, 0.5) * 1.05, clamp(uHaze, 0.0, 1.0) * 0.55);
    return c;
  }

  vec3 icyColor(vec3 ds, float lat){
    if (uBanded > 0.5) {
      // 冰巨星：带极淡、极细，整体偏青蓝。**天王星和海王星的区别全在 config 的
      // IceGiant 参数里** —— 天王星 band_contrast 近 0 + haze 高（真实天王星就是
      // 一颗几乎没有细节的青球），海王星带纹明显 + spot 给出大暗斑。
      vec3 w = warpT(ds * 3.0, 0.36, 0.83, 0.0, uTurbulence);
      float band = 0.5 + 0.5 * sin(lat * uBandFreq + fbm(w * 1.8) * 2.2);
      // band_contrast = 1.0 精确复现原来的 smoothstep(0.34, 0.82)。
      float half_ = mix(0.44, 0.24, clamp(uBandContrast, 0.0, 1.0));
      band = smoothstep(0.58 - half_, 0.58 + half_, band);
      float bw = clamp(uBandWeight, 0.0, 1.0);
      vec3 c = mix(uAccent, uBase, band * bw + (1.0 - bw) * 0.5);
      // 暗斑（海王星的大暗斑）：一颗高纬的暗涡。spot = 0 就整段跳过。
      if (uSpot > 0.001) {
        vec3 sd = normalize(vec3(0.62, -0.42, 0.66));
        float dd = length(ds - sd);
        float s = exp(-pow(dd / 0.17, 2.0));
        c = mix(c, uAccent * 0.42, s * clamp(uSpot, 0.0, 1.0));
      }
      // 雾霾：整体去饱和并向基色靠 —— 天王星那层厚雾把细节全糊掉。
      float lum = dot(c, vec3(0.299, 0.587, 0.114));
      c = mix(c, vec3(lum) * 0.9 + uBase * 0.25, clamp(uHaze, 0.0, 1.0) * 0.6);
      return c;
    }
    // 冰封卫星：光滑冰面 + 细裂纹 + 少量坑。
    float var = fbm(ds * 2.4);
    vec3 c = mix(uBase, uAccent, var * uMottle);
    float crack = abs(fbm(ds * uCrackFreq) - 0.5);
    // crack_width = 0.08 精确复现原来的 smoothstep(0.14, 0.06, ...)。
    c = mix(c, vec3(0.86, 0.94, 1.02),
            smoothstep(uCrackWidth * 1.75, uCrackWidth * 0.75, crack) * uCrackAmount);
    c += craterField(ds, 5.0, uCraterDensity) * 0.10;
    return c;
  }

  vec3 rockColor(vec3 ds, float lat, float h){
    float var = fbm(ds * 3.0);
    vec3 c = mix(uBase, uAccent, var * uMottle);
    if (uClass == 3) {
      // 火星：红荒漠 + 暗反照率区（Syrtis Major 那种）+ 极冠。
      c = mix(c, uAccent * 1.05, smoothstep(0.42, 0.72, fbm(ds * 2.1 + 5.0)) * uDarkRegion);
      float capN = fbm(ds * 4.5) * 0.10;
      float pole = smoothstep(uPolarCap - capN, uPolarCap + 0.11 - capN, lat);
      c = mix(c, vec3(0.94, 0.95, 0.96), pole * 0.85);
    } else {
      float capN = fbm(ds * 5.0) * 0.10;
      float pole = smoothstep(uPolarCap - capN, uPolarCap + 0.09 - capN, lat);
      c = mix(c, vec3(0.88, 0.90, 0.94), pole * 0.6);
    }
    // 高度分层：高处长亮、洼地压暗（月海就是「洼地压暗」，所以和岩石共用同一个字段）。
    c *= 0.82 + uReliefShade * smoothstep(-0.1, 0.7, h);
    // 陨石坑：岩石/卫星/矮行星都有，密度由 config 的 crater_density 给
    // （卫星最多、矮行星次之、行星最少）。没有它，这几类只能靠颜色区分。
    c += craterField(ds, 4.6, uCraterDensity * 0.55) * 0.10;
    return c;
  }

  vec3 titanColor(vec3 ds, float lat){
    float latT = clamp((lat + 1.0) * 0.5, 0.0, 1.0);
    vec3 c = mix(uAccent, uBase, latT);
    // 甲烷湖：极区少量暗斑。lake_polar 越大湖越集中在极点。
    float lake = smoothstep(0.72, 0.9, fbm(ds * 3.4 + 9.0))
               * smoothstep(uLakePolar, uLakePolar + 0.35, abs(ds.y));
    c = mix(c, vec3(0.08, 0.10, 0.12), lake * uLake);
    // 雾霾：土卫六整颗是橙色的霾，几乎看不到地表。
    c = mix(c, mix(uBase, uAccent, 0.5) * 1.05, clamp(uHaze, 0.0, 1.0) * 0.6);
    return c;
  }

  vec3 venusColor(vec3 ds, float lat){
    // 浓厚硫磺云：**高速**纬向涡旋（金星大气 4 天绕一圈，是最有辨识度的特征）。
    // 用强域扰动把纬向条纹撕成 Y 形/涡卷，而不是一张均匀的驼色纸。
    vec3 w = warpT(ds * 6.0 + vec3(0.0, lat * 2.0, 0.0), 0.71, 0.42, 0.0, uTurbulence);
    float s = fbm(w * 2.6);
    float swirl = 0.5 + 0.5 * sin(lat * uSwirlFreq + s * uSwirlAmt);
    float streak = fbm(vec3(ds.x * 3.0, ds.y * 16.0, ds.z * 3.0));
    vec3 c = mix(uAccent, uBase, smoothstep(0.15, 0.85, s * 0.62 + swirl * 0.38));
    c *= 0.86 + uStreak * streak;
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
      // ⚠ 必须和上面那条分支一样**转回对象空间**再乘 vObjToWorld。
      // ds 是「已经反转了 uSpin」的采样方向，直接当对象空间用会让气巨的法线整体多转
      // 一个 -uSpin；而 index.js 每帧都在推进 uSpin ⇒ **明暗交界线跟着自转一起转**，
      // 看上去就是「光源方向在自转」（用户实测：木星、土星最明显，正因为它们是气巨）。
      nSurface = rotAxis(normalize(ds - bump * uRelief * 0.05), vec3(0.0, 1.0, 0.0), uSpin);
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
        // 循环上界用 **uniform**（真实城数）而不是数组容量 CITY_MAX：常量上界会把
        // 12 次迭代全部展开；而且用真实数量还**顺带省掉空槽的判定**。
        for (int i = 0; i < uCityCount; i++) {
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
#endif // PX_PLANET_PLANET_FRAG
