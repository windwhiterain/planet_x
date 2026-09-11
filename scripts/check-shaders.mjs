#!/usr/bin/env node
// Offline GLSL validator for planet_x-vfx's three.js shaders.
//
//   node check-shaders.mjs [--defines C=12] [--glslang PATH] [file.vert|file.frag ...]
//
// GLSL **住在独立文件里**（web/static/shaders/**），不在 JS 模板字符串里了。引用关系走
// three.js 的 `#include <px/...>`，所以本脚本必须**镜像 three 的解析语义**再喂给 glslang
// （见下面 resolveIncludes 那段）。不带参数时校验目录下全部 `.vert`/`.frag`。
//
// Why a prelude is needed
// -----------------------
// The project hands these sources to THREE.ShaderMaterial. three.js r169 (web/static/index.html importmap)
// ALWAYS prepends `#version 300 es` plus a pile of `#define`s and the built-in
// attribute/uniform declarations for any non-RawShaderMaterial
// (three.module.js, WebGLProgram: "GLSL 3.0 conversion for built-in materials
// and ShaderMaterial"). So a shader body on its own is NOT valid source: names
// like `position`, `modelMatrix`, `gl_FragColor` only exist after the prefix is
// applied. This script reproduces that prefix, then runs glslang.
//
// Modes
//   default : reproduce three.js ShaderMaterial prefix + `#version 300 es`
//   --raw   : pass the file through untouched (file must carry its own #version)
//
// `--defines` only needs the values that vary at runtime; the names are taken
// from the project's `defines: { ... }` blocks. Defaults are the largest values
// the project uses, i.e. the strictest thing to validate.

import { readFileSync, writeFileSync, mkdtempSync, existsSync, readdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

// glslang 是**外部**工具（体积 + 平台相关），不进版本库。
//
// ⚠ **它不住在任何 worktree 里** —— 放在所有 worktree 之外的固定位置：
//   <仓库的父目录>/.tools/glsl-tools/
// 以前放在「各 worktree 自己的 scratch/」下，结果是**每开一个 worktree 就复制一份 67M**
// （最多同时存在三份），而清掉那个 worktree 时门又会「静默跳过」却报绿 —— 两头都错。
// 开发工具是**机器级**的，不是**分支级**的。
//
// 仍可用 `GLSLANG=PATH` 或 `--glslang PATH` 覆盖；装法见 .tools/glsl-tools/README.md。
const CANDIDATES = [
  process.env.GLSLANG,
  fileURLToPath(new URL('../../.tools/glsl-tools/glslang-16.5.0/bin/glslang.exe', import.meta.url)),
  fileURLToPath(new URL('../../.tools/glsl-tools/glslang.exe', import.meta.url)),
].filter(Boolean);
const DEFAULT_GLSLANG = CANDIDATES.find((p) => { try { return existsSync(p); } catch (e) { return false; } }) || CANDIDATES[0];

if (!existsSync(DEFAULT_GLSLANG) && !process.argv.includes('--glslang') && !process.env.GLSLANG) {
  console.log('check-shaders: 跳过（没找到 glslang；装法见 <仓库父目录>/.tools/glsl-tools/README.md，或用 GLSLANG=PATH 指定）');
  process.exit(0);
}

const argv = process.argv.slice(2);
let glslang = DEFAULT_GLSLANG;
let raw = false;
// 只剩 CITY_MAX：噪声八度数与 godray/streak/ghost 的循环上界**都改成 uniform 了**
// （常量上界会被 D3D 的 fxc 完全展开 —— 那份教训见
//  .agents/notes/shader-compile-stall.md），所以它们不再是 #define。
let defines = { CITY_MAX: 12 };
const inputs = [];

for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  if (a === '--raw') raw = true;
  else if (a === '--glslang') glslang = argv[++i];
  else if (a === '--defines') {
    for (const kv of argv[++i].split(',')) {
      const [k, v] = kv.split('=');
      defines[k.trim()] = (v ?? '1').trim();
    }
  } else inputs.push(a);
}

// 无参数不是错误：下面按「校验 web/static/shaders 下全部 .vert/.frag 入口」处理。

// --- three.js r169 ShaderMaterial prefix (trimmed of light/fog/map chunks) ---
const PRECISION = ['float', 'int', 'sampler2D', 'samplerCube', 'sampler3D', 'sampler2DArray',
  'sampler2DShadow', 'samplerCubeShadow', 'sampler2DArrayShadow',
  'isampler2D', 'isampler3D', 'isamplerCube', 'isampler2DArray',
  'usampler2D', 'usampler3D', 'usamplerCube', 'usampler2DArray']
  .map((t) => `precision highp ${t};`).join('\n');

// Exactly what WebGLProgram declares per stage. NOTE the asymmetry: modelMatrix,
// normalMatrix, modelViewMatrix and projectionMatrix exist ONLY in the vertex
// stage - that is why using modelMatrix in a fragment shader fails at runtime,
// and this prelude must not hide it.
const VERT_UNIFORMS = `uniform mat4 modelMatrix;
uniform mat4 modelViewMatrix;
uniform mat4 projectionMatrix;
uniform mat4 viewMatrix;
uniform mat3 normalMatrix;
uniform vec3 cameraPosition;
uniform bool isOrthographic;`;

const FRAG_UNIFORMS = `uniform mat4 viewMatrix;
uniform vec3 cameraPosition;
uniform bool isOrthographic;`;

const DEFINE_TEXT = Object.entries(defines).map(([k, v]) => `#define ${k} ${v}`).join('\n');

// #version must be line 1; the rest mirrors WebGLProgram's prefix, flat (the
// project uses no instancing/skinning/vertex-colour attribute chunks).
function prelude(stage) {
  const version = '#version 300 es';
  if (stage === 'vert') {
    return `${version}
#define attribute in
#define varying out
#define texture2D texture
${PRECISION}
#define SHADER_TYPE ShaderMaterial
#define SHADER_NAME
${DEFINE_TEXT}
${VERT_UNIFORMS}
attribute vec3 position;
attribute vec3 normal;
attribute vec2 uv;
`;
  }
  return `${version}
#define varying in
layout(location = 0) out highp vec4 pc_fragColor;
#define gl_FragColor pc_fragColor
#define gl_FragDepthEXT gl_FragDepth
#define texture2D texture
#define textureCube texture
#define texture2DProj textureProj
#define texture2DLodEXT textureLod
#define texture2DProjLodEXT textureProjLod
#define textureCubeLodEXT textureLod
#define texture2DGradEXT textureGrad
#define texture2DProjGradEXT textureProjGrad
#define textureCubeGradEXT textureGrad
${PRECISION}
#define SHADER_TYPE ShaderMaterial
#define SHADER_NAME
${DEFINE_TEXT}
${FRAG_UNIFORMS}
`;
}

// --- chunk 加载 + include 解析 -------------------------------------------------
// 逐字照抄 three r169 `src/renderers/webgl/WebGLProgram.js` 的语义：
//   const includePattern = /^[ \t]*#include +<([\w\d./]+)>/gm;
//   function resolveIncludes(s){ return s.replace(includePattern, includeReplacer); }
//   function includeReplacer(m, include){ ...; return resolveIncludes(string); }   ← 递归
// 包括「找不到就抛」这一条：门禁松了，运行时才炸就没意义了。
//
// ⚠ three 这套解析器**没有 include guard、也不去重**：同一个 chunk 经两条路径进来就会
// 重复定义函数。所以每个 .glsl 必须自带 `#ifndef PX_...` 守卫（本目录全部如此）——
// glslang 会在守卫写错时报重定义，那正是我们要它守的东西。
const SHADER_DIR = fileURLToPath(new URL('../web/static/shaders', import.meta.url));
const INCLUDE = /^[ \t]*#include +<([\w\d./]+)>/gm;

function loadChunks() {
  const map = new Map();
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else map.set(relative(SHADER_DIR, p).split(sep).join('/'), readFileSync(p, 'utf8'));
    }
  };
  walk(SHADER_DIR);
  return map;
}

const CHUNKS = loadChunks();

function resolveIncludes(text, stack = []) {
  return text.replace(INCLUDE, (_m, name) => {
    const chunk = CHUNKS.get(name);
    if (chunk === undefined) throw new Error('Can not resolve #include <' + name + '>');
    if (stack.includes(name)) throw new Error('include 成环: ' + [...stack, name].join(' -> '));
    return resolveIncludes(chunk, [...stack, name]);
  });
}

// 不带参数 ⇒ 校验全部入口（`.vert` / `.frag`）。公共块是 `.glsl`，只经 include 进来。
const entries = (inputs.length ? inputs : [...CHUNKS.keys()]
  .filter((k) => k.endsWith('.vert') || k.endsWith('.frag'))
  .map((k) => join(SHADER_DIR, k))).sort();

if (entries.length === 0) {
  console.error('check-shaders: web/static/shaders 下没有 .vert/.frag 入口');
  process.exit(1);
}

const dir = mkdtempSync(join(tmpdir(), 'glslcheck-'));
let checked = 0;
let failed = 0;

function validate(stage, text, label) {
  const file = join(dir, `${label.replace(/[^\w.-]/g, '_')}.${stage}`);
  writeFileSync(file, text, 'utf8');
  const r = spawnSync(glslang, ['-S', stage, file], { encoding: 'utf8' });
  const out = `${r.stdout || ''}${r.stderr || ''}`;
  if (r.status === 0) {
    console.log(`  PASS  ${label} [${stage}]`);
    return true;
  }
  console.log(`  FAIL  ${label} [${stage}]  (glslang exit ${r.status})`);
  console.log(out.trim().split('\n').map((l) => '        ' + l).join('\n'));
  return false;
}

for (const file of entries) {
  const stage = file.endsWith('.vert') ? 'vert' : 'frag';
  const label = relative(SHADER_DIR, file).split(sep).join('/');
  let body;
  try {
    body = resolveIncludes(readFileSync(file, 'utf8'));
  } catch (e) {
    console.log(`  FAIL  ${label}  (${e.message})`);
    failed++; checked++;
    continue;
  }
  checked++;
  if (!validate(stage, prelude(stage) + body, label)) failed++;
}

console.log(`\n${checked - failed}/${checked} stage shader(s) valid  (GLSL ES 3.00, three.js ShaderMaterial prefix)`);
process.exit(failed === 0 ? 0 : 1);
