#ifndef PX_SUN_CORONA_FRAG
#define PX_SUN_CORONA_FRAG
#include <px/noise/fbm.glsl>

  precision highp float;

  uniform float uTime;
  uniform float uSunR;        // 光球半径（世界单位）
  uniform float uOuter;       // 体积外半径（世界单位）
  uniform float uOuterRatio;  // uOuter / uSunR（外缘平滑归零用）
  uniform float uIntensity;
  // 色球的**独立强度**。色球比日冕亮约 1e4 倍 —— 把两者塞进同一个密度场、同一个
  // 强度，日缘就永远填不满（实测盘缘 200、紧挨着外面只有 143，一道台阶）。
  uniform float uChromo;
  // three 只在**顶点**前缀里给 projectionMatrix，片元里没有 —— 所以要自己传进来
  // （viewMatrix 片元里有）。用来把体积的深度报成**近壁**，见下面 gl_FragDepth。
  uniform mat4 uProj;
  // 场景深度（来自 postfx 的深度预趟）。**体积积分靠它夹断** —— 见 main 里那段说明。
  uniform sampler2D uDepth;
  uniform vec2 uResolution;
  uniform vec2 uNearFar;
  uniform float uHasDepth;
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

  // 密度场 —— **日珥**，不是一团晕。
  //
  // 旧写法是「各向同性的 3D fbm × 纯径向幂律」，那必然读成均匀球壳：径向幂律球对称，
  // 各向同性噪声又没有方向偏好，两者相乘只会得到「球面上均匀的斑驳」。用户的原话是
  // 「太阳的大气太耿直了，弄得像日珥一点」。
  //
  // 日珥是**沿磁力线**的结构：贴日面一圈针状体森林（spicules），往外是弧（loops/arcade），
  // 共同特征是**沿径向拉长**、彼此之间有**真空隙**。所以这里做三件事：
  //   ① **各向异性采样**：把噪声坐标在径向压缩（kR < kT）⇒ 结构自然沿径向拉长成丝/弧
  //   ② **足点调制**：一层低频把弧带切成「一根根」，而不是「一圈均匀」
  //   ③ **切向剪切**：时间演化走切向（差动自转）⇒ 结构会扭动，而不是整团平移
  // 山脊项仍由**同一次** fbm 折出来（多跑一遍噪声等于把 ALU 翻倍，没必要）。
  float coronaDensity(vec3 p, float rr, float t){
    float fall = pow(max(1.0 / rr, 0.0), uFalloff);
    if (fall < 2.0e-3) return 0.0;                  // 便宜的先算：够小就别进噪声

    // 局部正交基：u = 径向，t1/t2 = 切向。各向异性采样必须在**这个**基里做 ——
    // 直接对 p 乘不同系数是做不到「按径向拉伸」的（那只是各向异性缩放世界轴）。
    vec3 u = p / max(length(p), 1e-4);
    vec3 ref = abs(u.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 t1 = normalize(cross(u, ref));
    vec3 t2 = cross(u, t1);
    float pr = dot(p, u) / uSunR;                   // 径向坐标（日半径为单位）
    float pa = dot(p, t1) / uSunR;                  // 切向坐标
    float pb = dot(p, t2) / uSunR;
    // 径向频率低、切向频率高 ⇒ 沿径向拉长约 3 倍
    vec3 q = u * (pr * 0.55) + t1 * (pa * 1.7) + t2 * (pb * 1.7);

    // ③ 切向剪切：把切向相位随半径与时间错开 ⇒ 弧会一层层错动（差动自转的观感）
    float shear = t * 0.03 + pr * 0.35;
    float n = fbm(q * 2.6 + vec3(shear, -t * 0.05, -t * 0.04));
    float ridge = 1.0 - abs(2.0 * n - 1.0);
    // ① 丝：低值区**真的压到 0**，丝之间才有真空隙（平滑过渡会糊成灰雾）
    float filament = smoothstep(0.38, 0.86, n) * (0.22 + 1.35 * smoothstep(0.26, 0.90, ridge));
    // ② 足点：低频切成一束束；同一坐标再取一层 vnoise 拿「束与束之间的空」
    float fp = vnoise(vec3(pa * 0.9, pb * 0.9, pr * 1.6) + vec3(t * 0.015, 0.0, 5.7));
    // 贴日面的一圈弧道（spicules + 低层弧）。
    // ⚠ 同样必须**薄**：任何球对称的厚层都会把日面整体洗掉 —— 见下面 ④ 那段。
    float arcade = exp(-pow((rr - 1.0) / 0.13, 2.0));
    float shape = filament * (0.30 + 1.15 * fp)              // 一根根，不是一圈均匀
                + arcade * (0.55 + 0.85 * smoothstep(0.30, 0.88, ridge));  // 贴面弧道

    // ④ 色球 / 针状体层：贴日面一薄层的**红色**针状体。
    // 这一层**原先是一层等半径的球壳网格**（chromo.frag，R*1.012）：壳在几何上**没有径向
    // 厚度**，外缘永远等于轮廓线，所以怎么调噪声都只能读成「一圈均匀硬红环」——日珥的本质
    // 恰恰是径向**长短不一**的针与弧。挪进体积后每根针按噪声各自延伸，外缘才是参差的。
    //
    // ⚠ 厚度必须**薄**：色球在日面上几乎透明、只在临边亮，这一点是靠「正穿路径短、掠射
    // 路径长」自动成立的 —— 但只在壳厚 ≪ 日半径时成立。第一版给了 σ=0.17R，于是正穿也有
    // 0.34R 的路径，整张日面被红洗成暗红盘（光球全被盖住）。σ=0.06R 时正穿/掠射的路径比
    // 约 1:8，才读得出「临边一圈红」。
    // ⚠ 峰值必须在 rr=1 **之外**（1.015）。峰值落在 rr=1 时，那一半在光球**体内**、
    // 被光球挡住，可见的只剩外侧尾巴 —— 日缘于是留下一条填不上的暗缝（实测亮度
    // 187 → 141）。色球本来就贴在光球**之上**，峰值外移既合物理也把缝补上。
    // 外缘平滑归零：体积球本身有个硬轮廓，密度必须**在球面之前**就回到 0，
    // 否则那个球体的剪影会在天上切出一圈硬边（和之前 billboard 的方角是同一类错）。
    float outer = smoothstep(uOuterRatio, uOuterRatio * 0.70, rr);
    return fall * shape * outer;
  }

  // --- 色球：**自己一条细步长的积分** ------------------------------------------
  // ⚠ 逻辑要点（这一段是「改数值没用」的真正原因）：
  //   色球壳厚 σ=0.075R ≈ 0.5 世界单位，而日冕那条循环的步长来自
  //   「穿过**外球**(4R)的整条弦 / uSteps」≈ 1.4 世界单位 —— **比壳本身还厚**。
  //   于是壳整个掉进采样点之间：uChromo 从 26 调到 60，日缘的值一个都不变；
  //   删掉自吸收也一样。**欠采样还会抖出又硬又碎的边**（用户报的「奇怪的硬边」）。
  //   所以色球必须有独立的一条、步长与壳厚相称的积分。
  float CHROMO_SIGMA = 0.075;   // 基准壳厚（日半径为单位）
  float CHROMO_PEAK  = 1.010;   // 壳的基准高度（必须在 1 之外，否则一半被光球挡住）
  float CHROMO_TMAX  = 2.9;     // 厚度最大倍数（步长与区间按它算，保证最厚的针也采得到）

  // 色球密度。**关键：壳厚不是常数，而是被一个低频高度场调制** ——
  // 厚度恒定的壳必然读成「一圈均匀的环」（不管噪声多花），因为它的外缘就是一条等半径线。
  // 参考图（SDO）里每根针**长短不一**，那要求"这一片有多厚"本身是个场。
  float chromoField(vec3 p, float rr, float t){
    // 便宜的先算：偏离壳的最大可能范围就直接返回，局部基与噪声一个都不用算。
    float halfMax = 3.0 * CHROMO_SIGMA * CHROMO_TMAX;
    if (rr > CHROMO_PEAK + halfMax || rr < CHROMO_PEAK - halfMax) return 0.0;

    // 切向频率高、径向低 ⇒ 每根针都沿**自己的法线**往外长（不是在某个固定轴上压扁 ——
    // 旧 chromo.frag 就是 `p * vec3(38,38,9)`，针全朝对象空间的同一根轴，转到侧面就露馅）
    vec3 u = p / max(length(p), 1e-4);
    vec3 ref = abs(u.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 t1 = normalize(cross(u, ref));
    vec3 t2 = cross(u, t1);
    float pr = dot(p, u) / uSunR;
    float pa = dot(p, t1) / uSunR;
    float pb = dot(p, t2) / uSunR;

    // 细针：高频、切向频率高
    vec3 qs = u * (pr * 1.2) + t1 * (pa * 7.0) + t2 * (pb * 7.0);
    float sp = vnoise(qs * 3.0 + vec3(0.0, 0.0, -t * 0.40));
    float needle = pow(clamp(1.0 - abs(2.0 * sp - 1.0), 0.0, 1.0), 4.0);

    // 高度场：低频，决定「这一片针有多长」⇒ 参差的须状外缘
    float hgt = vnoise(vec3(pa * 2.1, pb * 2.1, pr * 2.6) + vec3(0.0, 0.0, 17.3));
    float thick = CHROMO_SIGMA * (0.45 + CHROMO_TMAX * hgt * hgt);
    // 高的地方峰值也往外推 ⇒ 针真的"长出去"，而不是只在原地变亮
    float peak = CHROMO_PEAK + thick * 0.45;
    float base = exp(-pow((rr - peak) / thick, 2.0));
    if (base < 0.004) return 0.0;
    return base * (0.25 + 2.10 * needle);
  }

  void main(){
    vec3 ro = cameraPosition;
    vec3 rd = normalize(vWorld - ro);

    vec2 to = hitSphere(ro, rd, uOuter);
    if (to.y <= 0.0) discard;                       // 这条射线根本不碰体积
    float t0 = max(to.x, 0.0);
    float t1 = to.y;

    // --- ① 在**场景深度**处夹断（体积渲染的正确性关键）---------------------------
    // 用户的原话：「raymarching 必须在深度前面停下来」。不夹断的话，遮挡物**背后**的介质
    // 也会被累加进来 —— 现象就是日冕糊在行星的背光面上。
    //
    // ⚠ 这一步**不能**在场景那一趟里做：日冕渲染进的那个 framebuffer，深度附件就是要采的
    // 纹理，**同一 framebuffer 的附件不能同时被采样**（反馈环）。所以深度来自 postfx 的
    // **深度预趟**（另一张纹理）。
    //
    // 注意这与之前那个「在光球表面截断」的写法**不是一回事**：那是在一个**固定半径**上切，
    // 于是 b = R 处路径长度突降一半 ⇒ 阶跃 ⇒ 黑带。这里是切在**真实的遮挡面**上，
    // 位置随场景连续变化 —— 所以它既正确、又不会造出硬边。
    if (uHasDepth > 0.5) {
      float dz = texture(uDepth, gl_FragCoord.xy / uResolution).x;
      // 窗口深度 → 沿**视线方向**的距离（正值）。⚠ 这个式子的输出是正距离，**不是**负的视空间 z。
      // 我第一次就栽在这儿：拿它去除一个负的 dirView.z ⇒ tScene 变负 ⇒ t1 < t0 ⇒ **每个片元
      // 都被 discard** ⇒ 日冕整个消失（用户一眼看出「根本没有日晕日珥」）。
      float n = uNearFar.x, f = uNearFar.y;
      float viewDist = (2.0 * n * f) / (f + n - (2.0 * dz - 1.0) * (f - n));
      // 视空间射线方向（world 方向用 viewMatrix 转过去；片元里有 viewMatrix）
      vec3 dirView = (viewMatrix * vec4(rd, 0.0)).xyz;
      // 沿射线的距离 = 视轴上的距离 / 方向与视轴夹角的余弦 = viewDist / (−dirView.z)
      float tScene = viewDist / max(-dirView.z, 1e-6);
      t1 = min(t1, tScene);
    }
    if (t1 <= t0) discard;

    // 步长由「这条射线穿过体积的长度」决定 ⇒ 不论掠射还是正穿都保证覆盖，不会漏采样。
    float dt = (t1 - t0) / float(max(uSteps, 1));
    float t = uTime;
    vec3 acc = vec3(0.0);
    // ⚠ **不要自吸收项**。这里曾经有一个 `trans *= exp(-a*0.5)`（当初的理由是「免得贴边
    // 糊成一片」），代价是把一个**光学薄的发射体**做成了**有饱和**的：密度一涨 trans 就衰减
    // 得更快，**累积量被钳死** —— 于是把 uChromo 从 26 调到 60，日缘的值一个数都没变
    // （两侧都是 114/113）。用户那句「这是个逻辑问题，你改数值是没有用的」说的就是它。
    // 日冕/色球都是光学薄的，累加就该是 Σ col·density·dt，没有透射率这一项。
    // 起点抖半个步长：固定步长会在球面上留下同心分层，抖一下就没有了。
    float tt = t0 + dt * 0.5;
    for (int i = 0; i < uSteps; i++) {
      if (tt > t1) break;
      vec3 p = ro + rd * tt;
      float rr = length(p) / uSunR;
      float d = coronaDensity(p, rr, t);
      if (d > 0.0) {
        // 颜色沿半径走三段：rr≈1 是**色球**（深红）→ 暖白（K 日冕）→ 蓝白（外冕）。
        // 深红那一段原先由一层独立的球壳提供，现在由密度场自己带出来。
        vec3 col = mix(vec3(1.45, 0.34, 0.16), vec3(1.15, 0.90, 0.62),
                       smoothstep(1.005, 1.35, rr));
        col = mix(col, vec3(0.48, 0.56, 0.95), smoothstep(1.8, 3.2, rr));
        acc += col * d * dt;
      }
      tt += dt;
    }

    // --- 色球：细步长积分（步长 ≪ 壳厚，才采得到）------------------------------
    float hiR = (CHROMO_PEAK + 4.0 * CHROMO_SIGMA * CHROMO_TMAX) * uSunR;
    float loR = max((CHROMO_PEAK - 4.0 * CHROMO_SIGMA * CHROMO_TMAX) * uSunR, 0.0);
    vec2 spHi = hitSphere(ro, rd, hiR);
    if (spHi.y > 0.0) {
      float a0 = max(spHi.x, 0.0);
      float a1 = spHi.y;
      vec2 spLo = hitSphere(ro, rd, loR);
      // 壳 = 「hi 球内、lo 球外」= 两段：近侧 [a0, spLo.x] 与远侧 [spLo.y, a1]。
      // 与 lo 球无交时只剩一整段（射线从旁边擦过）。
      bool two = (spLo.y > 0.0 && spLo.x > a0);
      vec4 seg = vec4(a0, a1, 0.0, 0.0);
      if (two) { seg = vec4(a0, min(spLo.x, a1), max(spLo.y, a0), a1); }
      // 步长取壳厚的 1/3：再粗就会漏采样（这正是原来那个 bug 的成因）。
      // 步长按**最薄**的针取（厚度最小 0.45σ），否则细针照样漏采样
      float dc = CHROMO_SIGMA * 0.45 * uSunR * 0.5;
      float chromoAcc = 0.0;
      for (int k = 0; k < 2; k++) {
        if (k == 1 && !two) break;
        float sa = (k == 0) ? seg.x : seg.z;
        float sb = (k == 0) ? seg.y : seg.w;
        float segLen = sb - sa;
        if (segLen <= 0.0) continue;
        int ns = int(clamp(ceil(segLen / dc), 1.0, 48.0));
        for (int s = 0; s < 48; s++) {
          if (s >= ns) break;
          float sc = sa + segLen * (float(s) + 0.5) / float(ns);
          vec3 p = ro + rd * sc;
          chromoAcc += chromoField(p, length(p) / uSunR, t) * (segLen / float(ns));
        }
      }
      acc += vec3(1.45, 0.34, 0.16) * chromoAcc * uChromo;
    }

    // ⚠ **体积的深度必须报「近壁」，不能沿用被渲染那一面的深度。**
    //
    // 这个网格是 `side: BackSide`（只渲染**远壁**，为的是相机进到球内时也有片元）。于是
    // 片元深度 = 球的远壁 ⇒ 球**内部**的一切（太阳的光球、行星、星空）都比它近 ⇒ 深度测试
    // 把它们当成遮挡物。后果有两个，用户一句话说清了：
    //   · 日冕在**日面之前**的那一段永远画不出来 ——「太阳本身相当于裸体」
    //   · **日冕背后**的物体也能把它挡住 ——「可以被任何物体遮挡」
    // 报近壁之后，体积才真正表现为「**包围**着这些物体」，而不是被它们遮挡。
    // 体积的深度报**体积入口**（近壁）。有了上面那道夹断，这才是自洽的：
    //   · 比体积入口更近的东西 ⇒ 深度测试把它挡住 ✓（正确）
    //   · 体积内部/之后的物体 ⇒ 不靠深度测试，靠**积分夹断**处理 ✓（正确）
    // 报日面则会让「太阳背后的物体」挡不住体积；报远壁则会让「球内一切」都挡住它
    // （曾经的「裸体太阳」）。入口是唯一与夹断逻辑配套的选择。
    vec3 pn = ro + rd * max(to.x, 0.0);
    vec4 clip = uProj * viewMatrix * vec4(pn, 1.0);
    gl_FragDepth = clamp((clip.z / max(abs(clip.w), 1e-4)) * 0.5 + 0.5, 0.0, 1.0);

    gl_FragColor = vec4(acc * uIntensity, 1.0);
  }
#endif // PX_SUN_CORONA_FRAG
