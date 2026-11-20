# Trusted scalar module linking — bounded v0 subset (draft)

**Status:** source-implemented candidate, **NOT_EXECUTED** on the current
compiler branch. This is not full AttoLang import or native-call ABI support.

## Scope

Entry source has one or more leading named `import foo from "...";` lines
followed by strict pure signed-i64 functions, including a zero-argument
`main`. Imported source files use the existing bounded scalar function
parser. Each named import must resolve to a `pub fnc` declaration. Local
private functions and unimported public functions do not become ambient
symbols. Named helper calls are **statically lowered to the existing
checked Cranelift scalar IR**, which generates native machine code.

The source authority is the nearest ancestor `.ores.pkg.toml` containing
a `[package]` name. The subset admits `+path/to/file.atto` relative to
that package and `./path/to/file.atto` relative to the entry source.
The retired `+/...` spelling, `../`, absolute/bare/ambient imports,
nested package markers, symlinks, missing sources, private/missing exports,
duplicate local bindings, late imports, and unsupported source forms
reject before generating CLIF. The CLI reuses its Unix descriptor-relative
no-follow opening for imported files, with eight-import, 16-KiB-per-import
and existing native AST/evaluation budgets. Host package parsing is
intentionally restricted; it is **not a general TOML package manager**.

## Deliberate unsupported features

- This is **not** a first-class native function ABI or separate Cranelift
  JIT `call` instruction. Parameter expressions are checked static
  substitutions under the existing once-only *linear-use* contract.
  An imported `twice(x) { return x + x; }` still rejects with
  `E_PIPE_NONLINEAR_BINDING_UNSUPPORTED`. Do not claim all three
  positives in [conformance #233](https://github.com/atto-lang/atto-platform-conformance/pull/233)
  until it actually executes, not just parses or gets statically simplified.
- No mutable locals, recursion, cross-package dependencies, trait imports,
  aliasing, nested module imports, package lock/archive resolution,
  source entrypoints with runtime parameters, general `pub[pkg]` visibility,
  namespace imports, generic functions, standard-library I/O or AOT.
- No imported source can run in the experimental `isolate` command.
  The native JIT runs **trusted** source in the host process and is
  **not an untrusted TenantVM isolation boundary**.
- Source-mode profile/version selection and comprehensive source-snapshot
  atomicity need separate review before enabling untrusted usage. A
  checked scalar compiled guest is not proof of secure multi-tenancy.

## Required verification and integration gates

Run real `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets
-- -D warnings` and `cargo test --locked --all-targets` on **this exact
current PR head/tree**, including the new `linked_scalar_module_native`
integration executable. A true receipt requires native `run` -> `42\n`,
`ir` -> verified CLIF, parser-only `check` with no output, and negative
`check/ir/run` outputs with no CLIF/guest stdout. Record physical nonzero
Actions steps; a zero-step job is `NOT_EXECUTED`.

This branch intentionally stacks on [Cranelift #58](https://github.com/atto-lang/atto-source.cranelift/pull/58)
because that PR supplies the checked `Program.locals` scalar model and
pure-function parser. It must not merge separately ahead of #58 without
an exact-tree integration/rebase test. Native ABI and recursive call-graph
support remain on [Cranelift issue #71](https://github.com/atto-lang/atto-source.cranelift/issues/71).
