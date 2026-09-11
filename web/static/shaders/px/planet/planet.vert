#ifndef PX_PLANET_PLANET_VERT
#define PX_PLANET_PLANET_VERT
  varying vec3 vObjPos;
  varying vec3 vWorldPos;
  varying vec3 vRel;      // 相对**本体中心**的世界偏移（星环投影必须用它，不能用 vWorldPos）
  varying vec3 vNormal;
  // 对象→世界的 3x3。**必须**从顶点着色器传下来：modelMatrix 是顶点着色器才有的
  // 内置量，片元里引用它是「undeclared identifier」——整颗行星会编译失败、什么都不显示。
  varying mat3 vObjToWorld;
  void main(){
    vObjToWorld = mat3(modelMatrix);
    vObjPos = position;
    // 世界空间法线：normalMatrix 是「对象→视图空间」的。而片元里的光方向是世界空间的
    // （太阳在原点）。两者混用会让 dot(n, lightDir) 失去意义——旧版踩过的坑。
    vNormal = normalize(mat3(modelMatrix) * normal);
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    vRel = mat3(modelMatrix) * position;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
#endif // PX_PLANET_PLANET_VERT
