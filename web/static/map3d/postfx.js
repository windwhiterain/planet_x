// 行星X WebUI — HDR 后处理管线。
//
// 顺序（每一步为什么在这个位置都写在下面）：
//
//   RenderPass          场景 → HalfFloat 线性 HDR render target（可开 MSAA）
//   UnrealBloomPass     阈值以上的高光溢出（太阳、大气边缘、城市灯、引擎羽流）
//   SunFlarePass        体积光(god rays) + 各向异性拉丝 + 镜头鬼影 —— **一次**全屏
//   OutputPass          ACES Filmic 色调映射 + 线性→sRGB（three 官方 pass；它读
//                       `renderer.toneMapping` / `toneMappingExposure` / `outputColorSpace`）
//   GradePass           色差 + 暗角 + 胶片颗粒（**在 sRGB 之后**做：这三个都是「相机/胶片」
//                       的伪影，物理上发生在显示编码之后）
//   FXAA                MSAA 不可用时的兜底抗锯齿（WebGL2 上默认走 MSAA，这一条不启用）
//
// 三个刻意的取舍：
//   * **MSAA 而不是 SMAA**：`EffectComposer` 会吃掉 `WebGLRenderer({antialias:true})`，但
//     r169 的 `WebGLRenderTarget` 支持 `samples`，把带 `samples` 的 target 交给 composer，
//     几何边缘（轨道线、舰体、球体轮廓）就有真 4×MSAA——比后处理抗锯齿干净得多，而且免费。
//   * **太阳的遮挡用解析求交，不用深度纹理**：要接 composer 的 depth texture 得自己搭
//     render target 并保证每个 pass 都不破坏它，脆且难调。改为在 CPU 上拿「相机→太阳」线段
//     与天体显示球求交（同一套 `occluders` 数据，O(天体数)），得到一个可见度标量。
//     效果正是想要的：太阳被行星挡住时，画面里的体积光/拉丝/鬼影一起消失。
//   * **god rays 采的是 bloom 之后的图**：这样拉出来的是「日冕/大气/城市」这些真实的高光，
//     而不是几何体本体——伪影小得多。

import * as THREE from 'three';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { ShaderPass } from 'three/addons/postprocessing/ShaderPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { FXAAShader } from 'three/addons/shaders/FXAAShader.js';
import { POST_COMMON_GLSL } from './util.js';
import { TUNING } from './tuning.js';

const FULLSCREEN_VERT = /* glsl */`
  varying vec2 vUv;
  void main(){
    vUv = uv;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`;

// ---------------------------------------------------------------------------
// SunFlarePass：体积光 + 各向异性拉丝 + 镜头鬼影
// ---------------------------------------------------------------------------
const SUNFLARE_FRAG = /* glsl */`
  precision highp float;
  ${POST_COMMON_GLSL}
  uniform sampler2D tDiffuse;
  uniform vec2  uSun;         // 太阳的屏幕 uv（0..1，可能落在屏幕外）
  uniform float uVis;         // 太阳可见度（被天体挡住 → 0）
  uniform vec2  uTexel;       // 1 / 分辨率
  uniform vec2  uAspect;      // (aspect, 1)
  uniform float uGodStrength, uDecay, uDensity, uWeight, uThreshold;
  uniform float uStreak;
  uniform float uGhost;
  // 采样数 / 环数是 **uniform** 而不是 #define：常量上界会让编译器把循环完全展开
  // （理由与实测数据见 util.js 里 NOISE_GLSL 那段）。这三个都是全屏 pass 的主循环，
  // GODRAY 24 次、STREAK 25 次展开是实打实的编译量。
  uniform int uGodraySamples;
  uniform int uStreakTaps;
  uniform int uGhostCount;
  varying vec2 vUv;

  // 光圈核：一条**柔和的光斑**。
  // 旧版是「同心环 + 极角量化的六边形硬边」：cos(r*22.0) 画出一圈圈离散的环、
  // 极角 floor 出六条硬边，在虚空里读起来是一个**靶子**而不是镜头反射
  // （实测：太阳跑到画面外时，画面另一侧会浮出一整片粉色靶环 —— 用户报的「神秘的点」）。
  // 真实的鬼影是镀膜/光圈叶片的**离焦**像：一个软核加一圈很淡的外晕，没有离散环。
  float aperture(vec2 p){
    float r2 = dot(p, p);
    if (r2 > 1.0) return 0.0;
    float core = exp(-r2 * 4.5);            // 软核
    float halo = 0.16 * exp(-r2 * 1.5);     // 极淡的外晕
    // 边缘窗口保证收到 0，不留硬边（旧版 smoothstep(1.0,0.82) 太窄，等于没淡出）。
    return (core + halo) * smoothstep(1.0, 0.45, sqrt(r2));
  }

  // 只让「真的比白更亮」的东西参与散射（太阳本体 / 它的紧致辉光 / 引擎羽流）。
  // 关键：**不能**用 s * luma(s) —— 那是把亮度平方，HDR 下会瞬间炸到几十，整幅画面洗白。
  float sourceMask(vec3 s){
    return smoothstep(uThreshold, uThreshold * 3.0, luma(s));
  }

  void main(){
    vec4 base = texture2D(tDiffuse, vUv);
    float vis = uVis;
    if (vis <= 0.001) { gl_FragColor = base; return; }

    vec3 flare = vec3(0.0);

    // --- ① 体积光（沿「本像素 → 太阳」的径向散射）---------------------------------
    // delta 是**追到太阳为止**的整段路（乘以 density），所以每个像素的采样点分布相同；
    // 屏幕上的径向梯度来自下面那条 radial 包络，而不是来自采样几何（采样几何本身给不出梯度）。
    // 每条射线 24 次纹理采样，是全屏 pass 里最贵的一项；关掉时（低质量档）必须真的
    // 跳过循环，而不是把结果乘 0 —— 否则「关掉体积光」一点都不会变快。
    if (uGodStrength > 0.0) {
      vec2 delta = (uSun - vUv) * (uDensity / float(uGodraySamples)) * uAspect;
      vec2 uv = vUv;
      float illum = uWeight;
      vec3 acc = vec3(0.0);
      for (int i = 0; i < uGodraySamples; i++) {
        uv += delta;
        vec3 s = texture2D(tDiffuse, uv).rgb;
        acc += s * sourceMask(s) * illum;
        illum *= uDecay;
      }
      // 归一：sum(illum) ≈ uWeight/(1-uDecay)，再乘一个整体的观感系数。
      acc *= uGodStrength * (1.0 - uDecay) * 0.42 / max(uWeight, 1e-4);
      // 包络：集中在太阳附近（有遮挡物时的那道「光柱」就是这个包络被物体切开的样子）。
      float d2s = length((vUv - uSun) * uAspect);
      flare += acc * exp(-d2s * 3.1);
    }

    // --- ② 各向异性拉丝（变形宽银幕镜头那道水平蓝线） -------------------------
    // 采样是**锚在太阳的屏幕上**做的，所以整行的 st 是同一个值——必须再乘一条横向包络，
    // 否则会画出一条贯穿全屏的亮带（踩过这个坑）。
    if (uStreak > 0.0) {
      vec3 st = vec3(0.0);
      float wsum = 0.0;
      for (int i = -uStreakTaps; i <= uStreakTaps; i++) {
        float fi = float(i);
        // 指数核（真正的横向拖尾来自下面那条 hfall，这里只负责取平均）。
        float w = exp(-abs(fi) * 0.16);
        vec2 suv = vec2(uSun.x + fi * uTexel.x * 9.0, uSun.y);
        vec3 s = texture2D(tDiffuse, suv).rgb;
        st += s * sourceMask(s) * w;
        wsum += w;
      }
      st /= max(wsum, 1e-4);
      // 横向：长长的指数尾（这就是「拉丝」）；纵向：一条很窄的包络（不然整行都会被点亮）。
      float hfall = exp(-abs(vUv.x - uSun.x) * uAspect.x * 6.5);
      float vfall = exp(-pow((vUv.y - uSun.y) * uAspect.y * 46.0, 2.0));
      flare += st * uStreak * 0.55 * hfall * vfall * vec3(0.72, 0.84, 1.25);
    }

    // --- ③ 镜头鬼影：沿「太阳 → 屏幕中心」连线的一组彩色光圈 ------------------
    if (uGhost > 0.0) {
      vec2 c = vec2(0.5);
      vec2 axis = c - uSun;
      for (int i = 0; i < uGhostCount; i++) {
        // 每个鬼影在连线上有固定的位置/大小/色相（同一颗镜头，每次看到的都一样）。
        float t = -0.45 + float(i) * 0.42;
        float sc = 0.05 + 0.075 * fract(float(i) * 0.617 + 0.23);
        vec2 gp = (uSun + axis * t - vUv) * uAspect / sc;
        float ap = aperture(gp);
        vec3 tint = 0.5 + 0.5 * cos(vec3(0.0, 2.094, 4.188) + float(i) * 1.7);
        // 鬼影的亮度跟着太阳在画面里的「入射强度」走（越靠边越暗，符合真实镀膜反射）。
        float off = smoothstep(0.0, 0.85, length(uSun - c));
        // **屏幕边缘遮罩**：鬼影是屏幕空间的，画面边框是硬的 —— 不做遮罩它就会被
        // 边框**硬切**，在半空里露出半个靶环。按鬼影自己的屏幕位置（uv 0..1）淡出。
        vec2 ep = uSun + axis * t;
        float edge = smoothstep(0.0, 0.16, ep.x) * smoothstep(1.0, 0.84, ep.x)
                   * smoothstep(0.0, 0.16, ep.y) * smoothstep(1.0, 0.84, ep.y);
        flare += tint * ap * uGhost * (0.35 + 0.65 * off) * 0.5 * edge;
      }
    }

    gl_FragColor = vec4(base.rgb + flare * vis, base.a);
  }
`;

// ---------------------------------------------------------------------------
// GradePass：色差 + 暗角 + 胶片颗粒（在 sRGB 之后）
// ---------------------------------------------------------------------------
const GRADE_FRAG = /* glsl */`
  precision highp float;
  ${POST_COMMON_GLSL}
  uniform sampler2D tDiffuse;
  uniform vec2  uTexel;
  uniform float uCA;        // 色差强度（像素）
  uniform float uVignette;
  uniform float uGrain;
  uniform float uTime;
  varying vec2 vUv;

  void main(){
    vec2 uv = vUv;
    vec2 d = uv - 0.5;
    float r2 = dot(d, d);

    // --- 色差：横向色差随离画面中心的距离平方增长（真实镜头的横向色差就是这个形状）---
    vec2 off = d * r2 * uCA * 4.0 * uTexel;
    vec3 col;
    col.r = texture2D(tDiffuse, uv + off).r;
    col.g = texture2D(tDiffuse, uv).g;
    col.b = texture2D(tDiffuse, uv - off).b;

    // --- 暗角：cos^4 律的廉价近似（真实镜头的自然渐晕，不是「画个黑圈」）---------
    float vig = 1.0 - uVignette * pow(clamp(r2 * 2.0, 0.0, 1.0), 1.45);
    col *= vig;

    // --- 胶片颗粒：暗部多、亮部少（真实银盐颗粒的信噪比特征）-------------------
    if (uGrain > 0.0) {
      float n = grainNoise(uv * vec2(1920.0, 1080.0), fract(uTime) * 91.7) - 0.5;
      float l = luma(col);
      col += n * uGrain * (1.0 - l * 0.75);
    }

    gl_FragColor = vec4(max(col, 0.0), 1.0);
  }
`;

export function createPostFX(renderer, tier, size) {
  const pr = renderer.getPixelRatio();
  const w = Math.max(2, Math.floor(size.w * pr));
  const h = Math.max(2, Math.floor(size.h * pr));

  // 带 MSAA 的 HDR target：composer 会 clone 它，`samples` 会被 copy 过去（r169 的
  // WebGLRenderTarget.copy 抄了 samples），所以 renderTarget1/2 都是多重采样的。
  const rt = new THREE.WebGLRenderTarget(w, h, {
    type: THREE.HalfFloatType,
    format: THREE.RGBAFormat,
    minFilter: THREE.LinearFilter,
    magFilter: THREE.LinearFilter,
    depthBuffer: true,
    stencilBuffer: false,
    samples: tier.msaa || 0,
  });
  rt.texture.colorSpace = THREE.LinearSRGBColorSpace;
  rt.texture.name = 'px-hdr';

  const composer = new EffectComposer(renderer, rt);
  composer.setPixelRatio(pr);
  composer.setSize(size.w, size.h);

  const renderPass = new RenderPass(null, null);
  composer.addPass(renderPass);

  const bloom = new UnrealBloomPass(
    new THREE.Vector2(w, h),
    TUNING.bloomStrength, TUNING.bloomRadius, TUNING.bloomThreshold,
  );
  bloom.enabled = !!tier.bloom;
  composer.addPass(bloom);

  const flareMat = new THREE.ShaderMaterial({
    uniforms: {
      tDiffuse: { value: null },
      uSun: { value: new THREE.Vector2(0.5, 0.5) },
      uVis: { value: 1 },
      uTexel: { value: new THREE.Vector2(1 / w, 1 / h) },
      uAspect: { value: new THREE.Vector2(1, 1) },
      uGodStrength: { value: TUNING.godrayStrength },
      uDecay: { value: TUNING.godrayDecay },
      uDensity: { value: TUNING.godrayDensity },
      uWeight: { value: TUNING.godrayWeight },
      uThreshold: { value: TUNING.godrayThreshold },
      uStreak: { value: TUNING.streakStrength },
      uGhost: { value: TUNING.ghostStrength },
      // 采样数/环数改由 uniform 驱动（原来是 defines，会全展开）。见 SHADER 里的注释。
      uGodraySamples: { value: Math.max(6, TUNING.godraySamples | 0) },
      uStreakTaps: { value: 12 },
      uGhostCount: { value: 3 },
    },
    vertexShader: FULLSCREEN_VERT,
    fragmentShader: SUNFLARE_FRAG,
    depthTest: false,
    depthWrite: false,
  });
  const sunFlare = new ShaderPass(flareMat);
  sunFlare.enabled = !!(tier.godrays || tier.flare);
  composer.addPass(sunFlare);

  const output = new OutputPass();
  composer.addPass(output);

  const gradeMat = new THREE.ShaderMaterial({
    uniforms: {
      tDiffuse: { value: null },
      uTexel: { value: new THREE.Vector2(1 / w, 1 / h) },
      uCA: { value: TUNING.caStrength },
      uVignette: { value: TUNING.vignette },
      uGrain: { value: TUNING.grain },
      uTime: { value: 0 },
    },
    vertexShader: FULLSCREEN_VERT,
    fragmentShader: GRADE_FRAG,
    depthTest: false,
    depthWrite: false,
  });
  const grade = new ShaderPass(gradeMat);
  grade.enabled = !!tier.grade;
  composer.addPass(grade);

  // FXAA 只在没有 MSAA 的档位上兜底（低档）。它是**最后**一道，作用在已编码的 sRGB 上。
  const fxaa = new ShaderPass(FXAAShader);
  fxaa.enabled = !(tier.msaa > 0);
  fxaa.material.uniforms.resolution.value.set(1 / w, 1 / h);
  composer.addPass(fxaa);

  let curW = w, curH = h;

  return {
    composer,
    passes: { renderPass, bloom, sunFlare, output, grade, fxaa },

    setScene(scene, camera) {
      renderPass.scene = scene;
      renderPass.camera = camera;
    },

    setSize(width, height) {
      composer.setPixelRatio(renderer.getPixelRatio());
      composer.setSize(width, height);
      const pr2 = renderer.getPixelRatio();
      curW = Math.max(2, Math.floor(width * pr2));
      curH = Math.max(2, Math.floor(height * pr2));
      bloom.setSize(curW, curH);
      flareMat.uniforms.uTexel.value.set(1 / curW, 1 / curH);
      flareMat.uniforms.uAspect.value.set(curW / Math.max(curH, 1), 1);
      gradeMat.uniforms.uTexel.value.set(1 / curW, 1 / curH);
      fxaa.material.uniforms.resolution.value.set(1 / curW, 1 / curH);
    },

    // 每帧由 index.js 调：太阳的屏幕位置 + 可见度（解析求交的结果）+ 时间（颗粒用）。
    setSun(screenX, screenY, visibility) {
      flareMat.uniforms.uSun.value.set(screenX, screenY);
      flareMat.uniforms.uVis.value = visibility;
    },

    setTime(t) {
      gradeMat.uniforms.uTime.value = t;
      // 颗粒每帧都要抖：three 的 ShaderPass 不会因为 uniform 变化重编，直接改值即可。
    },

    // 把 `TUNING` 的当前值重新灌进各个 pass。**必须**有这一条：uniform 是在
    // `createPostFX` 时从 TUNING 拷进去的，之后 `PlanetXMap.tune({...})` 改 TUNING
    // 对已经建好的 pass **毫无影响**——现象是「调了参数画面没变」，很容易误判成
    // 「这个参数没用」（本会话就因此做过一次无效的 A/B 对照）。
    applyTuning() {
      bloom.strength = TUNING.bloomStrength;
      bloom.radius = TUNING.bloomRadius;
      bloom.threshold = TUNING.bloomThreshold;
      flareMat.uniforms.uGodStrength.value = TUNING.godrayStrength;
      flareMat.uniforms.uDecay.value = TUNING.godrayDecay;
      flareMat.uniforms.uDensity.value = TUNING.godrayDensity;
      flareMat.uniforms.uWeight.value = TUNING.godrayWeight;
      flareMat.uniforms.uThreshold.value = TUNING.godrayThreshold;
      flareMat.uniforms.uStreak.value = TUNING.streakStrength;
      flareMat.uniforms.uGhost.value = TUNING.ghostStrength;
      gradeMat.uniforms.uCA.value = TUNING.caStrength;
      gradeMat.uniforms.uVignette.value = TUNING.vignette;
      gradeMat.uniforms.uGrain.value = TUNING.grain;
    },

    // 质量档切换：只改「开不开/多少个 tap」，不重建 composer（重建会漏资源）。
    setTier(tier) {
      const on = TUNING.postfx !== false;
      bloom.enabled = on && !!tier.bloom;
      sunFlare.enabled = on && !!(tier.godrays || tier.flare);
      flareMat.uniforms.uStreak.value = tier.flare ? TUNING.streakStrength : 0;
      flareMat.uniforms.uGhost.value = tier.flare ? TUNING.ghostStrength : 0;
      flareMat.uniforms.uGodStrength.value = tier.godrays ? TUNING.godrayStrength : 0;
      grade.enabled = on && !!tier.grade;
      fxaa.enabled = !(tier.msaa > 0);
    },

    setExposure(e) {
      renderer.toneMappingExposure = e;
    },

    render(dt) {
      composer.render(dt);
    },

    dispose() {
      composer.dispose();
      bloom.dispose();
      sunFlare.dispose();
      grade.dispose();
      fxaa.dispose();
      rt.dispose();
    },
  };
}
