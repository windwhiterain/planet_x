# 截图 / 场景测试（`scripts/shots/`）

**渲染改动只能用像素验收** —— 计数指标（drawCalls/triangles/programs）在着色器编译失败、
物体什么都没画的时候**一个都不抖**（本仓为此误判过一次「无回归」）。这一层把
「拍图 → 量像素 → 判据 / 基线」做成能重复跑的命令行。

工具**进版本库**（不再住在 gitignore 的 `scratch/` 下随会话蒸发）。
完整推导、五个设计决定、量具清单、未做项都在
[`.agents/notes/shot-harness.md`](../../.agents/notes/shot-harness.md)。

```bash
pwsh -File scripts/web.ps1                      # ① 服务（框架会自己找它并核对 exe 路径）
node scripts/shots/run.mjs --list               # ② 看有哪些场景
node scripts/shots/run.mjs --all                # ③ 拍：scratch/shots/<场景>.png + .json
node scripts/shots/run.mjs --all --strict       #    判据不过 / 基线漂移 ⇒ exit 1
node scripts/shots/run.mjs --all --update       #    把当前画面写成基线
node scripts/shots/run.mjs --scene sun-disk --query 'q=ultra;tune=postfx:0,coronaOn:0'
node scripts/shots/run.mjs --stats <图.png> [--crop x,y,w,h --scale k]
node scripts/shots/run.mjs --solve 195,62,10    #    屏幕色 → 着色器该输出的 HDR
```

- 浏览器：**自己的** headless Edge（CDP `127.0.0.1:9333`，`--headless=new` 走真 GPU）。
  不用 DSH 的 `browser_*`（跨会话共享，会被别的会话导航走）。
- 页面标签**不关**（默认）：服务有「最后一个页面关掉就自退」的规则，关了截图器会把服务器也带走。
