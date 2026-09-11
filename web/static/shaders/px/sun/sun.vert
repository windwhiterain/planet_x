#ifndef PX_SUN_SUN_VERT
#define PX_SUN_SUN_VERT
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vNormal;
  void main(){
    vObjPos = position;
    // 世界空间法线：normalMatrix 是「对象→视图」空间的，而光照方向是世界空间的，
    // 两者混用会让 dot(n, lightDir) 失去意义（旧版踩过的坑，见 notes/webui-3d-rendering）。
    vNormal = normalize(mat3(modelMatrix) * normal);
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
#endif // PX_SUN_SUN_VERT
