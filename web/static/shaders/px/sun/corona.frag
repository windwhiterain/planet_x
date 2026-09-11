#ifndef PX_SUN_CORONA_FRAG
#define PX_SUN_CORONA_FRAG
#include <px/noise/fbm.glsl>

  precision highp float;

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
    float base = exp(-pow((rr - 1.015) / 0.075, 2.0));
    if (base > 0.004) {
      // 切向频率高、径向低 ⇒ 每根针都沿**自己的法线**往外长（不是在某个固定轴上压扁 ——
      // 旧 chromo.frag 就是 `p * vec3(38,38,9)`，针全朝对象空间的同一根轴，转到侧面就露馅）
      vec3 qs = u * (pr * 1.2) + t1 * (pa * 7.0) + t2 * (pb * 7.0);
      float sp = vnoise(qs * 3.0 + vec3(0.0, 0.0, -t * 0.40));
      float needle = pow(clamp(1.0 - abs(2.0 * sp - 1.0), 0.0, 1.0), 4.0);
      shape += base * (0.12 + 1.55 * needle);
    }
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
        // 颜色沿半径走三段：rr≈1 是**色球**（深红）→ 暖白（K 日冕）→ 蓝白（外冕）。
        // 深红那一段原先由一层独立的球壳提供，现在由密度场自己带出来。
        vec3 col = mix(vec3(1.45, 0.34, 0.16), vec3(1.15, 0.90, 0.62),
                       smoothstep(1.005, 1.35, rr));
        col = mix(col, vec3(0.48, 0.56, 0.95), smoothstep(1.8, 3.2, rr));
        float a = d * dt;
        acc += col * a * trans;
        trans *= exp(-a * 0.5);
      }
      tt += dt;
    }

    gl_FragColor = vec4(acc * uIntensity, 1.0);
  }
#endif // PX_SUN_CORONA_FRAG
