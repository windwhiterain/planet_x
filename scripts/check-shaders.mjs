#!/usr/bin/env node
// Offline GLSL validator for planet_x-vfx's three.js shaders.
//
//   node check-shaders.mjs [--defines F=5,C=12] [--raw] <file.js|file.glsl> ...
//
// Why a prelude is needed
// -----------------------
// The project writes shaders as GLSL inside JS template literals and hands them
// to THREE.ShaderMaterial. three.js r169 (web/static/index.html importmap)
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

import { readFileSync, writeFileSync, mkdtempSync, existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, basename } from 'node:path';
import { fileURLToPath } from 'node:url';

// glslang 是**外部**工具，不进版本库（体积 + 平台相关）。默认去 scratch/glsl-tools 找；
// 找不到就跳过（让 scripts/check-js.sh 仍然能过），并用 `--glslang PATH` 或环境变量覆盖。
// 装法见 scratch/glsl-tools/README.md；那是一个一次性的开发工具，不是构建依赖。
const CANDIDATES = [
  process.env.GLSLANG,
  fileURLToPath(new URL('../scratch/glsl-tools/glslang-16.5.0/bin/glslang.exe', import.meta.url)),
  fileURLToPath(new URL('../scratch/glsl-tools/glslang.exe', import.meta.url)),
].filter(Boolean);
const DEFAULT_GLSLANG = CANDIDATES.find((p) => { try { return existsSync(p); } catch (e) { return false; } }) || CANDIDATES[0];

if (!existsSync(DEFAULT_GLSLANG) && !process.argv.includes('--glslang') && !process.env.GLSLANG) {
  console.log('check-shaders: 跳过（没找到 glslang；装法见 scratch/glsl-tools/README.md，或用 GLSLANG=PATH 指定）');
  process.exit(0);
}

const argv = process.argv.slice(2);
let glslang = DEFAULT_GLSLANG;
let raw = false;
let defines = { FBM_OCT: 5, CITY_MAX: 12, GODRAY_SAMPLES: 24, STREAK_TAPS: 12, GHOST_COUNT: 6 };
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

if (inputs.length === 0) {
  console.error('usage: node check-shaders.mjs [--defines F=5] [--raw] [--glslang PATH] <file.js|file.glsl> ...');
  process.exit(64);
}

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

// --- extraction -------------------------------------------------------------
const BLOCK = /(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:\/\*\s*glsl\s*\*\/\s*)?`([\s\S]*?)`\s*;/g;

function extract(src) {
  const out = [];
  let m;
  BLOCK.lastIndex = 0;
  while ((m = BLOCK.exec(src)) !== null) out.push({ name: m[1], body: m[2] });
  return out;
}

// Resolve ${NAME} - the project splices shared snippets this way. Unknown names
// are left verbatim so glslang reports them instead of us silently passing.
function expand(body, table, seen = new Set()) {
  return body.replace(/\$\{([A-Za-z_$][\w$]*)\}/g, (whole, name) => {
    if (!(name in table)) return whole;
    if (seen.has(name)) return `/* cyclic ${name} */`;
    return expand(table[name], table, new Set([...seen, name]));
  });
}

const table = {};
for (const input of inputs) {
  for (const { name, body } of extract(readFileSync(input, 'utf8'))) table[name] = body;
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

for (const input of inputs) {
  const src = readFileSync(input, 'utf8');

  if (raw || input.endsWith('.glsl')) {
    const stage = /\bgl_Position\b/.test(src) ? 'vert' : 'frag';
    checked++;
    if (!validate(stage, src, basename(input))) failed++;
    continue;
  }

  const blocks = extract(src);
  for (const { name, body } of blocks) {
    if (!/\bvoid\s+main\s*\(/.test(body)) continue; // shared snippet, not a stage
    const stage = /_VERT$/i.test(name) || /\bgl_Position\b/.test(body) ? 'vert' : 'frag';
    checked++;
    if (!validate(stage, prelude(stage) + expand(body, table), name)) failed++;
  }
}

console.log(`\n${checked - failed}/${checked} stage shader(s) valid  (GLSL ES 3.00, three.js ShaderMaterial prefix)`);
process.exit(failed === 0 ? 0 : 1);
