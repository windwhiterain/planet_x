#!/usr/bin/env bash
# 语法门：把 web/static 下的每个 ES module 当 .mjs 交给 `node --check`。
#
# 为什么需要它：GLSL 是写在 JS **模板字符串**里的，而模板字符串里出现一个反引号就会提前
# 终止它——于是整个模块变成一个「语法错误」，现象是页面里 `window.PlanetXMap` 根本不存在。
# 这个坑已经踩过两次（`normalMatrix`、`s * luma(s)`），所以单独做一道门。
#
# 用法：scripts/check-js.sh            （默认查 web/static 全树 + web/static/map3d）
set -uo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0

# 先跑专用扫描器：`node --check` 只会报第一个错误，而「GLSL 里混进反引号」往往是成片出现的。
if ! node "$root/scripts/check-glsl-backticks.mjs" "$root/web/static"; then
    fail=1
fi

# 再真正编译一遍 GLSL（GLSL ES 3.00 + three.js 的 ShaderMaterial 前缀）。
# 这一步能抓住 `node --check` 抓不到的：vec3+vec2、片元里用 modelMatrix、函数先用后定义……
# 这些都是「页面里整颗行星什么都不显示、控制台才报一行」的坑。glslang 缺失时脚本自己跳过。
if ! node "$root/scripts/check-shaders.mjs" \
    "$root/web/static/map3d/util.js" "$root/web/static/map3d/index.js" \
    "$root/web/static/map3d/models.js" "$root/web/static/map3d/planet.js" \
    "$root/web/static/map3d/postfx.js" "$root/web/static/map3d/sky.js" \
    "$root/web/static/map3d/sun.js"; then
    fail=1
fi
while IFS= read -r f; do
    rel="${f#"$root"/}"
    case "$rel" in
        # `map3d/` 下是 ES module：按 .mjs 解析（顶层 `import`/`export` 才合法）。
        web/static/map3d/*.js) ext=mjs ;;
        # 其余的 `web/static/*.js` 是**经典脚本**（`<script src>`，没有 import/export）。
        # 按 CommonJS 解析：经典脚本允许顶层同名 `function` 重复声明（`advance` 就有两个），
        # 而 module 语义不允许——用 .mjs 去查会报假错。
        *) ext=js ;;
    esac
    dest="$tmp/$(echo "$rel" | tr '/' '_').$ext"
    cp "$f" "$dest"
    if ! out="$(node --check "$dest" 2>&1)"; then
        # 报错里的临时路径换回仓库相对路径，方便直接跳过去。
        echo "$out" | sed "s|$dest|$rel|g"
        fail=1
    fi
done < <(find "$root/web/static" -name '*.js' -not -path '*/node_modules/*' | sort)
[ "$fail" = 0 ] && echo "check-js: OK"
exit "$fail"
