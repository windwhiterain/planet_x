#!/usr/bin/env bash
# 语法门：GLSL 清单 + GLSL 编译 + ES module 语法。
#
# GLSL 现在住在 `web/static/shaders/**` 的**独立文件**里，引用关系走 three.js 自带的
# `#include <px/...>`（递归解析，见 scripts/check-shaders.mjs 里那段镜像实现）。
# 这三道各抓一层错，缺一不可：
#   ① 清单核对 —— 新增 .glsl 忘了登记 ⇒ 运行时 "Can not resolve #include"，页面白屏
#   ② GLSL 编译 —— vec3+vec2、片元里用 modelMatrix、函数先用后定义、include 成环、
#      守卫写错导致重复定义…… 都是「整颗行星什么都不显示、控制台才报一行」的坑
#   ③ 模块语法 —— 每个 ES module / 经典脚本是否真的能被解析
#
# 历史：这套门曾经有个 `check-glsl-backticks.mjs` 在前头扫「GLSL 注释里混进反引号」。
# 那个坑前后踩了五次（模板字符串被反引号提前终止 ⇒ 整个模块语法错误）。GLSL 挪进独立
# 文件后**反引号只是普通字符**，那个补丁性扫描器已随之删除 —— 结构上消灭，而不是继续
# 靠一道门看住。
#
# 用法：scripts/check-js.sh
set -uo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0

# ① 清单 vs 磁盘
if ! node "$root/scripts/check-glsl-manifest.mjs"; then
    fail=1
fi

# ② 编译全部入口着色器（glslang 缺失时 check-shaders.mjs 自己跳过并说明）
if ! node "$root/scripts/check-shaders.mjs"; then
    fail=1
fi

# ③ ES module / 经典脚本语法
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
