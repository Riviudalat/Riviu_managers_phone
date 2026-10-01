# Rust review checklist — local edition

Adapted from Apollo's Rust guidance after checking compatibility with Riviu. This
is a focused review aid, not an exhaustive language reference or automatic fix list.

## Types and ownership

- Prefer small explicit domain types to ambiguous flags/strings; use Option for missing
  evidence and Result for errors. Unknown is not false; an enum variant is not proof.
- Borrow slices/str for read-only APIs when ownership is unnecessary. Clone only for an
  intentional owner/lifetime boundary. Avoid replacing clarity with blanket lifetimes.
- `&T: Send` requires `T: Sync`; `&mut T: Send` requires `T: Send`.
  `Arc<T>` is not unconditional thread safety: its Send/Sync require suitable T bounds.
  Mutex protects access, not every invariant; Rc/RefCell do not become cross-thread safe
  merely because their contents are wrapped in another object. Verify actual compiler bounds.
- Prefer static dispatch for fixed hot paths and traits for real injected boundaries.
  Do not spread generics/dyn dispatch solely to follow a table; measure cost when relevant.

## Async, storage and effects

- Name ownership of every spawned worker and its shutdown/join path. Use bounded queues,
  one deadline across retries and cancellation checks before effects, not arbitrary sleeps.
- Never hold a synchronous mutex guard or SQLite transaction across awaited network/device
  work. Use the existing StorageExecutor and controller admission, not a new side channel.
- Retrying a read can be safe; replaying a sent input may duplicate a public effect.
  Preserve typed uncertainty and durable intent. A task abort is not a rollback.
- Lock ordering, stale revisions, lease fencing and partial writes need explicit negative
  tests. Test virtual clocks when possible; host load thresholds are not business invariants.

## Errors, performance and unsafe

- Add context without leaking tokens, passwords, private text or database contents.
  Keep machine-readable classification independent of translated user-facing wording.
- Optimize after a trace/benchmark establishes the bottleneck. Prefer existing tools;
  profiler/package installation requires separate scope. Do not use cache-warm numbers
  as a claim about clean builds or every phone.
- `unsafe` needs a complete SAFETY argument: pointer validity, lifetime, initialization,
  allocation bounds, alignment, aliasing, overlap requirements and thread behavior as
  relevant. Non-null and aligned alone do not make copy_nonoverlapping safe.
- Follow the pinned toolchain. Do not introduce APIs that require a newer compiler without
  a separate supported-toolchain decision. Examples from Rust1.96+ are not valid merely
  because upstream skill recommends them.

## Tests and documentation

- Behavioral test first, then smallest fix. Do not delete existing implementation to
  satisfy a generic TDD prescription. Preserve unrelated user changes.
- Test helpers must not compute expected using the same potentially faulty code.
  Inversion on an isolated copy should fail the intended assertion, not compilation.
- Update owner docs, not several snapshots of the same constants. Report real command
  exit and scope; unit success does not certify native app, device or installer.
