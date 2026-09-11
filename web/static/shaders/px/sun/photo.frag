#ifndef PX_SUN_SUN_PHOTO_FRAG
#define PX_SUN_SUN_PHOTO_FRAG
#include <px/noise/perlin.glsl>
#include <px/noise/warpT.glsl>

  precision highp float;

  uniform float uTime;
  uniform float uIntensity;
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;

  // --- 配色：**线性 HDR 锚点**，不是「看着调」出来的 ---------------------------
  // 参考图（SDO/AIA 304Å）上那几档橙，隔着 `ACES + exposure + sRGB` 三跳看，直接照着屏幕
  // 调 HDR 会一直调歪：ACES 在 0.7 以上压得极扁、而且红通道先饱和，于是「再亮一点」的结果
  // 是**丢掉颜色和结构**（实测：日面 (254,238,220)，p10..p90 只有 238..243 —— 一块白饼）。
  //
  // 下面五个常量是 `node scripts/shots/run.mjs --solve <屏幕rgb>` **反解**出来的
  // （ACES 的逆，见 scripts/shots/grade.mjs）：先用参考图定出屏幕上的五个锚点，
  // 再反解出 HDR，于是「配色」这一步只剩审美，不用担心亮度一调颜色就跟着跑。
  //   C_VOID  ← 屏幕 (28,5,2)     暗条核心
  //   C_HOLE  ← 屏幕 (105,22,5)   冕洞/暗区
  //   C_MID   ← 屏幕 (195,62,10)  典型日面
  //   C_ACT   ← 屏幕 (225,115,30) 活动区
  //   C_CORE  ← 屏幕 (252,180,80) 活动区亮核（全场唯一 >1 的部分，负责喂 bloom）
  // 注意中段：C_MID 的绿只有 0.09、蓝只有 0.008 —— 屏幕上的深橙在 ACES 之前**几乎是纯红**。
  const vec3 C_VOID = vec3(0.0395, 0.0095, 0.0046);
  const vec3 C_HOLE = vec3(0.1956, 0.0295, 0.0053);
  const vec3 C_MID  = vec3(0.5843, 0.0749, 0.0036);
  const vec3 C_ACT  = vec3(0.9140, 0.1691, 0.0121);
  const vec3 C_CORE = vec3(1.9795, 0.4071, 0.0374);

  vec3 heatColor(float h){
    h = clamp(h, 0.0, 1.0);
    vec3 c = mix(C_VOID, C_HOLE, smoothstep(0.08, 0.34, h));
    c = mix(c, C_MID,  smoothstep(0.30, 0.62, h));
    c = mix(c, C_ACT,  smoothstep(0.60, 0.86, h));
    return mix(c, C_CORE, smoothstep(0.86, 1.00, h));
  }

  void main(){
    vec3 p = normalize(vObjPos);
    vec3 n = normalize(vNormal);
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float mu = clamp(dot(n, viewDir), 0.0, 1.0);
    float t = uTime;

    // ⓪ **域扰动（distortion）**：结构场在采样之前先被一个低频位移场推一下。
    // 参考图里那些暗区/活动区**没有一块是圆的** —— 边界全是扭的、带须的（磁流管的形状）。
    // 直接用 fbm 阈值得到的只会是圆头圆脑的斑块，怎么调频率都不像。
    // 位移量有硬约束：`amt · warpK ≲ 0.3`（推导见 px/noise/warp.glsl，超了映射会反折、
    // 出现带尖点的折痕）。这里 0.50 × 0.55 = 0.275，留了余量。
    // ⚠ 位移场的频率要**落在盘面上**才有"扭"的效果：warpK 太小时位移在整个日面上几乎是常数，
    // 那只是把噪声整体平移了一下（等于换了个随机种子），看起来完全没有扰动。
    vec3 pw = warpT(p, 0.50, 0.55, t * 0.04, 1.0);

    // ① 米粒组织：两套不同尺度/相位对流交叉淡入淡出，做出「沸腾」而不是「飘动」。
    //    `1-ridged` 给出的是**亮格 + 暗沟**的网状（304Å 的日面正是这种「苔藓/地毯」质感，
    //    而不是可见光那种圆滚滚的米粒）。
    //    米粒自身**不加扰动** —— 它该是细密均匀的，扰动只给大尺度结构。
    float ph = 0.5 + 0.5 * sin(t * 0.55);
    float SC = 105.0;
    float g1 = 1.0 - ridgedP(p * SC + vec3(0.0, 0.0, t * 0.35));
    float g2 = 1.0 - ridgedP(p * SC + vec3(37.0, 11.0, t * 0.35 + 41.0));
    float gran = smoothstep(0.10, 0.78, mix(g1, g2, ph));
    // ② 超米粒：更大尺度的对流胞，给米粒组织一个「群」的结构（约 10 倍于米粒）。
    float superG = smoothstep(0.30, 0.74, fbmP(pw * 9.0 + vec3(0.0, 0.0, t * 0.06)));
    float mott = gran * 0.58 + superG * 0.42;

    // ③ 大尺度：暗区（冕洞 / 暗条通道）+ 活动区 —— **这两层才是 304Å 的看头**。
    //    暗区：低频、大块、边界软（参考图的日面 p10 = 0：超过一成的像素基本全黑）。
    //    ⚠ 阈值是配着**梯度噪声**调的：`fbmP` 的分布比 `fbm`（格点噪声）**窄**，
    //    同一组阈值在换噪声之后会圈走两倍面积（第一版换完直接变成"熔岩星球"）。
    //    极区额外压暗：参考图上半盘就是一大片暗区。
    float hole = smoothstep(0.50, 0.68, fbmP(pw * 2.7 + vec3(5.0, 9.0, t * 0.02)));
    hole = max(hole, 0.80 * smoothstep(0.58, 0.96, abs(p.y)));
    // 活动区：高频、小而亮。第一版取了 4.2 的低频 ⇒ 亮斑有半个日面那么大，
    // 读起来是「大陆与海洋」的行星地图，不是恒星表面的活动区。
    // `ridgedP` 典型值只有 ~0.25（Σa·n²，n 平均 0.5），阈值必须比 value 版低一截。
    float act = smoothstep(0.46, 0.86, ridgedP(pw * 7.5 + vec3(21.0, 3.0, t * 0.03)));

    // 合成：典型日面(C_MID, heat≈0.62) → 暗区往下压到近黑 → 活动区往上顶到亮黄。
    float heat = 0.69 + (mott - 0.5) * 0.52
               - 0.80 * hole
               + 0.30 * act * (1.0 - 0.85 * hole);
    heat = clamp(heat, 0.0, 1.0);

    vec3 col = heatColor(heat);

    // ④ 临边：**304Å 是临边增亮**（色球在切向的路径更长 ⇒ 日缘一圈更亮），
    //    和可见光的临边昏暗正好相反。参考图 4× 放大后能直接看到那条亮边。
    //    原先这里写的是 `1 - 0.52(1-μ) - 0.14(1-μ)²`（临边**昏暗**），在深橙盘面上会把
    //    轮廓线压成一道暗缝 —— 那是上一个 session 反复修的那条「黑边」。
    //    ⚠ 盘心必须**正好 1.0**：ACES 的饱和度随电平变（同一组 HDR 值压暗一点就更红），
    //    配色锚点只在「输出电平 == 锚点电平」时才是准的。第一版拿 0.84 打底，实测盘面
    //    被推成 (170,20,33)，比目标 (185,70,14) 红得多。
    float limb = 1.00 + 0.55 * pow(1.0 - mu, 3.5);
    col *= limb;

    gl_FragColor = vec4(col * uIntensity, 1.0);
  }
#endif // PX_SUN_SUN_PHOTO_FRAG
