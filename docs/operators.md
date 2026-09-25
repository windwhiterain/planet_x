# Operators: contract, identity, generation, loading

An operator has two halves:

* a **declaration** — a Rust type carrying the parameter type, the input shape, and the output
  domain, plus the *name of the library* that implements it. It is compiled into every graph
  program. `px_graph_schema/src/contract.rs`.
* an **implementation** — one exported function in a `dylib`, loaded at run time by symbol name.
  `px_graph_schema/src/ops.rs` is the only loader.

A graph program therefore has operator *signatures* at compile time and operator *code* only at run
time. Nothing is loaded until a node actually renders.

## 1. The `PxOp` contract

```rust
pub trait PxOp: Sized {
    const ID: &'static str;      // human-readable id, e.g. "field.fbm" — goes into the node key
    const LIB: &'static str;     // package that implements it, e.g. "px_field_op"
    const SYMBOL: &'static str;  // concat!(LIB, "__", <type name>), computed by the macro
    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    type Inputs: PxInputs;
    type Payload: Build;
    fn new() -> Self;
    fn interface() -> u64;                                          // provided
    fn source_hash() -> Result<&'static str, String>;                // provided
    fn decl_hash() -> &'static str;                                  // provided, default ""
    fn render(&self, p: &Self::Params, i: &Self::Inputs)              // provided
        -> Result<Self::Payload, String>;
}
```

| Associated type | What it fixes | How a mistake shows up |
|---|---|---|
| `Params` | the hyper-parameters, deserialized from `art/<graph>/<node>.toml` (or defaulted when that file is absent) | missing `Default` / `Serialize` / `DeserializeOwned` / `PxKeyed` is a bound error; the parameter structs carry `#[serde(deny_unknown_fields)]`, so a misspelled TOML field name is rejected when the file is read |
| `Inputs` | which upstreams this operator eats, and their field names | `cached` constructs `O::Inputs` in the graph script; a missing upstream or a wrong domain is a compile error (`expected MixInput, found …`) |
| `Payload` | the output domain (`Field`, `VolumeData`, `MeshData`, `Curve`, …) | must implement `Build` (`detail` / `encode` / `decode`); feeding a volume to something wanting a field is a compile error |

There is **no registry, no descriptor table, and no string-id dispatch**. `LIB` and `SYMBOL` are
`const`s, so "which library, which symbol" is decided when the declaration is compiled. `render`
goes through the loader on every call: the library itself is opened once per process, but the
symbol lookup repeats.

* `interface()` = `interface_hash(&[type_name::<Params>(), type_name::<Inputs>(),
  type_name::<Payload>()])` — blake3 over the domain tag `px_cook/interface/v1` and the three
  length-prefixed type-name strings, taking the first 8 bytes as a little-endian `u64`
  (`px_graph_schema/src/contract.rs`). It replaces a hand-written `version`: changing a field of the
  parameter struct changes the type name and therefore the hash. It is deliberately **not cached** —
  a `static` inside a generic function is not split per monomorphisation.
* `source_hash()` is the implementation's identity, read at run time out of the dylib
  (`ops::source_hash(LIB)`), not a compile-time constant of the graph program.
* `decl_hash()` is the source fingerprint of the crate the declaration lives in; `px_op!` fills it
  with `env!("PX_SOURCE_HASH")`, so a declaring crate needs a `build.rs`.
* `render()` default body: `ops::body::<Self>()?` then call the returned function pointer.
* `PxInputs` has one method, `collect(&self, hasher: &mut blake3::Hasher)`. `()` implements it, so
  "no upstream" is a shape, not an option (`cached(&graph, "clusters", field::Fbm, params, ())`).

## 2. Macros and derives

| Item | Where | What it produces |
|---|---|---|
| `px_op! { Name, "id", "lib", Params, Inputs, Payload }` | `px_graph_schema/src/contract.rs` | `pub struct Name;` (doc attributes preserved) and `impl PxOp`: `ID`/`LIB`, `SYMBOL = concat!(lib, "__", stringify!(Name))`, the three associated types, `new()`, `decl_hash() = env!("PX_SOURCE_HASH")` |
| `px_body! { Name, \|p, i\| … }` | same file | `#[unsafe(export_name = concat!(env!("CARGO_PKG_NAME"), "__", stringify!(Name)))] pub extern "Rust" fn __px_body(&<Name as PxOp>::Params, &<Name as PxOp>::Inputs) -> Result<Payload, String>` returning `Ok($body)`. The exported Rust name is fixed (`__px_body`), so **one `px_body!` per module** |
| `px_body_raw! { Name, Params, Inputs, Payload, \|p, i\| … }` | same file | the same symbol convention with the three types written as explicit paths, so no `PxOp` type is needed. Generated instance libraries use it so they depend only on the author-side crate + the contract, never on the graph-side operator type (importing that would statically link the driver into every instance library) |
| `px_impl_lib!()` | same file | three exports: `<pkg>__source_hash` (this library's `env!("PX_SOURCE_HASH")`), `<pkg>__contract_hash` (`px_graph_schema::SOURCE_HASH` as compiled into this library), `<pkg>__toolchain_hash` (`env!("PX_TOOLCHAIN_HASH")`). Written once per implementation library |
| `px_local_op!` | `px_cook/src/lib.rs` | `impl PxOp` for a type whose implementation is written *in the graph program*: `LIB = ""`, `SYMBOL = stringify!(Name)`, `source_hash()` = this crate's `PX_SOURCE_HASH`, `render()` = the inline body. Nothing is loaded |
| `#[derive(PxParams)]` | `px_derive/src/lib.rs` | `impl PxKeyed for <struct>`: for each named field, the field-name bytes followed by `HashField::hash_field(&self.field)`; a field marked `#[nohash]` is skipped |
| `#[derive(PxInputs)]` | same file | `impl PxInputs for <struct>`: for each named field, the field-name bytes followed by `self.field.key` |

Both derives require named fields (the field *name* is what enters the key, not the position).
`HashField` (`px_graph_schema/src/identity.rs`) is a closed set — integers, floats, `bool`, `[T; N]`,
`str`, `String`, and `Cooked<T>` (which contributes its *key*, with the tag
`px_cook/cooked-field/v1`). Adding a parameter field of a type that has no `HashField` impl is a
compile error at the derive-generated call site, which is the point.

⚠ The declaration ↔ implementation link is **two strings** (`"px_field_op"` + the type name), which
the compiler cannot check. `px_graphs/tests/ops_load.rs` closes that gap by actually loading the
declared operators it names (a hand-written list, not derived from `px_decls`) and by reading the
identity of each of the five preset libraries and asserting they differ pairwise.

## 3. Identity

Two different keys are in play, computed by two different functions.

### 3.1 A node key (the artifact's identity)

`px_graph_schema::keys::node_key` (called from `px_cook::cached`), domain tag `px_cook/v1`:

```
op.id ‖ interface ‖ source_hash ‖ canonical params JSON ‖ upstream keys
```

`upstream keys` is whatever the operator's `Inputs` folds in: `#[derive(PxInputs)]` writes each
field name plus that upstream's key. A bare value produced without caching enters the graph through
`Cooked::of`, whose key is the content hash of its encoded bytes (domain tag `px_cook/cooked/v1`) —
so "from the cache" and "wrapped by hand" are the same thing as far as keys are concerned. A
`Cooked<T>` cannot be a `Params` field: `Params` is `Serialize + DeserializeOwned` and the key uses
`canonical_params`, which serialises, while `Cooked` implements neither. `PxKeyed::key` and
`HashField` are part of the contract, but nothing on the production path calls them — the
serialised parameter JSON is what enters the node key.

Not in a node key: the node's name, the graph's name, and the graph script's own source text.

### 3.2 An instance key (code, not data)

A **generic instance** is "reuse one declaration + one agent-written generic argument, compiled into
a content-addressed dylib". Its file name *is* its key, so the key can only be computed at run time.
The one function is `px_cook::inst::key` (`px_cook/src/inst.rs`); everything else
(`key_of`, `key_of_facts`, `info_of_facts`) forwards to it. In hash order:

| # | Input | Where it comes from |
|---|---|---|
| 1 | `px_inst/v1` | domain tag, `VERSION` in `px_cook/src/inst.rs` |
| 2 | toolchain fingerprint | `px_graph_schema::TOOLCHAIN_HASH`, computed by `px_fingerprint::toolchain_hash()` in the contract's `build.rs`: `rustc -vV` full text + `TARGET` + `RUSTFLAGS` + `PROFILE` (deliberately **not** `DEBUG` / `opt-level`, which do not move layout) |
| 3 | `decl_hash` | the declaring crate's `PX_SOURCE_HASH` (via `px_decls`); for element functions, `px_elem::DECL_HASH` |
| 4 | root rosters | `px_fingerprint::roster()` run at run time on every entry of the recipe's `roots`, from disk; labels are prefixed `<root>::` before hashing |
| 5 | `interface` | `PxOp::interface()` of the reused declaration, baked into the generated type as a `u64` literal |
| 6 | body template | the recipe's `body` text after `normalize_template` (all whitespace removed) |
| 7 | source file bytes | `<workspace root>/<source>` read raw, with a `u64` length prefix; the **path** is not hashed |

Consequence: **the build environment is part of the identity.** The same source compiled under
another profile / `TARGET` / `RUSTFLAGS` is a different instance key, so one workspace rebuild can
rotate every instance key with the sources byte-identical. At load time the mismatch is refused
(the toolchain table above), so it never answers silently wrong — but the rebuild still costs the
full re-compile and re-cook.

Text inputs are length-prefixed; the interface is written as raw 8 little-endian bytes. So identity
is **six content axes plus a domain tag**. The `op_id` and the graph program's own `PX_SOURCE_HASH`
are **not** in an instance key: they are labels for readings and errors, and two graphs that use the
same generic function and the same parameters land on the same key, the same library, and the same
artifact. The contract's own fingerprint is not an axis either — it reaches the key only inside
`decl_hash`, whose roster includes `px_graph_schema` as a path dependency.

Consequences that are easy to get wrong:

* **A comment edit changes the key.** The fingerprint hashes file bytes, and comments are bytes.
* Editing an *algorithm* changes the implementation library's `decl_hash`/`source_hash`; editing a
  *source file* changes axis 7; editing a *template* changes axis 6; adding a field to an input
  struct changes axis 5. Each rotates exactly the affected keys.
* Keys are pure functions of content: checkouts at different paths produce the same key (labels are
  `<crate directory name>/<path inside the crate>`, never absolute paths).

### 3.3 The source fingerprint

`px_fingerprint/src/lib.rs` is a plain rlib shared by the build scripts (through
`[build-dependencies]`) and by run-time key computation (through `[dependencies]`).

* A **roster** maps `label → path`; label = `<crate directory name>/<path relative to the crate>`.
  It is a `BTreeMap`, so hashing order is deterministic.
* Per crate it collects every `.rs` **and `.wgsl`** under `src/` recursively, plus the crate's
  `build.rs`. Shaders count because they are `include_str!`-ed into the binary, not resources.
* It then recurses into every **reachable path dependency** (transitive closure) named in
  `Cargo.toml`. The scan accepts any section header containing `dependencies`, so path dependencies
  declared under `[build-dependencies]` or `[dev-dependencies]` are folded in as well.
* Skipped: directories named `tests`, any directory starting with `.`, and files matching
  `*_test.rs` / `test_*.rs`. `Cargo.toml` itself is not hashed; it only declares edges.
* The hash is blake3 over, per file, the label bytes, a `0` byte, the file length as `u64`, and the
  raw bytes. The label participates, so two identical files with different names do not collide.
* Because the dependency walk is a closure, a proc-macro that generates key-contributing code (for
  example `px_derive`) is covered even though it is only a transitive dependency.

## 4. The declaration table

`px_decls/src/lib.rs` exists because the generator is `px_graphs/build.rs`, and a build script
cannot compile operator types: it only sees `[build-dependencies]`. But the generated code needs
`type Params = <path>;` and the two facts `interface()` / `decl_hash()`, which exist only where the
types are compiled. `px_decls` is a small rlib that has compiled them and hands the facts over.

```
TABLE: &[(&str, fn() -> DeclFacts)]     // 32 rows; the only list of declarations
decl(name) -> Option<DeclFacts>         // "CloudCoarse" -> facts
names() / entries()                     // derived from TABLE
SCHEMAS: &[&str]                        // px_field_schema, px_volume_schema,
                                        // px_mesh_schema, px_nurbs_schema
workspace_root()
```

`DeclFacts` holds `interface`, `decl_hash`, the three type paths (`params` / `inputs` / `payload`,
taken from `core::any::type_name`, so they are real paths, not hand-written strings), `schema` and
`module` (split out of the declaration type's own full path), and `type_name` (the declaration's own
name, used by the gate that checks a row points at the type it claims).

`px_decls` lives **outside every implementation library's roster**: adding a row to it rotates no
instance key. Putting the same facts inside `px_*_schema` would mean that every added byte there —
a comment included — rotates that schema's `SOURCE_HASH`, and with it every instance key and node
key that depends on it.

⚠ It is **not** outside `px_graphs`' roster — that roster is taken over `[build-dependencies]` and
pulls in `px_decls` and `px_cook`. Today nothing under `px_graphs/src` uses `px_local_op!`, so
`px_graphs`' fingerprint reaches no key and editing `px_decls` stays free. Adding the first local
operator to a shipped graph ends that: from then on a byte of `px_decls` rotates that node's key.

Gates (`px_decls/tests/inst_gate.rs`):

* the number of `px_op!` invocations in `SCHEMAS`' `src/` trees equals the number of `TABLE` rows —
  counting is done by `px_cook::inst_scan::count_named`, one shared lexical scanner;
* every listed name resolves through `decl()`, points back at its own schema crate, and lives in
  module `ops`;
* names are unique, and each row's `type_name` equals the name it is filed under;
* the facts are alive: `interface != 0`, `decl_hash` non-empty, and the three type paths are clean
  (non-empty, no space, no `<`).

A declaration does not have to have a preset implementation. `FieldRemap` is declared with
`LIB = "px_field_op"` but no `px_body!` exists for it there; it exists as a declaration for generic
instances, and the load gate deliberately does not list it.

## 5. Code generation (stage 1, the plan half)

`px_graphs/build.rs` never invokes cargo and never compiles anything. It reads the recipe table,
validates it, asks `px_decls` for facts, and writes three files into `OUT_DIR`.

The recipe is a **data table** in `px_graphs/src/inst_recipe.rs`, included as a module by both
`px_graphs/src/insts.rs` and the build script, and restricted to type definitions and literals.

| `InstRecipe` field | Used for |
|---|---|
| `op_id` | readings, error messages, and claiming the right body in the catalogue — **not** instance identity |
| `decl` | looked up in `px_decls`; yields `interface`, `decl_hash`, the three type paths, `schema`, `module` |
| `type_name` | the graph-side type name; the generator emits `pub struct <type_name>;` and the body refers to `&<type_name>`. The source file must define a type with that name |
| `roots` | the crates this instance links at compile time; each one goes into the generated `[dependencies]` **and** into the key (axis 4) |
| `source` | the generic-argument file (`art/inst/*.rs`); its **content** is axis 7 and it is `include!`d by the generated crate |
| `body` | the body expression with the placeholder `ARG` where the generic argument goes. `ARG` is axis 6, so the table stores the template and the generator substitutes |

Validation, each failure naming the recipe row number and op id (the build script panics, so the
error surfaces as a failed `cargo build`):

1. the table is non-empty;
2. `op_id` is unique;
3. `decl` resolves in `px_decls`;
4. every entry of `roots` is a workspace member directory;
5. `source` exists on disk and textually defines `type_name` (`struct` / `enum` / `union` /
   `trait` / `type`, comments stripped).

The generator does not compute or freeze keys: keys are computed at run time by
`px_cook::inst::key_of_facts`, so nothing in the generated output depends on the bytes of
`art/inst/*.rs`. That is what keeps a body edit from recompiling the graph program.

`InstRecipe::generated_body()` replaces `ARG` **whole-word** with `&<type_name>`, so `ARGS` is left
alone; the substituted text is what the generated crate compiles, while the unsubstituted template
is what enters the key.

What lands in `OUT_DIR` (written only when the content differs, so mtimes survive untouched edits):

| File | Content |
|---|---|
| `insts_gen.rs` | one `pub struct <type_name>;` per recipe row, an inherent `impl` with `INST_ID` / `INST_DECL` / `INST_SOURCE` / `INST_ROOTS` / `INST_BODY` / `INST_TEMPLATE`, `impl InstNode` (`info()` = `info_of_facts(...)`), `impl PxOp` (`ID`, `LIB = ""`, `SYMBOL = px_inst__<decl>`, the three types from `px_decls`, `decl_hash()` as a literal, `source_hash()` = `OnceLock` + `key_of_facts`, `render()` = key → `library_path` → file check with `missing_hint` → `ops::load_at::<Self>`). Ends with `facts_of(type_name) -> (interface, decl_hash)` |
| `elem_gen.rs` | one unit struct per element spec, `impl PxOp` with `LIB = ""`, `SYMBOL = px_inst__<ty>`, `source_hash()` = `crate::elem::source_hash_of::<px_elem::specs::<ty>>()`, `render()` = `crate::elem::render::<…>` |
| `insts_gen_catalogue.rs` | `pub static INST_CODEGEN: &[InstCodegen]` — for each instance, op id, kind (`Decl` or `Element`), source file, body, the source line of the body, and the file that line is in (for error mapping). Read by `px build` / `px run --build` and by tests; it participates in no key and is not compiled into the graph program |

`px_graphs/src/insts.rs` `include!`s `insts_gen.rs` once inside `mod generated` and re-exports the
types; `px_graphs/src/elem.rs` does the same with `elem_gen.rs`. So stage 2 uses exactly the types
stage 1 generated.

The **element** family is the one case with no graph-side generated facts table: the specs are plain
constants in `px_elem/src/specs.rs` (`ELEM_SPECS`), the script-facing value is the generated unit
struct (`elem::Constant`), and the key is computed at run time from the body file bytes. The roots
of an element instance are always `px_elem` ∪ `px_field_schema` ∪ the spec's own `roots`
(`px_elem::all_roots`), which is also where the generated `[dependencies]` come from; one library
computes both, so they cannot drift.

## 6. Stage 1 execution, stage 2 loading

`px_cook::inst::BuildGraph` owns execution. `insts::build()` (`px_graphs/src/insts.rs`) registers
one node per recipe row and one per element spec, through the same `facts()` entry point; a duplicate
op id is refused. The graph knows nothing about specific graph programs.

* `missing()` is read-only: it stats `target/pcg/inst/<key>.dll` for every node.
* `compile_missing()` / `compile_one()` generate, compile, and collect. Per instance:
  `target/jit/<key>/{Cargo.toml, src/lib.rs, build.rs}` is written, `cargo build` runs in a nested
  workspace (`[workspace]` in the generated manifest; shared intermediate artifacts in
  `target/jit/target`), and the produced dylib is copied to `target/pcg/inst/<key>.dll` next to a
  sidecar `target/pcg/inst/<key>.json` (key, op id, declaration, declaring crate, roots, source,
  template, toolchain, symbol). Cargo's stdout/stderr is passed through verbatim, and a failure
  leaves the generated crate on disk and writes no library.
* The generated crate is always package `px_inst`, so the symbol is the compile-time literal
  `px_inst__<name>` on both sides; `px_cook::inst::symbol` is the single place that builds it.
* The nested cargo is given the *outer* build's `PROFILE` / `TARGET` / `RUSTFLAGS` (and uses the
  cargo binary captured at compile time), because the toolchain fingerprint is an axis of the key
  **and** is compared at load time: if the nested build disagreed, every instance would be
  correctly rejected.

Stage 2 is a graph binary. There is exactly one cache entry, `px_cook::cached`, which computes the
node key, decodes the artifact on a hit, and otherwise calls `O::render`. For a generated instance
type, `render` derives the instance key, checks the library is on disk (otherwise it returns the
missing hint with the command to run), and calls `ops::load_at::<Self>(path, SYMBOL)`.

`px run <graph>` runs the plan first and, if anything is missing, **fails** with the command to run
unless `--build` was given. `cargo build` never compiles instances; only `px build` and
`px run --build` do. `px build` walks instances one by one so the rest still get compiled after a
failure; `px build --gc` deletes instance libraries whose key is not in the current build graph
(`--deep` also prunes `target/jit/<key>` directories, `--target` the shared intermediates).

### The load handshake

`ops::open` is the only path that loads a library, and both forms — a package name and an absolute
path (instance libraries are named by content key, so `const LIB` cannot name them) — go through it.
Given a library and a symbol, it derives the identity-symbol prefix from the symbol itself
(`<prefix>__<name>`, i.e. the *package* name, which for an instance library is `px_inst`), then:

1. resolves the file: an absolute path or anything containing a separator is used as-is; a package
   name is searched as `PX_OP_DIR`, then the executable's directory, then its parent (test binaries
   live in `deps/`), then the current directory, as `<DLL_PREFIX><name><DLL_SUFFIX>`;
2. opens it (libraries are cached per process by the name string; a process never reloads);
3. reads `<prefix>__contract_hash` and compares it with `px_graph_schema::SOURCE_HASH` — the DLL and
   the graph program must come from the same contract source, or the type layouts cannot be assumed
   equal;
4. reads `<prefix>__toolchain_hash` and compares it with `px_graph_schema::TOOLCHAIN_HASH` — a
   matching contract is not enough, since the `extern "Rust"` ABI is the compiler's;
5. looks up the operator symbol and casts the address to `Body<O>` — the single `transmute` in the
   repository, with the signature fixed at compile time by the operator's associated types;
6. for package-name lookups only, compares the library's mtime against the newest `.rs` under
   `<root>/<name>/src` and prints a warning if the source is newer. This is a warning, not a
   refusal: the identity in the key is the library's own, so the run is simply using the old
   implementation.

Failure modes and what they look like:

| What you did | What happens |
|---|---|
| built only the graph program (`cargo build -p px_graphs`) | the preset libraries are absent: the loader reports that it cannot find `px_field_op` and prints the `cargo build -p px_field_op` command. Instance libraries are caught before that, by the `is_file` check in `render`, which prints the missing key and the `px build` command |
| rebuilt a library against a changed contract but not the graph program (or the reverse) | contract mismatch, refused: the message prints both fingerprints and the command to rebuild `-p <name>` |
| compiled the instance library with a different profile/target/`RUSTFLAGS` | toolchain mismatch, refused, with both fingerprints and the instruction to rebuild with the same toolchain (nested builds take the ingredients from `px_cook`'s own build script) |
| loaded a library that predates `__toolchain_hash` | refused: no toolchain identity symbol, rebuild it |
| edited an implementation and did not rebuild | the old library is what runs; the new key is a different key, so the old artifact is simply not reused — and the stale warning fires |
| edited `art/inst/*.rs` (or an `_alg` crate) and did not rerun stage 1 | the instance key changed; the old library is an **orphan**, not a wrong hit; the new key reports as missing with the command to build it |

## 7. What is deliberately not allowed

A graph program's `[dependencies]` may contain `px_cook` (the single door: `begin`, `cached`,
`node_params`, the per-domain operator tables), `px_graph_schema`, `px_*_schema`, `px_elem`,
`px_graph`, `px-scene`, `px_shader`, `px_protocol`, `px_verify`. It may **not** contain any
`px_*_op` or `px_*_alg`, in `[dependencies]` or `[dev-dependencies]`:

* depending on an implementation library means "edit one line of an implementation ⇒ recompile and
  relink every graph program", which is the property the run-time loader exists to protect;
* depending on an `_alg` crate means the generic algorithm bodies would be part of the graph
  program's own source roster, with the same effect.

The gate is `px_graphs/tests/crate_graph.rs`, which also checks the other three lines: every
implementation library declares `crate-type = ["dylib"]`; no implementation library depends on
`px_graph` or `px_cook` (otherwise a process would hold two copies of the driver); `px_graph` does
not depend on operators; and the schema crates do not depend on operators. Its list of operator
libraries is a hand-written constant (`OPS`), and it only covers the preset libraries — the element
family's roots are checked separately in `px_graphs/tests/elem.rs`, which asserts that no root of an
element instance is `px_graph`, `px_cook`, `px_render`, `px-scene`, `px_pass`, or `px_graphs`.

`px_graph/tests/source_hash.rs` guards the fingerprint mechanism itself: every fingerprinted crate
has a `build.rs` calling the shared `px_fingerprint` entry point, no `build.rs` `include!`s shared
code, and no declaration file reintroduces a hand-written source list (`include_str!`) or a
hand-written version constant.

`px_graphs/tests/inst_gate.rs` guards the generated side: the number of rows in the recipe plus
`ELEM_SPECS` equals the number of nodes in the build graph; `INST_TEMPLATE` matches the recipe row
verbatim and `INST_BODY` is the substituted form; the symbol string is identical on both sides; and
each instance's library path is exactly `library_path(its own key)`.

## 8. Adding a new operator library

The example below adds a domain with its own schema and implementation library. The same steps that
touch `params.rs` / `ops.rs` / `px_decls` / the implementation library apply when you add one more
operator to an existing domain.

1. **Create the schema crate** `<domain>_schema` and add it to the workspace `members` (and to
   `default-members` if it belongs in the fast chain).
   * `Cargo.toml`: path dependencies on `px_graph_schema` and `px_derive` (for the derives);
     `[build-dependencies] px_fingerprint`.
   * `build.rs`: `fn main() { px_fingerprint::cargo_fingerprint_for_crate(&[]); }` — required,
     because `px_op!` expands to `env!("PX_SOURCE_HASH")`.
   * `src/lib.rs`: at least `pub mod ops;` and `pub mod params;` (a missing `pub mod ops;` shows up
     as "cannot find `Name` in `<domain>`" at the graph script, nowhere near the cause).
2. **Write the parameters** in `src/params.rs`: one struct per operator, with
   `#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]`,
   `#[serde(default, deny_unknown_fields)]`, and an `impl Default`. A field of a type that has no
   `HashField` impl must be added to `px_graph_schema/src/identity.rs` first (the closed set).
3. **Write the declarations** in `src/ops.rs`: the input structs with
   `#[derive(px_derive::PxInputs)]` (named fields; the names are what the graph script must use),
   and one `px_op! { Name, "domain.name", "<domain>_op", params::NameParams, NameInput, Payload }`
   per operator. The payload type must implement `px_graph_schema::Build`.
4. **Register every declaration in `px_decls`**: add one row per operator
   (`("Name", || facts::<domain_schema::ops::Name>())`) to `TABLE`, and add `<domain>_schema` to
   `SCHEMAS`. `px_decls/tests/inst_gate.rs` fails until the row count matches the `px_op!` count.
5. **Create the implementation library** `<domain>_op` (also a workspace member):
   * `Cargo.toml`: `[lib] crate-type = ["dylib"]`, path dependencies on `px_graph_schema`, the
     schema crate, and any `_alg`/GPU helper crates; `[build-dependencies] px_fingerprint` and the
     same one-line `build.rs`.
   * `src/lib.rs`: the modules plus `px_graph_schema::px_impl_lib!();` exactly once.
   * `src/ops/<operator>.rs`: one file per operator, each containing one
     `px_graph_schema::px_body! { Name, |p, i| crate::ops::<operator>::eval(p, i) }` (one
     `px_body!` per module, because the exported Rust name is fixed). The exported symbol becomes
     `<package>__Name`, which must equal the declaration's `SYMBOL`.
6. **Add the library to the gates' hand-written lists**: `OPS` in
   `px_graphs/tests/crate_graph.rs`, `FINGERPRINTED_CRATES` and the implementation-library list in
   `px_graph/tests/source_hash.rs`, and the load list plus identity list in
   `px_graphs/tests/ops_load.rs`. These lists are hard-coded, so a new library is unchecked until it
   is named there.
7. **Expose the declarations to graph scripts**: re-export the schema's `ops` module from
   `px_cook/src/lib.rs`. Do **not** add the implementation library to `px_graphs/Cargo.toml`.
8. **If the library also ships a generic instance** (a reusable algorithm + an agent-written
   generic argument): put the shared algorithm in a `<domain>_alg` rlib (so the instance key can
   cover its roster), write the generic argument file under `art/inst/`, add one `InstRecipe` row in
   `px_graphs/src/inst_recipe.rs` (with `ARG` in `body`), and then run `cargo build` to regenerate
   `OUT_DIR`. `px list` shows the new key, `px build` compiles it, `px run <graph> --build` does both
   stages.
9. **If the library ships an element function** instead: add one line to the spec table in
   `px_elem/src/specs.rs`, two files — `px_elem/src/<name>.rs` (parameter and input types) and
   `px_elem/body/<name>.rs` (the per-cell function, outside `src/` so editing the algorithm does not
   rotate the author-side crate's fingerprint) — and a re-export of the parameter/input types in
   `px_elem/src/lib.rs` (the body files refer to them as `px_elem::<Type>`). No generator change and
   no `px_decls` row are needed.
10. **Verify**: `cargo build` (compiles everything, compiles no instance), then the affected gates
    (`cargo test -p px_decls -p px_graph`, `cargo test -p px_graphs --test inst_gate --test
    ops_load --test crate_graph`), then `px list` and one graph run.
