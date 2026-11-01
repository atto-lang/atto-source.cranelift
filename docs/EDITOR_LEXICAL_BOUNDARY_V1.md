# Cranelift editor lexical contract v1 boundary

The native Cranelift **scalar-only** frontend is not a general Oreslang
compiler and must not claim full parity with Java, LLVM, WASM or .NET.

The parameter-name gate copies only the **93 spellings** of the historical
Java lexer reserved-word inventory pinned at Java Lexer.java Git blob
`bba9e091c0ad1c9b75e0c61e9dd173fcc18a236a`.
The source contract is `oreslang-common-source` draft PR #201,
`contracts/editor-syntax/v1/editor-lexical-contract.json`
(Git blob `e9dbf3b6fad5562b7ca6df631ab3c45f478480ba`).

- Parser unit tests reject all 93 words in scalar parameter declarations
  and admit positive boundaries such as `eqx`, `island`, `nullish`.
- Native CLI tests actually enter the compile/JIT path for representative
  rejects and allowed scalar controls.
- Cranelift **supports** signed scalar `%` with a proven nonzero literal
  divisor but **rejects** `^`, Symbols, actors, concurrency, ownership,
  `nb-select`, and execution-domain source forms.
- Cranelift currently has an ASCII-only native scanner, no Java-compatible
  Symbol NFC/Unicode handling, and no lexical-v1 JSON probe interface.
  A negative admission is not a substitute for a native token-by-token
  comparison against the pinned 34-case differential corpus.
- The test plan status remains **NOT_EXECUTED** until genuine exact-head
  Rust test and native execution steps run. Synthetic source fixtures,
  unexecuted Actions jobs, and green editor tests are not compiler receipts.

Do not enable syntax support by silently translating or ignoring unsupported
constructs. Preserve the distinct Cranelift identity in the six-backend
admission matrix; do not alias Forge or LLVM.
