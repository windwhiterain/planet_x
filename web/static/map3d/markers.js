// 行星X WebUI — UI 标记层：恒定屏幕尺寸的 billboard / 阵营准星环 / 天体标签。
//
// 三条从旧版继承、**不要推翻**的原则：
//   1. **恒定屏幕尺寸**：所有标记每帧按相机距离换算世界尺寸（`世界长度 = k·距离·像素`），
//      所以拉远拉近标记不会忽大忽小。
//   2. **`depthTest:false` + 解析式遮挡**：标记若参与深度测试会被球面切边、被自己的行星地面
//      吃掉。所以一律不测深度，改为每帧对「相机→标记中心」这条线段与天体显示球求交，
//      用**背面深度**做连续淡出（`depth≤0` 在轮廓外、`0` 正好压在轮廓上、越大越深入球体背面）。
//      这才是空间感的来源——空间感靠这个，不靠 z-buffer。
//   3. **阵营色只出现在 UI 层**：准星环 + 标签 chip 前的色点。模型一律中性灰。

import * as THREE from 'three';
import { TUNING } from './tuning.js';

const LABEL_SS = 2;        // canvas 超采样倍率（HiDPI 下文字才不糊）

// --- 形状贴图 ---------------------------------------------------------------
const shapeTex = {};
function makeShapeTexture(shape) {
  const S = 128;
  const c = document.createElement('canvas');
  c.width = c.height = S;
  const ctx = c.getContext('2d');
  ctx.clearRect(0, 0, S, S);
  ctx.fillStyle = '#ffffff';
  ctx.strokeStyle = '#ffffff';
  const m = S / 2;
  if (shape === 'dot') {
    // 中心实心 + 外面一圈很淡的晕（远景点也因此能被 bloom 拎出来）。
    const g = ctx.createRadialGradient(m, m, 0, m, m, S * 0.5);
    g.addColorStop(0, 'rgba(255,255,255,1)');
    g.addColorStop(0.42, 'rgba(255,255,255,0.55)');
    g.addColorStop(0.62, 'rgba(255,255,255,0.10)');
    g.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = g;
    ctx.beginPath(); ctx.arc(m, m, S * 0.5, 0, Math.PI * 2); ctx.fill();
  } else if (shape === 'diamond') {
    ctx.beginPath();
    ctx.moveTo(m, S * 0.18); ctx.lineTo(S * 0.82, m); ctx.lineTo(m, S * 0.82); ctx.lineTo(S * 0.18, m);
    ctx.closePath(); ctx.fill();
  } else if (shape === 'reticle') {
    // 阵营准星环：细圆环 + 四个短刻度，中间留空给模型自己。
    ctx.lineWidth = S * 0.045;
    ctx.beginPath(); ctx.arc(m, m, S * 0.36, 0, Math.PI * 2); ctx.stroke();
    ctx.lineWidth = S * 0.055;
    for (let i = 0; i < 4; i++) {
      const a = i * Math.PI / 2 + Math.PI / 4;
      ctx.beginPath();
      ctx.moveTo(m + Math.cos(a) * S * 0.40, m + Math.sin(a) * S * 0.40);
      ctx.lineTo(m + Math.cos(a) * S * 0.46, m + Math.sin(a) * S * 0.46);
      ctx.stroke();
    }
  } else if (shape === 'select') {
    // 选中环：四段圆弧（缺口在正上下左右，读起来像取景框而不是一个圈）。
    ctx.lineWidth = S * 0.05;
    for (let i = 0; i < 4; i++) {
      const a0 = i * Math.PI / 2 + 0.30;
      ctx.beginPath();
      ctx.arc(m, m, S * 0.44, a0, a0 + Math.PI / 2 - 0.60);
      ctx.stroke();
    }
    // 四角刻度。
    ctx.lineWidth = S * 0.06;
    for (let i = 0; i < 4; i++) {
      const a = i * Math.PI / 2;
      ctx.beginPath();
      ctx.moveTo(m + Math.cos(a) * S * 0.30, m + Math.sin(a) * S * 0.30);
      ctx.lineTo(m + Math.cos(a) * S * 0.52, m + Math.sin(a) * S * 0.52);
      ctx.stroke();
    }
  }
  const tex = new THREE.CanvasTexture(c);
  tex.userData.shared = true;   // 多个标记共享，不随单个释放销毁
  tex.anisotropy = 4;
  tex.needsUpdate = true;
  return tex;
}
function shapeTexture(shape) {
  return shapeTex[shape] || (shapeTex[shape] = makeShapeTexture(shape));
}

// --- 标签 chip --------------------------------------------------------------
// 半透明深色圆角底 + 白字；名字前可带阵营色小色块（天体是某势力首都时用）。
// 返回 { sprite, px, aspect }：`px` 是**期望的屏幕高度**，真 scale 每帧由 update() 换算。
export function makeLabel(text, color = '#dbe6ff', fontPx = 13, dots = []) {
  const padX = 8, padY = 5, dotR = 3.2, dotGap = 6;
  const font = `600 ${fontPx * LABEL_SS}px system-ui, "Segoe UI", "Microsoft YaHei", sans-serif`;
  const c = document.createElement('canvas');
  let ctx = c.getContext('2d');
  ctx.font = font;
  const textW = ctx.measureText(text).width;
  const dotW = dots.length ? dots.length * (dotR * 2 * LABEL_SS + dotGap * LABEL_SS) : 0;
  const w = Math.ceil(textW + dotW) + padX * 2 * LABEL_SS;
  const h = Math.ceil(fontPx * 1.34 * LABEL_SS) + padY * 2 * LABEL_SS;
  c.width = w; c.height = h;
  ctx = c.getContext('2d');            // 改尺寸会重置 context 状态，必须重设
  ctx.font = font;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  const r = h / 2;
  // 底：圆角深色 chip（带一点由内向外变淡的渐变，压在任何行星上都读得清）。
  const grad = ctx.createLinearGradient(0, 0, 0, h);
  grad.addColorStop(0, 'rgba(12,18,34,0.74)');
  grad.addColorStop(1, 'rgba(4,7,16,0.66)');
  ctx.beginPath();
  ctx.moveTo(r, 0); ctx.lineTo(w - r, 0); ctx.arc(w - r, r, r, -Math.PI / 2, Math.PI / 2);
  ctx.lineTo(r, h); ctx.arc(r, r, r, Math.PI / 2, -Math.PI / 2);
  ctx.closePath();
  ctx.fillStyle = grad;
  ctx.fill();
  ctx.strokeStyle = 'rgba(255,255,255,0.13)';
  ctx.lineWidth = 1 * LABEL_SS;
  ctx.stroke();
  let x = padX * LABEL_SS;
  dots.forEach((d) => {
    ctx.beginPath();
    ctx.arc(x + dotR * LABEL_SS, h / 2, dotR * LABEL_SS, 0, Math.PI * 2);
    ctx.fillStyle = d;
    ctx.fill();
    x += dotR * 2 * LABEL_SS + dotGap * LABEL_SS;
  });
  ctx.fillStyle = color;
  ctx.fillText(text, x, h / 2 + LABEL_SS);
  const tex = new THREE.CanvasTexture(c);
  tex.minFilter = THREE.LinearFilter;
  tex.userData.shared = false;
  const mat = new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false, depthWrite: false });
  const sp = new THREE.Sprite(mat);
  return { sprite: sp, px: h / LABEL_SS, aspect: w / h };
}

// ---------------------------------------------------------------------------
export function createMarkers() {
  let lodItems = [];
  let labelItems = [];
  let occluders = [];
  const _d = new THREE.Vector3();
  const _v = new THREE.Vector3();

  // 解析式遮挡：把「相机 → 标记中心」这条线段与每个天体显示球求交。
  // depth ∈ (-∞, 1]：≤0 = 中心点在轮廓外（看得见）；0 = 正好压在轮廓边缘；越大 = 越深入背面。
  // (r - 垂距)/r 对球体是精确的「背面深度」，而且**连续** ⇒ 沿 [lo, hi] 淡出不会有跳变。
  function occlAlpha(pos) {
    if (!occluders.length) return 1;
    _d.copy(pos).sub(camera0.position);
    const tMax = _d.length();
    if (tMax < 1e-6) return 1;
    _d.divideScalar(tMax);
    let depth = -Infinity;
    for (let i = 0; i < occluders.length; i++) {
      const o = occluders[i];
      _v.copy(o.center).sub(camera0.position);
      const tc = _v.dot(_d);
      if (tc <= 0 || tc >= tMax) continue;          // 球体在标记之后 → 挡不住
      const d2 = _v.lengthSq() - tc * tc;
      const r2 = o.radius * o.radius;
      if (d2 >= r2) continue;                        // 光线从旁边擦过去
      const d = (o.radius - Math.sqrt(Math.max(d2, 0))) / o.radius;
      if (d > depth) depth = d;
    }
    if (depth === -Infinity) return 1;
    return 1 - THREE.MathUtils.smoothstep(depth, TUNING.occlFadeLo, TUNING.occlFadeHi);
  }

  let camera0 = null;

  function sprite(shape, colorHex, px, opacity = 1, aspect = 1) {
    const mat = new THREE.SpriteMaterial({
      map: shapeTexture(shape),
      color: new THREE.Color(colorHex || '#ffffff'),
      transparent: true,
      opacity,
      depthTest: false,
      depthWrite: false,
    });
    const sp = new THREE.Sprite(mat);
    return { sp, px, aspect, baseOpacity: opacity, always: true, fitWorld: 0 };
  }

  // 一个城市/舰标记 = 中性 3D 模型（近景）+ 阵营色准星环（常显）+ 中性远景点（远景观）
  //              + 选中环（只在被选中时出现）。
  function add(group, opts) {
    const g = new THREE.Group();
    g.userData = { kind: opts.kind, name: opts.name };
    if (opts.model) {
      opts.model.position.copy(opts.local);
      // 姿态：整颗天体已经乘过自己的轨道平面 + 轴倾角，所以这里拿到的 `orient` 已经是
      // **世界空间**的方向。默认把模型的 +Y 对上去（地面城要沿地表法线立起来）；
      // 舰的船体长轴在 +Z，所以传 orientAxis:'z'。
      if (opts.orient) {
        const up = (opts.orientAxis === 'z') ? new THREE.Vector3(0, 0, 1) : new THREE.Vector3(0, 1, 0);
        opts.model.quaternion.setFromUnitVectors(up, new THREE.Vector3(...opts.orient).normalize());
      }
      g.add(opts.model);
    }
    const sprites = [];
    const ret = sprite('reticle', opts.color, TUNING.reticlePx, 0.85);
    ret.fitWorld = opts.modelSize || 0;
    ret.sp.position.copy(opts.local);
    g.add(ret.sp);
    sprites.push(ret);

    const dot = sprite(opts.shape, '#eef2f8', opts.px, 1);
    dot.always = false;
    dot.sp.position.copy(opts.local);
    g.add(dot.sp);
    sprites.push(dot);

    const sel = sprite('select', opts.color, TUNING.reticlePx * 2.6, 0.0);
    sel.fitWorld = (opts.modelSize || 0) * 2.4;
    sel.sp.position.copy(opts.local);
    g.add(sel.sp);
    sprites.push(sel);

    g.position.copy(opts.center);
    group.add(g);
    const item = {
      group: g,
      mesh: opts.model || null,
      sprites, ret, dot, sel,
      name: opts.name,
      pos: opts.world.clone(),
      switchDist: opts.switchDist,
      fade: opts.fade !== false,
      selected: false,
      hovered: false,
      pulse: 0,
      spin: opts.spin || null,
    };
    lodItems.push(item);
    return item;
  }

  function addLabel(spriteObj, center, radius) {
    // 注意字段名：`makeLabel` 给的是 `{sprite, px, aspect}`，而这里下游统一用 `sp` 访问
    // Sprite（与城市/舰标记的 `sprites[].sp` 同一口径）。
    const item = {
      sp: spriteObj.sprite, px: spriteObj.px, aspect: spriteObj.aspect,
      center: center.clone(), radius,
    };
    labelItems.push(item);
    return item;
  }

  // 每帧：按相机距离切换「近景模型 / 远景 UI 精灵」、按固定像素换算世界尺寸、按遮挡淡出。
  function update(camera, renderer, t) {
    camera0 = camera;
    if (!camera || !renderer) return;
    const fov = camera.fov * Math.PI / 180;
    const vh = renderer.domElement.clientHeight || 600;
    const k = 2 * Math.tan(fov / 2) / vh;               // 世界长度 = k · 距离 · 像素

    for (const it of lodItems) {
      const dist = camera.position.distanceTo(it.pos);
      const wpp = k * dist;
      const near = dist <= (it.switchDist || 30);
      const a = it.fade === false ? 1 : occlAlpha(it.pos);
      if (it.mesh) it.mesh.visible = near;
      // 空间站的自转环。
      if (it.spin) it.spin.rotation.y = t * 0.35;
      for (const s of it.sprites) {
        const isSel = s === it.sel;
        if (isSel && !it.selected) { s.sp.visible = false; continue; }
        const vis = (s.always || !near) && a > 0.012;
        s.sp.visible = vis;
        if (!vis) continue;
        // 准星环在近景贴住模型大小（否则拉近后模型会捅出环外）。
        let px = s.fitWorld ? Math.min(Math.max(s.fitWorld / wpp + 9, s.px), 72) : s.px;
        if (isSel) px = (s.fitWorld ? Math.min(Math.max(s.fitWorld / wpp + 9, s.px), 120) : s.px);
        // 选中环缓慢自转 + 呼吸。
        if (isSel) { s.sp.material.rotation = t * 0.5; px *= 1.0 + 0.035 * Math.sin(t * 2.2); }
        s.sp.scale.set(px * wpp * s.aspect, px * wpp, 1);
        let op = s.baseOpacity * a;
        if (s === it.ret && it.hovered) op = Math.min(1, op * 1.7 + 0.25);
        if (isSel) op = Math.min(1, a * (0.75 + 0.25 * Math.sin(t * 2.2)));
        s.sp.material.opacity = op;
      }
    }

    // 标签：恒定像素尺寸 + 始终浮在天体上方（间距也按像素算）+ 同一套遮挡淡出
    // （用标签自身位置判定，否则会被自己的行星永远挡住）。
    for (const lb of labelItems) {
      const wpp = k * camera.position.distanceTo(lb.center);
      lb.sp.position.set(
        lb.center.x,
        lb.center.y + lb.radius + TUNING.labelGapPx * wpp,
        lb.center.z,
      );
      const a = occlAlpha(lb.sp.position);
      lb.sp.visible = a > 0.012;
      if (!lb.sp.visible) continue;
      lb.sp.scale.set(lb.px * wpp * lb.aspect, lb.px * wpp, 1);
      lb.sp.material.opacity = a;
    }
  }

  return {
    add,
    addLabel,
    update,
    setOccluders(list) { occluders = list; },
    get items() { return lodItems; },
    get labels() { return labelItems; },
    select(name, kind) {
      let hit = null;
      for (const it of lodItems) {
        const on = (kind == null || it.group.userData.kind === kind) && it.group.userData.name === name;
        it.selected = on;
        if (on) hit = it;
      }
      return hit;
    },
    hover(name, kind) {
      for (const it of lodItems) {
        it.hovered = (name != null) && (kind == null || it.group.userData.kind === kind) && it.group.userData.name === name;
      }
    },
    clear() { lodItems = []; labelItems = []; },
    count() { return { lod: lodItems.length, labels: labelItems.length, occluders: occluders.length }; },
  };
}

// 拿来做拾取的对象列表：标记的模型（Mesh）而不是 sprite（sprite 不参与 raycast 命中判定，
// 它们没有厚度，而且 `depthTest:false` 会让「点到标签上」也选中——旧版就是这个行为）。
export function pickTargets(groups) {
  return groups;
}
