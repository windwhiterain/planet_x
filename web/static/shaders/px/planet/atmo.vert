#ifndef PX_PLANET_ATMO_VERT
#define PX_PLANET_ATMO_VERT
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
#endif // PX_PLANET_ATMO_VERT
