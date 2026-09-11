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
import { INC } from './glsl.js';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { ShaderPass } from 'three/addons/postprocessing/ShaderPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { FXAAShader } from 'three/addons/shaders/FXAAShader.js';
import { POST_COMMON_GLSL } from './util.js';
import { TUNING } from './tuning.js';

const FULLSCREEN_VERT = INC('px/post/fullscreen.vert');

// ---------------------------------------------------------------------------
// SunFlarePass：体积光 + 各向异性拉丝 + 镜头鬼影
// ---------------------------------------------------------------------------
const SUNFLARE_FRAG = INC('px/post/sunflare.frag');

// ---------------------------------------------------------------------------
// GradePass：色差 + 暗角 + 胶片颗粒（在 sRGB 之后）
// ---------------------------------------------------------------------------
const GRADE_FRAG = INC('px/post/grade.frag');

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
      uThreshold: { value: TUNING.flareThreshold },
      uStreak: { value: TUNING.streakStrength },
      uGhost: { value: TUNING.ghostStrength },
      // 采样数/环数改由 uniform 驱动（原来是 defines，会全展开）。见 SHADER 里的注释。
      uStreakTaps: { value: 12 },
      uGhostCount: { value: 3 },
    },
    vertexShader: FULLSCREEN_VERT,
    fragmentShader: SUNFLARE_FRAG,
    depthTest: false,
    depthWrite: false,
  });
  const sunFlare = new ShaderPass(flareMat);
  sunFlare.enabled = !!tier.flare;
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
      flareMat.uniforms.uThreshold.value = TUNING.flareThreshold;
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
      sunFlare.enabled = on && !!tier.flare;
      flareMat.uniforms.uStreak.value = tier.flare ? TUNING.streakStrength : 0;
      flareMat.uniforms.uGhost.value = tier.flare ? TUNING.ghostStrength : 0;
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
