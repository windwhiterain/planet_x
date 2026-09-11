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

    def run_into(self, dest: Path, seed: int, rounds: int, extra=()) -> None:
        """**不吃缓存**地跑一次（确定性守卫要跑两遍同一份世界，就是靠它）。

        `extra` 用来追加开关，例如 `("--start", "w.ron")`：从**捏过的档**起跑（那时 `--seed`
        被忽略）。
        """
        args = [
            str(self.path),
            "--seed", str(seed),
            "--round", str(rounds),
            "--index", str(dest),
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

    def edit(self, ckpt: Path, patch: dict) -> list[str]:
        """**Python 直接改档**：按名字找实体、改字面量。返回没落地的字段（空 = 全成）。

        `patch` 的形状与读面同名同形：`{"ships": {"北辰": {"hull": 6.0}}, "factions": {...},
        "cities": {...}}`。名字对不上、字段名打错 ⇒ 进返回值（**响亮**，不静默跳过）。
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
                row = next((r for r in table if r.get("name") == name), None)
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
        """读出档里的状态（Python 拿它算补丁：某势力的库存、某舰的 `hull_max`……）。"""
        return json.loads(ckpt.read_text(encoding="utf-8"))["round_state"]["state"]

    def scenario(self, name: str, seed: int, rounds: int, patch: dict | None = None,
                 start_round: int = 0) -> Path:
        """一条龙：造（可捏）→ 推进 `rounds` 回合 → 投影，返回投影目录。

        场景**不进指纹缓存**（几回合、零点几秒就重造），放在 `target/test-fixtures/scenario/<名字>/`
        ——`sweep_stale` 只扫投影缓存那种名字（`<指纹>-s<seed>-r<回合>`），不会碰它。
        """
        base = CACHE_ROOT / "scenario" / name
        shutil.rmtree(base, ignore_errors=True)
        base.mkdir(parents=True, exist_ok=True)
        ckpt = self.gen(base / "w.json", seed, round=start_round, patch=patch)
        proj = base / "out"
        self.run_into(proj, seed, rounds, extra=("--start", str(ckpt)))
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


# ── 断言记录（够用就行：一个组一份清单 + 非零退出码）────────────────────────


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
    args = ap.parse_args(argv)
    kind = args.bin if args.bin in ("release", "debug") else "release"
    if args.bin not in ("release", "debug"):
        os.environ["PLANET_X_BIN"] = args.bin
    h = Harness(kind=kind, refresh=args.refresh, jobs=args.jobs)
    ck = Checks(name)
    print(f"[{name}] 二进制 {h.path}（{kind}）指纹 {h.fingerprint()}")
    t0 = time.time()
    try:
        run(h, ck)
    finally:
        h.report()
        print(f"[{name}] 用时 {time.time() - t0:.1f} s")
    return ck.finish()
