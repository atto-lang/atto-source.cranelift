# Bounded, trusted native module calls — Cranelift v0 draft

**Status: implementation candidate, NOT EXECUTED on this PR head.**
This is independently compiled native scalar function code inside a trusted
single-OS-process Cranelift JIT, **not** an untrusted TenantVM sandbox, an
AOT object compiler, or complete AttoLang module/import support.

## Source graph and authority

An entry `.atto` source may use up to eight leading statically named
`import helper from "+path/to/unit.atto";` or importer-relative
`"./path/to/unit.atto"` forms, optionally `as local_name`, followed
by a bounded set of scalar functions. Import resolution requires the
nearest modern `.ores.pkg.toml` marker with the supported schema and
one package name. The entry and every selected helper must remain within
the package authority and may not traverse nested package markers.
Imported helpers must be declared `pub fnc`. Private/unimported
definitions do not gain ambient link visibility.

The entry parser rejects retired `+/`, `..`, absolute/bare/ambient
imports, illegal suffixes, symlink sources, invalid/missing exports, late
imports, duplicate local aliases, malformed markers and unsupported
block-comment header ambiguity before CLIF or JIT code allocation.
There are eight import edges, 16 KiB per imported unit, 4 KiB marker,
16 total functions, 4 parameters and 4 locals per function, 32 lexical
expression levels, 4096 total AST expression nodes, and 16 call-graph
levels. All source graph processing is Unix-only until equivalent
ancestor no-follow file-read proofs are implemented on other targets.

## Native ABI and errors

The new `src/linked_native.rs` first validates all selected function
bodies, including dormant helpers, for bound symbols, call arity, forward
local references and call graph cycles. Every helper is then declared
and emitted **as its own native CLIF/JIT function**, not as an inlined
substitution. Each helper has a signed-i64 argument list and returns
`(value: i64, status: i32)`. The existing stable host entry ABI remains
four padded signed-i64 slots, an output pointer, and an `i32` status.

Arguments evaluate left to right, exactly once, before invoking a helper,
even when its corresponding parameter is unused. A helper may read one
parameter multiple times without duplicating the caller expression.
`val` initializers execute eagerly and exactly once in lexical order.
Checked `+`, `-` and `*` return `status=1` on overflow. A nonzero
status from any call propagates unchanged through its callers; entry
`*out` is never written on failure. No host-language interpreter,
unchecked wrapping arithmetic, or fallback from `ir` into JIT allocation
is allowed. Recursive call graphs and unsupported general types fail
closed before code generation.

## Executable acceptance targets — unverified

- Canonical `+path`, importer-relative and `as` aliased public helper:
  `check` successful, `ir` emits verified CLIF, `run` returns `42\n`
- `twice(21)` with `return x+x`: native `call` instruction, guest
  output `42\n`; this requires actual independent native helper ABIs
- Nested helpers and lexical `val` initializers: result `42\n`
- Overflow in call argument, callee body, discarded parameter argument
  or unused `val` initializer: no stdout or entry output, controlled
  signed-i64 overflow status
- Compile negatives: invalid/missing/private/duplicate imports, bad
  call arity, unbound dormant helper, recursion, symlink/path escape,
  unsupported no-follow platform and import size/graph budgets

The portable target corpus is
[conformance #233](https://github.com/atto-lang/atto-platform-conformance/pull/233).
Its historical `fixtures/import-v1` predecessor uses retired `+/` and
must remain as a distinct legacy profile.

## Build, security and integration gates

Requires **actual** source-authorized native execution of
`cargo fmt --all -- --check`,
`cargo clippy --locked --all-targets -- -D warnings`, and
`cargo test --locked --all-targets` (including the newly added actual
CLI `check`/`ir`/`run` tests). Record exact current source SHA/tree,
nonzero job steps, compiler/toolchain, complete guest stdout/stderr,
positive and compile-negative tests. Static code review or clean Git
blobs do not count as compiled guest PASS.

The source repository currently explicitly denies all external private
source exports via `.github/private-ci-approved-repositories.txt`.
There is **no** authorization to mirror private source into another
organization without independent security approval and the reviewed
publisher workflow. Historical CI on #58's different Git tree cannot
certify this branch.

This PR is **stacked on Cranelift #58** and must be integrated only after
its overlap and independent-review gates, with full exact-head retests.
General imports, namespace/module libraries, arbitrary types, actors,
untrusted tenant isolation, recursive native calls, task effects,
channel I/O, and full AOT remain separate work.


## Standalone multi-function source (candidate, NOT EXECUTED)

The same checked independent function ABI is now attempted for **unimported**
standalone `.atto` sources containing two or more parsable scalar `fnc`
definitions when the stage-0 single-main frontend rejects the input.
No `.ores.pkg.toml` marker is needed without imports. Existing successful
single-function scalar and pipeline paths remain unchanged. No source-language
interpreter or substitution is used for the new branch.

```atto
pub fnc twice(int value): int { return value + value; }
pub fnc main(): int { return twice(21); }
```

The added real CLI/JIT fixtures require `check`, `ir` (with actual `call`
and separate `main`/`twice` CLIF definitions), and `run` returning 42;
dormant unbound helpers, cycles, duplicate names, and overflow must fail
without CLIF output or a success-value print. These are source assertions
only until exact-head native Cargo and guest execution have run.

`isolate` still rejects linked native programs; this change does not add
an untrusted TenantVM sandbox, AOT or actor support.
