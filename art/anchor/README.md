# 判据的靶子（anchor）

这个目录里的东西**不是**资产、**不是**产物、**不是**仪器 —— 它们是**判据本身**。

| 文件 | 是什么 |
|---|---|
| `orbit-bare.png` 等六张 | **J1 的判据图**：`orbit-bare` / `orbit-bare-nolight` / `orbit-bare-shadow` / `orbit-rings` / `orbit-soft` / `orbit-proxy-fine-bound`，960×640 |
| `frozen/*.pxart` 六份 | **老形状**（`--no-frame-graph`）的冻产物，锚宿主读得懂的那种；「逃生门」那一行判的就是它们的**文件字节** |
| `hashes.txt` | 登记在案的读数（含出处与量法） |

## ⚠ 为什么它们在 git 里，而别的仪器在 `target/`

**因为 2026-09-18 那一次，`target/oracle/` 整个不见了。**

丢的：锚 exe（`D7ED54FDB8323EDD…`）、`hashes.txt`、`target/oracle/*.ps1` 那一套仪器、
`pxart-frozen/` 那六份副本。
**活下来的**：六张判据图（当时恰好也在 `target/j1/` 下留了一份）、以及六份冻产物
（`target/pcg` 是内容寻址的，用 `--bin scene <名> --no-frame-graph` 重烘，
**文件字节的 sha256 与登记值逐字相同**）。

⇒ 由此立的规矩：

> **一份能被删掉的判据，不是判据。**

`target/` 是 git 忽略的、随时可能被清掉的目录 —— 把**靶子**放在那里，
等于让"这个工程能不能被验证"取决于一次 `rm -rf`。
所以靶子进仓库；**仪器**（脚本、探针、读数）仍然留在 `target/`，因为它们可以重写，靶子不能。

⚠ 而**锚 exe 找不回来了**：它是**冻结的构建产物**，重建出来的不是同一个字节序列，
而"冻结"正是它作为 oracle 的全部意义。它的**读数**都记在 `hashes.txt` 与
`.agents/notes/art/*.md` 里，所以已定的判据不受影响；**但"向 oracle 提问"那个能力没了**
（§143 靠它测出了透明相位的次序规则）。要用就得重建一支，而重建的那支**必须先把这六张图重出一遍**、
逐字节对上，才够格当 oracle。

## 怎么用

```powershell
# J1：我们的宿主出图 → 与这里的六张逐字节比
cargo run -q -p px_graphs --bin scene orbit-bare
cargo run -q -p px_render_wgpu -- --offline --scene <产物> --out x.png --width 960 --height 640
# x.png 的 sha256 前 16 应当等于 hashes.txt 里那一格

# 逃生门：老形状产物的**文件字节**哈希
cargo run -q -p px_graphs --bin scene orbit-bare --no-frame-graph
# 拿到的 .pxart 的 sha256 前 16 应当 == 2795F948E6987E11
```

⚠ **老形状是 oracle 的母语，不是交付的形状。**
它今天只有"取 oracle 读数"和"逃生门判据"两个正当用途；
产品形状是**帧图**（`passes` 由帧图配方生成）。
本宿主**读不了**老形状（结构性地缺 `scene_depth`），这是**已知且正确**的
——能读老形状的只有锚，而锚已经不在了。

## 判据的两半（§146.3 立）

1. **判据的输入必须是刚烘出来的产物**（`manifest.json` 可能是陈旧的，实测过一次五档全报"没有资源"）；
2. **而夹具/靶子必须住在仓库里** —— 两半合起来才回答得了"我跑的是哪份产物"**和**"下一个人怎么跑出同样的东西"。
