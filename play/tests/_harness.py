"""数据级测试的共享脚手架（**不用测试框架**，几个脚本分组跑）。

三句话（方案见 `.agents/notes/test-decoupled-suite.md`）：

1. **一次编译**：断言跑在 `planet_x` **跑出来的数据**上（`--index` 投影），测试代码
   （本目录的 `.py`）改动**不需要重编任何 Rust**。今天的问题是 8 个测试二进制**每个都
   静态链一遍整个 crate**：改一个文件要 46.7 s 重建，而真跑只有 26.5 s。
2. **轨迹复用**：一次运行按 `(二进制指纹, seed, 回合数, 分辨率)` 缓存进
   `target/test-fixtures/`，被**所有组、所有断言**复用（今天那四条长局跑的是**完全相同**的
   一批世界，2.8 万回合被重算了四遍）。二进制或 `config/game.ron` 一变，指纹就变 ⇒
   **缓存自动失效**，绝不手写「已知过期」的 golden。
3. **分组**：每个 `g*.py` 是一个组，`run.py` 按组合跑。组内**一份数据、多条断言**，
   失败信息里带 `(seed, round)` 以便定位。

用法（用 kit 那个 venv 的 python，它带 pandas）：

    play/planet_xq/.venv/Scripts/python.exe play/tests/run.py            # 默认：快组
    play/planet_xq/.venv/Scripts/python.exe play/tests/run.py all -j 7   # 全组、7 路并行
    play/planet_xq/.venv/Scripts/python.exe play/tests/g3_long.py        # 单跑一个组

环境变量：`PLANET_X_BIN` 指向别处的二进制（默认 `target/<bin>/planet_x[.exe]`）。
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pandas as pd

REPO = Path(__file__).resolve().parents[2]
CONFIG = REPO / "config" / "game.ron"
CACHE_ROOT = REPO / "target" / "test-fixtures"
_CACHE_NAME = re.compile(r"^[0-9a-f]{16}-s\d+-r\d+$")   # 投影缓存的命名（扫描时只认它）


# ── 投影缓存 ────────────────────────────────────────────────────────────────


def _exe(kind: str) -> Path:
    env = os.environ.get("PLANET_X_BIN")
    if env:
        return Path(env)
    return REPO / "target" / kind / ("planet_x.exe" if os.name == "nt" else "planet_x")


class Harness:
    """一个二进制 + 一个缓存根：把「跑一次世界」变成可复用的目录。"""

    def __init__(self, kind: str = "release", refresh: bool = False, jobs: int = 0):
        self.kind = kind
        self.refresh = refresh
        self.jobs = jobs or min(8, (os.cpu_count() or 4))
        self.path = _exe(kind)
        if not self.path.exists():
            sys.exit(
                f"找不到二进制 {self.path}\n"
                f"先编一个：cargo build{' --release' if kind == 'release' else ''}\n"
                f"（或用 PLANET_X_BIN 指向已编好的 planet_x）"
            )
        self.hits: list[str] = []
        self.misses: list[tuple[str, float]] = []
        self.warnings: list[str] = []          # `planet_x_state` 的回执（只在真跳过东西时说话）
        self._kit = KIT

    # 指纹 = 二进制（mtime+size）+ 配置哈希。**代码一改，缓存自动失效**——这是整套方案的
    # 安全底线（方案 §5）：二进制换了，「轨迹」就不该复用。
    def fingerprint(self) -> str:
        st = self.path.stat()
        h = hashlib.sha256()
        h.update(f"{self.path}:{st.st_mtime_ns}:{st.st_size}".encode())
        if CONFIG.exists():
            h.update(hashlib.sha256(CONFIG.read_bytes()).digest())
        return h.hexdigest()[:16]

    def dir_for(self, seed: int, rounds: int) -> Path:
        return CACHE_ROOT / f"{self.fingerprint()}-s{seed}-r{rounds}"

    def sweep_stale(self) -> int:
        """删掉**别的指纹**留下的投影目录，返回释放的 MB。

        为什么必须有这一步：一份 1000 回合的投影 ~170 MB，`run.py all` 一次就要 7 份
        （≈1.2 GB）；而**每次改一行引擎代码，指纹就变一次** ⇒ 旧指纹的目录全成了没人再读的死重。
        实测（2026-10）：`target/test-fixtures` 涨到 **10.9 GB / 72 个目录**，把 C 盘写到只剩
        0.1 GB，`--index` 直接报 `os error 112（磁盘空间不足）`——测试红的样子像代码坏了，
        其实是磁盘满了。既然「指纹变了 ⇒ 旧投影一定不会被复用」，扫掉它们永远是对的。
        """
        cur = self.fingerprint()
        freed = 0
        for d in CACHE_ROOT.glob("*"):
            # 只扫**投影缓存**那种名字（`<指纹>-s<seed>-r<回合>`）：`scenario/` 这类别的东西不碰
            if not d.is_dir() or d.name.startswith(cur) or not _CACHE_NAME.match(d.name):
                continue
            freed += _dir_size(d)
            shutil.rmtree(d, ignore_errors=True)
        return round(freed / 1e6, 1)

    def projection(self, seed: int, rounds: int) -> Path:
        """按 `(seed, 回合数)` 取一份投影目录：命中就直接返回，否则跑一次。

        ⚠ **没有「分辨率」这一维**：`--every K` 只管 stdout 的轨迹快照，**不动 `--index`
        写出来的投影**（实测 `--every 10` 与全量一模一样的 169.3 MB / 8.7 s）。要真减投影
        只有两条路：少几个 seed / 少几回合（或者给 `--index` 加表过滤——用户裁决不做：
        *「省的后面新测试又要改」*）。
        """
        dest = self.dir_for(seed, rounds)
        if not self.refresh and (dest / "_cache.json").exists():
            self.hits.append(dest.name)
            return dest
        # **近路 A：同 seed 已有更长的轨迹 ⇒ 截断读**（只跑短组时不必重新生成长的）。
        if not self.refresh and (longer := self._sibling(seed, rounds, longer=True)) is not None:
            self._truncate(dest, longer, seed, rounds)
            return dest
        # **近路 B：同 seed 已有更短的轨迹（带存档）⇒ 从存档**接着生成**，
        # 再把它前面那几回合的行拼回来**。实测：18/18 张表「前缀 + 续跑尾部 == 直跑」逐字全等。
        if not self.refresh and (shorter := self._sibling(seed, rounds, longer=False)) is not None:
            ckpt = shorter / "_ckpt.json"
            if ckpt.exists():
                self._extend(dest, shorter, ckpt, seed, rounds)
                return dest
        tmp = dest.with_name(dest.name + f".tmp{os.getpid()}")
        shutil.rmtree(tmp, ignore_errors=True)
        tmp.mkdir(parents=True, exist_ok=True)
        t0 = time.time()
        self.run_into(tmp, seed, rounds)
        elapsed = time.time() - t0
        info = {
            "seed": seed,
            "rounds": rounds,
            "binary": str(self.path),
            "fingerprint": self.fingerprint(),
            "elapsed_s": round(elapsed, 1),
            "size_mb": round(_dir_size(tmp) / 1e6, 1),
        }
        (tmp / "_cache.json").write_text(
            json.dumps(info, ensure_ascii=False, indent=1), encoding="utf-8"
        )
        shutil.rmtree(dest, ignore_errors=True)
        os.replace(tmp, dest)
        self.misses.append((dest.name, elapsed))
        return dest

    def _sibling(self, seed: int, rounds: int, longer: bool) -> Path | None:
        """同 seed、**同指纹**的邻档轨迹（取最接近的那个）。"""
        best: tuple[Path, int] | None = None
        for d in CACHE_ROOT.glob(f"{self.fingerprint()}-s{seed}-r*"):
            if not d.is_dir() or not _CACHE_NAME.match(d.name):
                continue
            r = int(d.name.rsplit("-r", 1)[1])
            if (r > rounds) if longer else (r < rounds):
                if best is None or (r < best[1] if longer else r > best[1]):
                    best = (d, r)
        return best[0] if best else None

    # 投影目录里**不是**轨迹的东西：元数据、存档、日志、摘要缓存（拼接时不带过去）。
    _META = ("_cache.json", "_ckpt.json", "_run.log")

    def _copy_tables(self, dest: Path, src: Path, lo: int, hi: int, mode: str) -> None:
        """把 `src` 的投影**整目录**按回合过滤后搬进 `dest`（`mode` = `"w"` 或 `"a"`）。

        ⚠ 不能只搬 `idx/`：投影目录还有**顶层的** `main.jsonl`（逐回合状态）与 `schema.json`
        （声明）——我第一版漏了它们，冷跑四组全红（`schema.json` 找不到）。
        `*.jsonl` 逐行按 `lo < round <= hi` 过滤（不整份读进内存），其余文件原样拷。
        """
        for f in sorted(src.rglob("*")):
            if not f.is_file():
                continue
            rel = f.relative_to(src)
            if f.name in self._META or f.name.startswith("_digest_extract-") or f.suffix == ".pkl":
                continue
            out = dest / rel
            out.parent.mkdir(parents=True, exist_ok=True)
            if f.suffix != ".jsonl":
                if mode == "w":
                    shutil.copy2(f, out)
                continue
            # ⚠ `newline=""`：Windows 上文本模式会把 `\n` 翻译成 `\r\n`，那样拼出来的投影
            # 与直跑**字节不同**（内容一样，但也就不该说"逐字全等"了）。
            with f.open(encoding="utf-8", newline="") as fi, out.open(mode, encoding="utf-8", newline="") as fo:
                for line in fi:
                    if not line.strip():
                        continue
                    if lo < json.loads(line).get("round", 0) <= hi:
                        fo.write(line)

    def _finish(self, dest: Path, seed: int, rounds: int, t0: float, kind: str, src: str) -> None:
        (dest / "_cache.json").write_text(
            json.dumps(
                {"seed": seed, "rounds": rounds, "binary": str(self.path),
                 "fingerprint": self.fingerprint(), "elapsed_s": round(time.time() - t0, 1),
                 "size_mb": round(_dir_size(dest) / 1e6, 1), kind: src},
                ensure_ascii=False, indent=1,
            ),
            encoding="utf-8",
        )
        self.misses.append((dest.name, round(time.time() - t0, 1)))

    def _truncate(self, dest: Path, longer: Path, seed: int, rounds: int) -> None:
        """**截断读**：只跑短组时，直接读已有长轨迹的前 `rounds` 回合，不重新生成。

        ⚠ 截断出来的目录**不留存档**：长轨迹的存档在更靠后的回合上，拿它当"更短轨迹的存档"
        会让下一次续跑接错地方。没有存档 ⇒ 下一次要更长的轨迹时会走续跑或重跑，都不会错。
        """
        t0 = time.time()
        tmp = dest.with_name(dest.name + f".tmp{os.getpid()}")
        shutil.rmtree(tmp, ignore_errors=True)
        tmp.mkdir(parents=True, exist_ok=True)
        self._copy_tables(tmp, longer, -1, rounds, "w")
        shutil.rmtree(dest, ignore_errors=True)
        os.replace(tmp, dest)
        self._finish(dest, seed, rounds, t0, "truncated_from", longer.name)

    def _extend(self, dest: Path, shorter: Path, ckpt: Path, seed: int, rounds: int) -> None:
        """**接着生成**：从短轨迹的存档续跑到 `rounds`，再把短轨迹的前缀行拼回来。

        实测（2026-10）：对 seed 7 的 400 回合轨迹续跑到 450，18/18 张投影表
        「前缀 + 续跑尾部」与**直跑 450 逐字全等** ⇒ 拼接是安全的（确定性 + 存档续跑同构）。
        """
        m = int(shorter.name.rsplit("-r", 1)[1])
        t0 = time.time()
        tmp = dest.with_name(dest.name + f".tmp{os.getpid()}")
        tail = tmp.with_name(tmp.name + "-tail")
        shutil.rmtree(tmp, ignore_errors=True)
        shutil.rmtree(tail, ignore_errors=True)
        tail.mkdir(parents=True, exist_ok=True)
        self.run_into(tail, seed, rounds - m, extra=("--start", str(ckpt)))
        tmp.mkdir(parents=True, exist_ok=True)
        self._copy_tables(tmp, shorter, -1, m, "w")
        self._copy_tables(tmp, tail, m, rounds + 1_000_000, "a")
        # 新档的存档 = 续跑那一段的终点
        if (tail / "_ckpt.json").exists():
            shutil.copy2(tail / "_ckpt.json", tmp / "_ckpt.json")
        shutil.rmtree(tail, ignore_errors=True)
        shutil.rmtree(dest, ignore_errors=True)
        os.replace(tmp, dest)
        self._finish(dest, seed, rounds, t0, "extended_from", shorter.name)

    def run_into(self, dest: Path, seed: int, rounds: int, extra=()) -> None:
        """**不吃缓存**地跑一次（确定性守卫要跑两遍同一份世界，就是靠它）。

        `extra` 用来追加开关，例如 `("--start", "w.json")`：从**捏过的档**起跑（那时 `--seed`
        被忽略）。
        """
        args = [
            str(self.path),
            "--seed", str(seed),
            "--round", str(rounds),
            "--index", str(dest),
            # 每次都留一份**存档**：同 seed 的更长轨迹就是从它续跑的（短组跑完，长组接着生成）。
            # ⚠ 调用方自己给了 `--save`（g1 的读面/replay 用例）就别再加——clap 不允许重复。
            *([] if "--save" in extra else ["--save", str(dest / "_ckpt.json")]),
            *extra,
        ]
        log = dest / "_run.log"
        dest.mkdir(parents=True, exist_ok=True)
        with open(log, "wb") as fh:
            p = subprocess.run(args, stdout=fh, stderr=subprocess.STDOUT, cwd=str(REPO))
        if p.returncode != 0:
            tail = log.read_text(encoding="utf-8", errors="replace")[-2000:]
            raise RuntimeError(f"planet_x 退出码 {p.returncode}：{' '.join(args)}\n{tail}")

    # ── 合成场景：造世界 → 捏世界 → 推进 → **只看数据**断言 ──────────────────────
    #
    # 「合成场景」型用例（把某艘舰的船体改成一半、把库存清零、把一座城的人口压到 1……）以前只能
    # 在 Rust 里做——要编进 crate、调内部 API。现在**档本身可以存成 JSON**
    # （`--save w.json`，见 `config::CheckpointFormat`），于是「造」用 `planet_x`、「捏」用 Python
    # 的 `json` 模块就够了：
    #
    # * **不进 Rust**：不用维护一份「哪些字段可改」的清单（那份清单会无限长）；
    # * **状态字段是字面量**，没有控制面那种 `Inherit` 链语义（链才需要引擎解析有效值）；
    # * `serde_json` 开了 `float_roundtrip` ⇒ f64 往返逐位不变（g1 有「JSON 档与 RON 档推进出
    #   同一份投影」那条守卫盯着），而 `json::key2` 让元组键（`城|建筑id`）也往返得回来。

    def gen(self, dest: Path, seed: int, round: int = 0, patch: dict | None = None) -> Path:
        """造一个世界（**存成 JSON 档**，所以 Python 能直接改），可选先推进 `round` 回合再捏。"""
        dest.parent.mkdir(parents=True, exist_ok=True)
        args = [str(self.path), "--seed", str(seed), "--round", str(round), "--quiet", "--save", str(dest)]
        p = subprocess.run(args, cwd=str(REPO), capture_output=True, text=True, encoding="utf-8")
        if p.returncode != 0:
            raise RuntimeError(f"planet_x 退出码 {p.returncode}：{' '.join(args)}\n{p.stderr[-1500:]}")
        if patch:
            misses = self.edit(dest, patch)
            if misses:
                self.warnings.append("没落地的补丁字段：" + "、".join(misses))
        return dest

    _id_cache: dict[str, str] | None = None

    def identity_keys(self) -> dict[str, str]:
        """`{state 里的表名: 身份键}`（如 `{"ships": "舰名", …}`）——**问引擎，不手抄**。

        唯一真值是 `model::IDENTITY`（Rust 一处声明），随 `--nouns`（= web 的
        `GET /api/schema`）发出来；这里只是把「结构体名 → state 里的表名」用 state schema
        自带的 `properties.<kind>.items.$ref` 接上。

        以前这里手抄过一张 `_ID_KEY` 镜像表：引擎改名它**不会红**，只会悄悄用一个旧键去
        找记录（找不到就报"补丁没落地"，把人引向错误的方向）。现在这类镜像表一律删掉。
        """
        if self._id_cache is None:
            doc = json.loads(self.capture(["--nouns"]))
            structs = (doc.get("identity") or {}).get("structs") or {}
            props = (doc.get("state") or {}).get("properties") or {}
            out: dict[str, str] = {}
            for kind, spec in props.items():
                ref = ((spec or {}).get("items") or {}).get("$ref") or ""
                struct = ref.rsplit("/", 1)[-1]
                if struct in structs:
                    out[kind] = structs[struct]
            if not structs or not out:
                raise RuntimeError(
                    "引擎的 `--nouns` 没给出身份键（identity.structs / state.properties 为空）"
                    "——`declared` 与真实世界对不上时，这里必须响亮地失败，不许静默回落")
            self._id_cache = out
        return self._id_cache

    def edit(self, ckpt: Path, patch: dict) -> list[str]:
        """**Python 直接改档**：按名字找实体、改字面量。返回没落地的字段（空 = 全成）。

        `patch` 的形状与 state 同形、**键名就是引擎的字段名**（现在是中文名词）：
        `{"ships": {"北辰": {"船体": 6.0}}, "factions": {...}, "cities": {...}}`。
        名字对不上、字段名打错 ⇒ 进返回值（**响亮**，不静默跳过）。
        """
        doc = json.loads(ckpt.read_text(encoding="utf-8"))
        state = doc["round_state"]["state"]
        misses: list[str] = []
        for kind, entities in patch.items():
            table = state.get(kind)
            if table is None:
                misses += [f"{kind}.{name}.{k}" for name, f in entities.items() for k in f]
                continue
            for name, fields in entities.items():
                id_key = self.identity_keys().get(kind)
                if id_key is None:
                    # 引擎没声明这种实体的身份键 ⇒ 响亮地报（不猜 `name`）
                    misses += [f"{kind}.{name}.{k}（引擎没声明 {kind} 的身份键）" for k in fields]
                    continue
                row = next((r for r in table if r.get(id_key) == name), None)
                if row is None:
                    misses += [f"{kind}.{name}.{k}" for k in fields]
                    continue
                for k, v in fields.items():
                    if k not in row:
                        misses.append(f"{kind}.{name}.{k}")
                    else:
                        row[k] = v
        ckpt.write_text(json.dumps(doc, ensure_ascii=False), encoding="utf-8")
        return misses

    def state_dump(self, ckpt: Path) -> dict:
        """读出档里的状态（Python 拿它算补丁：某势力的库存、某舰的 `船体上限`……）。"""
        return json.loads(ckpt.read_text(encoding="utf-8"))["round_state"]["state"]

    def scenario(self, name: str, seed: int, rounds: int, patch: dict | None = None,
                 start_round: int = 0) -> Path:
        """一条龙：造（可捏）→ 推进 `rounds` 回合 → 投影，返回投影目录。

        场景**不进指纹缓存**（几回合、零点几秒就重造），放在 `target/test-fixtures/scenario/<名字>/`
        ——`sweep_stale` 只扫投影缓存那种名字（`<指纹>-s<seed>-r<回合>`），不会碰它。

        ⚠ `patch` 只能改**状态字段**（`ships`/`cities`/`factions`… 那些列表型的表）。要拨
        **控制叶**（`state.control` 是字典型：图库 / 预算 / 舰队默认…）用 [`scenario_apply`]。
        """
        base = CACHE_ROOT / "scenario" / name
        shutil.rmtree(base, ignore_errors=True)
        base.mkdir(parents=True, exist_ok=True)
        ckpt = self.gen(base / "w.json", seed, round=start_round, patch=patch)
        proj = base / "out"
        self.run_into(proj, seed, rounds, extra=("--start", str(ckpt)))
        return proj

    # ── 合成场景 · 拨控制叶 ────────────────────────────────────────────────────
    #
    # `scenario()` 的 `patch` 走的是「Python 直接改档」（`edit()`），只能改**列表型的表**。
    # 类 C 剩下的那几条要**拨控制叶**：`state.control` 是**字典型**的叶库
    # （`{势力: {blueprints: {图名: {value, mode}}, construction_budget: {资源: …}, …}}`），
    # Python 侧没有它的形状镜像——硬抄一份必然漂移。
    #
    # 所以这里走**引擎自己的写入口**：`--apply` 的 diff 形状已经定义好、而且 g4 在逐条对账
    # （`src/control/wire.rs` 的补丁结构体）。代价只是多起几次 CLI（每次零点几秒），换来的是
    # 「Python 里不出现第二份控制面形状」。见施工图 `.agents/notes/test-migration-backlog.md` §5.6。

    def _apply_once(self, cur: Path, diff: dict, dest: Path, seed: int, tag: str) -> Path:
        """把一份 `--apply` 补丁叠到档 `cur` 上、存成 `dest`（`--round 0` ⇒ 只叠不推进）。"""
        dpath = dest.parent / f"{tag}.json"
        dpath.write_text(json.dumps(diff, ensure_ascii=False), encoding="utf-8")
        args = [str(self.path), "--seed", str(seed), "--start", str(cur),
                "--apply", str(dpath), "--round", "0", "--quiet", "--save", str(dest)]
        p = subprocess.run(args, cwd=str(REPO), capture_output=True, text=True, encoding="utf-8")
        if p.returncode != 0:
            raise RuntimeError(f"planet_x 退出码 {p.returncode}：{' '.join(args)}\n{p.stderr[-2000:]}")
        # 回执的语义（`src/main.rs`）：`NOTE_APPLY_TOOKOVER` / `NOTE_APPLY_REMOVED` 是**预期
        # 行为**（写值即接管、删叶换来源）；只有 `WARN_APPLY_SKIPPED`（叶片没落地）才是
        # 「这份场景不是你以为的那样」——进 warnings，由 `report()` 响亮报出。
        for line in p.stderr.splitlines():
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if rec.get("code") == "WARN_APPLY_SKIPPED":
                self.warnings.append(f"{tag}：--apply 丢了叶片 {rec.get('skipped')}")
        return dest

    def scenario_apply(self, name: str, seed: int, rounds: int, diffs: list[dict],
                       patch: dict | None = None, start_round: int = 0) -> Path:
        """一条龙：造（可捏）→ **逐份 `--apply` 拨控制叶** → 推进 `rounds` 回合 → 投影。

        `diffs` = 一串 `--apply` 补丁（形状同写面：`{"control": [...], "scope": {...}}`），
        **按顺序**叠加（后一份看到前一份的结果）。**一份也常常够**：`apply_diff` 里
        `blueprints` **先于** `buildings` 应用 ⇒ 「建图 + 把建造区指过去」一次成功。

        ⚠ **写值即接管**：只写值、不写 `mode` ⇒ 那片叶归 `Player`（系统从此不再改写它）。
        要让 AI 继续管那片叶（例如「这张图是 AI 自己造的，该被回收」），得**显式**写
        `"mode": "Inherit"`。这条不写清楚，造出来的 A/B 会整个反过来。
        """
        base = CACHE_ROOT / "scenario" / name
        shutil.rmtree(base, ignore_errors=True)
        base.mkdir(parents=True, exist_ok=True)
        cur = self.gen(base / "w.json", seed, round=start_round, patch=patch)
        for i, diff in enumerate(diffs):
            cur = self._apply_once(cur, diff, base / f"w{i + 1}.json", seed, f"{name}-diff{i}")
        proj = base / "out"
        self.run_into(proj, seed, rounds, extra=("--start", str(cur)))
        return proj

    def capture(self, args: list[str], stderr: bool = False):
        """跑一次二进制、拿它的 stdout（`--derived` / `--control` 这类单点导出）。不缓存。

        `stderr=True` 时返回 `(stdout, stderr)`——`--apply` 的回执（`NOTE_APPLY_*` /
        `WARN_APPLY_*`）走的是 stderr，那些回执本身也是契约的一部分。
        """
        p = subprocess.run([str(self.path), *args], cwd=str(REPO), capture_output=True,
                           text=True, encoding="utf-8")
        if p.returncode != 0:
            raise RuntimeError(f"planet_x 退出码 {p.returncode}：{' '.join(args)}\n{p.stderr[-2000:]}")
        return (p.stdout, p.stderr) if stderr else p.stdout

    def prewarm(self, keys: list[tuple[int, int]]) -> list[Path]:
        """并行把一批 (seed, 回合数) 备好（多个进程跑多个世界，互不干扰）。"""
        if len(keys) <= 1 or self.jobs <= 1:
            return [self.projection(s, r) for s, r in keys]
        with ThreadPoolExecutor(max_workers=min(self.jobs, len(keys))) as ex:
            return list(ex.map(lambda k: self.projection(k[0], k[1]), keys))

    def q(self, dirpath: Path, only=None):
        """读一份投影（kit 的 PlanetXQ）。`only` = 只装这几张表（长投影全装要 ~12 s）。"""
        return self._kit.load(str(dirpath), only=only)

    def digest(self, seed: int, rounds: int, build):
        """把 `build(投影目录)` 的结果按投影缓存成一个 pickle。

        投影是**真相**，摘要是**它的**缓存：摘要文件就躺在投影目录里 ⇒ 跟着它同生共死
        （二进制一变，投影目录换名，摘要自然也换）。这样「一条断言加进长组」不必再读
        170 MB（1000 回合全装 ≈ 12 s、读一遍 ≈ 8 s），只要 0.05 s 读摘要。

        摘要名字里带**抽取逻辑的代码指纹**（见 [`_code_stamp`]）：改了抽取逻辑就重算，
        只改 `run()` 里的判据就照旧命中。
        """
        d = self.projection(seed, rounds)
        path = d / f"_digest_{build.__name__}-{_code_stamp(build)}.pkl"
        if path.exists() and not self.refresh:
            return pd.read_pickle(path)
        out = build(d)
        pd.to_pickle(out, path)
        return out

    def digests(self, keys, build) -> list:
        """并行把一批 `(seed, 回合数)` 的摘要备好（摘要已在就只读 pickle）。"""
        if len(keys) <= 1 or self.jobs <= 1:
            return [self.digest(s, r, build) for s, r in keys]
        with ThreadPoolExecutor(max_workers=min(self.jobs, len(keys))) as ex:
            return list(ex.map(lambda k: self.digest(k[0], k[1], build), keys))

    def report(self) -> None:
        # **先报回执**：`edit()` / `scenario_apply()` 说过「有东西没落地」时，后面那些断言失败的
        # 真正原因往往在这里——而它在缓存统计里一个字都看不见（「没静默」得真的有人读它）。
        for w in self.warnings:
            print(f"  ⚠ 回执：{w}")
        if self.misses:
            total = sum(e for _, e in self.misses)
            print(f"  缓存：新跑 {len(self.misses)} 份（{total:.1f} s），命中 {len(self.hits)} 份")
        elif self.hits:
            print(f"  缓存：全部命中（{len(self.hits)} 份，0 s 重跑）")
        else:
            # 组自己起短局、拿单点 dump 对账（如 `g4_spec`）时一份投影都不碰——说清楚，
            # 免得「全部命中（0 份）」被读成「缓存生效」。
            print("  缓存：这一组没跑投影（自己起短局 + 单点 dump）")


def _dir_size(p: Path) -> int:
    return sum(f.stat().st_size for f in p.rglob("*") if f.is_file())


def projection_hashes(d: Path) -> dict[str, str]:
    """投影目录里**该是确定的那部分**的 sha256（`_` 开头的是我们自己的缓存/日志，跳过）。"""
    return {
        str(f.relative_to(d)): hashlib.sha256(f.read_bytes()).hexdigest()
        for f in sorted(d.rglob("*"))
        if f.is_file() and not f.name.startswith("_")
    }


def _code_stamp(build) -> str:
    """摘要的失效键 = 抽取逻辑的**内容**哈希（不是 mtime）。

    取的是「这个模块里除 `run` 以外的全部顶层函数/类」的源码：断言住在 `run()` 里，而
    判据的**数据**来自抽取函数——所以只改判据 ⇒ 摘要照旧命中；改了 `extract` / `_scan`
    这类抽取逻辑 ⇒ 摘要自动重算。比「按文件 mtime 判过期」可靠（内容没变就不该重算），
    也比手写版本号可靠（不会忘）。
    """
    import inspect

    mod = sys.modules.get(getattr(build, "__module__", ""))
    h = hashlib.sha256()
    if mod is not None:
        for name, obj in sorted(vars(mod).items()):
            if name == "run" or not (inspect.isfunction(obj) or inspect.isclass(obj)):
                continue
            if getattr(obj, "__module__", None) != build.__module__:
                continue
            try:
                h.update(inspect.getsource(obj).encode("utf-8"))
            except (OSError, TypeError):
                h.update(name.encode("utf-8"))
    try:
        h.update(inspect.getsource(build).encode("utf-8"))
    except (OSError, TypeError):
        pass
    return h.hexdigest()[:8]


def _load_kit():
    """读**本 worktree** 的 kit 源码。

    ⚠ 那个 venv 是 editable 装的，指向的是**它自己那个 worktree** 的 `planet_xq`（`.pth`
    里的 meta-path finder 优先级高于 `sys.path`）——直接 `import planet_xq` 会读到别人家的
    副本。所以这里按文件路径显式加载，保证「测的是本目录的这一份」。
    """
    local = REPO / "play" / "planet_xq" / "planet_xq" / "__init__.py"
    if local.exists():
        spec = importlib.util.spec_from_file_location(
            "planet_xq", local, submodule_search_locations=[str(local.parent)]
        )
        mod = importlib.util.module_from_spec(spec)
        sys.modules["planet_xq"] = mod
        spec.loader.exec_module(mod)
        return mod
    import planet_xq  # 退路：装在哪就用哪

    return planet_xq


# 进程内只加载一次（组脚本 `from _harness import KIT` 就能拿到同一份）。
KIT = _load_kit()


def _memoize_load():
    """**同一份投影在一个进程里只装一次**。

    实测（2026-10）：g2 一共 `KIT.load` **63 次**，只有 **31 个不同的 (目录, only) 组合**
    ⇒ 32 次是重复装同一份表（某个世界 `only=("cities",)` 装了 **8 遍**），而 load 总耗时
    **39.3 s** ≈ g2 的整个墙钟。这里按 `(目录, only)` 复用同一个 `PlanetXQ`。

    ⚠ 复用意味着**帧是共享的**：各组只读它们（grep 过，没有就地赋值）。要就地改就先 `.copy()`。
    ⚠ 生成（`projection`）发生在装之前；同一个 `(seed, 回合数)` 在一次运行里不会重生。
    """
    orig = KIT.load
    cache: dict = {}

    def load(dirpath, only=None):
        key = (str(dirpath), tuple(only) if only is not None else None)
        hit = cache.get(key)
        if hit is None:
            hit = orig(dirpath, only=only)
            cache[key] = hit
        return hit

    KIT.load = load


_memoize_load()


# ── 断言记录（够用就行：一个组一份清单 + 非零退出码）────────────────────────


# ── clean 记录：**源码与输入都没变、上次这一组全绿 ⇒ 整组跳过** ──────────────
#
# 用户裁决（2026-10）：*「用 dirty 机制，过了的就 clean 不再跑，轨迹变了全 dirty」*。
#
# 粒度是**组**，不是判据：`ck.check(name, cond)` 的 `cond` 在调用点就已经求值了，
# 想逐条跳过一个判据就得把 357 个调用点全改成 `if ck.dirty(...):`。组的粒度是免费的，
# 而且 99% 的开销本来就在「读投影 + 建摘要」那一段（判据自己是纳秒级）⇒ 组粒度拿走全部收益。
#
# 键分两半，任一变化 ⇒ 整组 dirty：
#   * `code`   = 组文件 + `_harness.py` + 读面 kit（改判据/改摘要/改读面都算）；
#   * `inputs` = 缓存里的投影文件（名+大小+mtime ⇒ **引擎一改指纹就变、轨迹重算 mtime 也变**）
#                以及 `web/static/**`（g4 会读那份静态读面）。
_CLEAN = CACHE_ROOT / "_clean.json"


def _source_key(name: str) -> str:
    h = hashlib.sha256()
    for p in (REPO / "play" / "tests" / f"{name}.py", REPO / "play" / "tests" / "_harness.py",
              REPO / "play" / "planet_xq" / "planet_xq" / "__init__.py"):
        try:
            h.update(p.read_bytes())
        except OSError:
            h.update(b"?")
    return h.hexdigest()[:16]


def _inputs_key() -> str:
    """投影 + 静态读面的「输入身份」。故意用 **名+大小+mtime**：内容没变就不算变。"""
    h = hashlib.sha256()
    skip = ("_cache.json", "_ckpt.json", "_run.log", "_clean.json")
    roots = [CACHE_ROOT, REPO / "web" / "static"]
    for root in roots:
        if not root.exists():
            continue
        for f in sorted(root.rglob("*")):
            if not f.is_file() or f.name in skip or f.suffix == ".pkl":
                continue
            st = f.stat()
            h.update(f"{f.relative_to(REPO)}:{st.st_size}:{st.st_mtime_ns}".encode())
    return h.hexdigest()[:16]


def _clean_load() -> dict:
    try:
        return json.loads(_CLEAN.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def _clean_store(name: str, key: dict | None) -> None:
    rec = _clean_load()
    if key is None:
        rec.pop(name, None)
    else:
        rec[name] = key
    try:
        _CLEAN.write_text(json.dumps(rec, ensure_ascii=False, indent=1), encoding="utf-8")
    except OSError:
        pass          # 记不上 clean 只是下次多跑一遍，不该让测试挂掉


class Checks:
    def __init__(self, group: str):
        self.group = group
        self.rows: list[tuple[bool, str, str]] = []

    def check(self, name: str, cond, detail: str = "") -> bool:
        self.rows.append((bool(cond), name, detail))
        return bool(cond)

    def finish(self) -> int:
        bad = [r for r in self.rows if not r[0]]
        width = max((len(r[1]) for r in self.rows), default=0)
        for ok, name, detail in self.rows:
            print(f"  {'ok  ' if ok else 'FAIL'} {name.ljust(width)}  {detail}")
        tail = f"，**{len(bad)} 条红**" if bad else ""
        print(f"[{self.group}] {len(self.rows) - len(bad)}/{len(self.rows)} 通过{tail}")
        return 1 if bad else 0


def group_main(name: str, run, argv: list[str] | None = None) -> int:
    """每个组统一的入口：`--bin/--refresh/--jobs` → `run(harness, checks)`。"""
    ap = argparse.ArgumentParser(prog=f"play/tests/{name}.py", description=__doc__)
    ap.add_argument("--bin", default=os.environ.get("PLANET_X_KIND", "release"),
                    help="release（长局默认，跑得快）| debug（重编快）| 二进制路径")
    ap.add_argument("--refresh", action="store_true", help="无视缓存，重跑投影")
    ap.add_argument("-j", "--jobs", type=int, default=0, help="并行跑几个世界（默认 ~8）")
    ap.add_argument("--no-clean", action="store_true",
                    help="无视 clean 记录强制重跑（`--refresh` 也会，但它还会重算投影）")
    args = ap.parse_args(argv)
    kind = args.bin if args.bin in ("release", "debug") else "release"
    if args.bin not in ("release", "debug"):
        os.environ["PLANET_X_BIN"] = args.bin
    h = Harness(kind=kind, refresh=args.refresh, jobs=args.jobs)
    ck = Checks(name)
    # 指纹 = 二进制 + `config/`：它必须进键 —— 只改了 `config/game.ron` 还没重跑投影时，
    # 缓存目录名（名字里带指纹）还没换，光看文件树会误判成 clean。
    key = {"code": _source_key(name), "inputs": _inputs_key(), "fp": h.fingerprint()}
    if not args.no_clean and not args.refresh and _clean_load().get(name) == key:
        print(f"[{name}] **clean**：源码与输入都没变、上次这一组全绿 ⇒ 整组跳过"
              f"（要重跑用 `--no-clean`，要重算投影用 `--refresh`）")
        return 0
    print(f"[{name}] 二进制 {h.path}（{kind}）指纹 {h.fingerprint()}")
    t0 = time.time()
    try:
        run(h, ck)
    finally:
        h.report()
        print(f"[{name}] 用时 {time.time() - t0:.1f} s")
    rc = ck.finish()
    # 只有「确实跑过判据、且全绿」才记 clean；红了就把记录清掉（别让下一次误跳）。
    # ⚠ 键要在**跑完之后**重算：这一跑可能刚生成了新投影（那属于输入变化），
    # 拿跑前的键去存，下一次一定对不上 ⇒ 永远 clean 不了。
    key["inputs"] = _inputs_key()
    _clean_store(name, key if (rc == 0 and ck.rows) else None)
    return rc
