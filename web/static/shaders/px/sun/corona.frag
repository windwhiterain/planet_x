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
#endif // PX_SUN_CORONA_FRAG
