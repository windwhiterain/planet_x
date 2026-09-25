# Planet X

A graph-based procedural content generation and rendering engine for agent-driven content
creation. Not a game and not an asset library — it is the machinery that lets an agent write a
graph script, cook a planet out of it, and render that planet.

## What it is

A pipeline, not a framework:

```
graph script  (plain Rust: a typed call chain, no DSL, no runtime DAG)
      |  px_cook::cached — the only function in a graph (reads the cache, calls the op on a miss)
      v
operators     (dylibs, loaded at runtime by *content identity*)
      |  payloads land in a content-addressed CAS
      v
.pxart artifacts
      |
      v
renderer      (bare wgpu host)  ->  PNG
```

Two properties explain almost everything else:

1. **Identity is content.** Operator libraries, artifacts, and cache keys are all hashed from
   content. Change one line of an implementation and exactly that one thing is rebuilt.
2. **The interface is compile-time; the implementation is runtime.** A graph program has its
   operator signatures fixed at compile time; the implementation is a dylib loaded at runtime
   by key. That is what makes "change an operator without recompiling the graph program" true.

## Layout

| Where | What |
|---|---|
| `px_protocol` | the data contract: types + serde + the wire format. No logic, almost never changes |
| `px_graph_schema` | the operator contract: `PxOp`, `px_op!`, `px_body!`, runtime loading |
| `px_*_schema` | per-domain data + declarations (field, volume, mesh, nurbs) |
| `px_*_op` | per-domain implementations, built as dylibs |
| `px_graph` | the execution engine: cache, CAS, graph driver |
| `px_graphs` | the graph programs (one bin per graph) + the `px` driver |
| `px_render` | the wgpu host: offline shots, resident service, preview window, param panel |
| `px_probe` | probes: GPU experiments whose *exit code* is the verdict |
| `px_verify` | the arbiter: analytic checks against a reference implementation |
| `game` | an unrelated market/economy simulation, kept as a workspace member but **not** a default one |
| `art/` | inputs (scene recipes, operator bodies, shaders) — not assets, they are source |

## Reading order

Start with [model.md](model.md) for the end-to-end picture. Before changing anything, read
[invariants.md](invariants.md). Then go to the one area you are touching:

| Area | Doc |
|---|---|
| the data contract and wire format | [protocol.md](protocol.md) |
| executing a graph: cache, CAS, driver | [graph.md](graph.md) |
| what an operator is, and how its identity is computed | [operators.md](operators.md) |
| the compute domains | [field.md](field.md), [volume.md](volume.md), [mesh.md](mesh.md), [nurbs.md](nurbs.md) |
| element-wise operators on a field | [elementwise.md](elementwise.md) |
| graph programs and the `px` driver | [programs.md](programs.md) |
| the renderer | [renderer.md](renderer.md) |
| the visual layer: recipes, materials, content | [art.md](art.md) |
| shaders and reflection | [shaders.md](shaders.md) |
| GPU access | [gpu.md](gpu.md) |
| verdicts: probes, the arbiter, reference images | [verdicts.md](verdicts.md) |
| unresolved questions and known defects | [backlog.md](backlog.md) |

Confirmed defects and stale artefacts, each with a reproduction or a source trace, are recorded in
[`FINDINGS.md`](../FINDINGS.md) at the repository root. This directory states what holds today; that
file states what is broken.

## Building and running

```powershell
cargo test                        # fast chain: default members only (no host, probes, or game)
cargo test --workspace            # everything
./format.sh                       # cargo fix + cargo fmt. It deletes nothing.

.\tools\px.ps1 -Task list         # list generic instances: id / declaration / roots / key / present
.\tools\px.ps1 -Task build        # compile the instances that are missing
.\tools\px.ps1 -Task run -Graph planet
.\tools\px.ps1 -Task test         # the same fast chain as `cargo test`
```

`cargo build` never invokes cargo itself and never compiles instances. Compiling an instance is
triggered only by `px build` or `px run --build`.

## Note on this documentation

These documents describe **what is true now**. They are not a changelog. There are no dated
rounds, no "previously we did X", no numbered-section references. When a decision matters, the
*document* states the constraint and its reason directly; the reasoning that led there is in the
git history, which is where history belongs.

If you find a statement here that the code contradicts, the code is right and this file is a bug.
