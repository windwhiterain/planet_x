# 截图 / 场景测试框架（`scripts/shots/`）

> 状态 `[x]`（分支 `feature/glsl-files`）｜ 索引：[notes.md](../notes.md)
> ｜ 前身：`scratch/shot.mjs` / `scratch/perf.mjs` / `scratch/interact.mjs`（**都在 gitignore 的
> scratch 下，随会话蒸发** —— 上一轮的性能/交互回归工具现在只剩一行索引）

## 0. 为什么要有它（一句话）

**渲染改动只能用像素验收**。本仓为「太阳透明」宣布过一次「无回归」，依据是
`bodies/drawCalls/triangles/programs` 四项计数**逐项相同** —— 而着色器根本没编译过、
那颗太阳什么都没画。计数只能当辅助（见 [glsl-files.md](glsl-files.md) 的第一条教训）。
所以这一层把「拍图 → 量像素 → 判据」做成一条能重复跑的命令行。

## 1. 用法

```bash
# 服务：只用 scripts/web.ps1 起（框架会自己找它，并核对 exe 路径）
pwsh -File scripts/web.ps1

node scripts/shots/run.mjs --list                 # 列出场景
node scripts/shots/run.mjs --all                  # 拍全部场景 → scratch/shots/*.png + *.json
node scripts/shots/run.mjs --scene sun-limb       # 拍一个
node scripts/shots/run.mjs --all --strict         # 判据不过 / 基线漂移 ⇒ exit 1（当门用）
node scripts/shots/run.mjs --all --update         # 把当前画面写成基线
node scripts/shots/run.mjs --scene sun-disk --query 'q=ultra;tune=postfx:0,coronaOn:0'   # 一次性 A/B 探针

# 量任意一张图（**参考图标定**用；不需要服务，也不需要浏览器）
node scripts/shots/run.mjs --stats scratch/ref/sdo-304.png
node scripts/shots/run.mjs --stats x.png --crop 500,280,280,200 --scale 3   # 裁一块放大看/量

# 配色换算：参考图上的屏幕色 ⇄ 着色器要输出的线性 HDR（ACES 正/反算）
node scripts/shots/run.mjs --solve 195,62,10     # → HDR [0.5843, 0.0749, 0.0036]
node scripts/shots/run.mjs --hdr 0.5843,0.0749,0.0036
```

## 2. 五个设计决定（都是踩出来的）

1. **浏览器必须是我自己的**。用 `127.0.0.1:9333` 上由本脚本拉起的 headless Edge
   （`--headless=new` 走**真 GPU**）。DSH 的 `browser_*` 是**跨会话共享**的，别的会话一导航，
   我拍到的就是别人那份代码 —— 上一轮为此白跑了七八轮。
2. **服务必须是本 worktree 的那个**。`/api/ping` 里有 exe 路径，与 `ROOT/target` 对不上就
   拒绝开拍（`--url` 可强指，但会印警告）。「改了代码画面不动」有一半是连错了实例。
3. **缓存一律关掉**（`Network.setCacheDisabled`）。静态文件没有 cache 头，浏览器会按启发式
   缓存住旧的 `map3d.js` / `*.frag`。
4. **场景 = 固定 URL 参数 + 固定相机 + 固定时间轴 + 隔离被测对象**：
   `q=ultra`（钉死档位）、`tune=adaptive:0`（自适应分辨率会让采样率飘）、
   `hide=labels,markers,bodies`（**隔离**：一颗行星正好飘到日面前面，判据就不可复现）、
   `t=12`（**冻结时间轴**：`uTime` 驱动米粒对流/针状体/云，不冻结两张图永远差几万像素）。
   `?t` 与 `?hide=bodies` 是本轮为它新加的调试开关（见 `index.js::readDebugQuery`）。
5. **标签页不关**。服务有「最后一个页面关掉就自退」的规则 —— 截图器一收尾就把服务器也带走
   （要收尾用 `--close`）。

## 3. 像素判据住哪

`metrics.mjs` 里的量具（都返回可断言的数）：

| 量具 | 用途 |
|---|---|
| `stats` / `annulus` / `radialProfile` | 区域/环带/径向剖面的均值、p10..p99、`whiteFrac`（三通道≥235 = ACES 白饼）、`warmth`(R−B) |
| `silhouette` | 以日心为原点按角度扫射线，得「轮廓半径 vs 角度」的 `std` / `spread` —— **日珥参差度**的核心判据（一圈硬环 ⇒ std≈0） |
| `signature` / `signatureDiff` | 24×15 的降采样网格（基线**不进**版本库的整张 PNG，只存这个 JSON，能 diff） |
| `rowProfile` | 一行的亮度剖面（看「日缘那道缝」最直接） |

⚠ **阈值要抬到背景之上**：天上有星云（~21–26 亮度）和恒星，`thr=6` 会把它们也算成「日珥」
（实测 `edgeStd` 直接爆到 227）。日缘场景用 `silThr: 55`。

## 4. 相机是**解析算出来**的，不是手调的

`scenes.mjs` 里：太阳在原点、半径 `TUNING.sunRadius`、fov 50° ⇒
`diskRadiusPx(d)` / `diskView(d)` / `limbView(d, φ)` / `makeCamera(...).project(p)` 全是闭式。
两个坑：
- `OrbitControls.maxPolarAngle = 0.47π` 会**夹**相机位置 —— 目标点在原点时必须给相机一个
  仰角（`diskView` 里 14°），否则 `setView()` 之后相机不在你以为的地方，判据全错。
- 看日缘要让**轮廓点**落在画面中心：轮廓点 L 满足 `(L−P)·L = 0`（即 `P·L = R²`），
  见 `limbView`。手调「大概对准边缘」会让每个场景的采样窗口都不一样。

## 5. 未做 / 待裁决

- `[ ]` **还没接进 `check-js.sh`**：它需要活着的服务 + 一块真 GPU，把它塞进「合流门」会把
  静态门变成有状态门。目前的用法是 `run.mjs --all --strict` 单独当**视觉门**跑。
  （用户此前那条待办「把 scratch/\*.mjs 提升进 scripts/ 并接进 check-js.sh」到这里算完成了一半：
  工具进版本库了，**是否进门待裁决**。）
- `[ ]` 基线目前只有 `signature`（颜色网格），**没有形貌判据的基线**：
  `edgeSpread` 这类判据只在场景的 `limits` 里卡了上下限，没有「与上一版差多少」的对照。
- `[ ]` `perf.mjs`（帧时间探针）与 `interact.mjs`（公开 API / 拾取回归）还没搬进来 ——
  它们的写法在 `.agents/notes/web-vfx-pipeline.md` §4 有记。
- `[ ]` 参考图（`scratch/ref/`）不进版本库（用户约束：资产必须程序生成）。要用的话自己放一张。
