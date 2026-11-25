# Actor Pub/Sub v1 parity target

Cranelift: this backend must consume the same source contract rather than defining a Rust-specific pub/sub API. Until ActorGroup/Topic lowering exists, reject unsupported guest constructs. Runtime integration must preserve bounded admission, generation fencing, and actor cleanup.

Normative contract: `contracts/v1/actor-pubsub-v1.json`.

No `define publisher` or `define subscriber` language forms are introduced. The shared source model is `ActorGroup`, `Topic<T>`, `Subscription<T>`, and `PublishReceipt`; actors may belong to multiple groups simultaneously, and group membership is never lifecycle ownership.

Golden source parity fixture: `contracts/v1/fixtures/actor-pubsub-v1.ores`.
The backend must either execute this fixture with the normative output or reject the unsupported source surface before lowering; host-only scaffolding is not parity evidence.
