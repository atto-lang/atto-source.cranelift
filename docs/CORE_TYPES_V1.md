# core/types v1 — Cranelift backend

This compiler owns its own `core/types.atto`; no backend is required to import byte-identical sources from another compiler or from a shared repository.

Public contract: logical value bit widths for bool, int and byte; signed-64-bit int min/max; `fits_i8/i16/i32/i64/u8/u16/u32/u64(int)` range admission. These functions cannot enforce real sized integer types, cast values, describe an ABI layout, or reflect over arbitrary objects. `fits_u64` only checks whether a *signed int* input is representable.

`void` is valueless and not a first-class `Unit`. Opaque `TypeId`, static `TypeInfo<T>`, real `Unit`, and checked sized-integer representations must come from explicit compiler mechanisms in separate work; no invented reflection or host fallback.

Conformance fixture: compile/link the actual `edge-positive.atto` and execute the guest (exit 0). Reject `void-value-negative.atto` at compile time and emit no guest artifact. Report exact source/head SHA and nonzero runner steps. **NOT_EXECUTED:** this is a staged library surface, not verified compiler/AOT parity.

## Namespace-only core source

Canonical `use namespace types;` file header plus public top-level functions; no `define module` wrapper. Consumers still explicitly import the symbols. Compiler parse, namespace source identity, static imports, and executed linked guests require separate proofs. Until the current PR compiles and runs, namespace guest parity remains NOT_EXECUTED.

## Exact-head executable compiler admission gate

The `core-types-native-compiler-admission` Actions job attempts to consume the unchanged `core/types.atto` with this repository's **real** compiler. The source-only oracle is insufficient. The job fails if the compiler rejects any required syntax; no host-model fallback or skip is permitted. This backend currently has an independently diagnosed namespace/boolean/type-lowering gap, so a green compiled-parity result is **not** claimed. The initial test proves stage-0 parser and IR only; real JIT function invocation and NativeAOT not proven if it genuinely executes and succeeds, and deeper runtime and 8 compiler-negative fixture tests remain separate work. Do not merge until the real compiled positive/negative and target-appropriate execution gates pass on this exact PR head.

## Cranelift source-namespace stage-0 admission

The bounded Rust front end now accepts a single `use namespace types;` header ahead of a scalar `fnc main`. Three backend-specific `.atto` fixtures test real native JIT execution of a return-64 function and reject duplicate/late headers with `CRANELIFT_NAMESPACE_DUPLICATE` and `CRANELIFT_NAMESPACE_HEADER_ORDER`. This does not grant cross-file imports, multiple exported functions, boolean predicates or general library linkage; the full `core/types.atto` remains a separate deliberately strict compilation gate. The current workflow has no executed evidence on its exact head, so full guest and AOT remain `NOT_EXECUTED`.
