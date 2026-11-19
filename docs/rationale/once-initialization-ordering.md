# `Once<T>` initialization ordering

## Why this is hard
`kernel::once::Once` is a lock-free write-once cell built on `UnsafeCell<MaybeUninit<T>>` and an `AtomicU8` state, so soundness lives entirely in the state protocol, not in the type system.

## Invariants relied on
- States are `UNINIT`, `INITIALIZING`, `READY`. Only the thread that wins the compare-exchange `UNINIT` to `INITIALIZING` (`AcqRel`/`Acquire`) may write the value, so it has exclusive access (`store_value`).
- `READY` is stored with `Release` only after the write; a reader that observes `READY` with `Acquire` sees a fully initialized `T` and no further mutation ever occurs (`stored_value`).
- `Sync` requires `T: Send + Sync` because references are shared across threads after init; `Send` requires `T: Send` because the winning thread may be any thread.
- Losers in `get_or_init` spin on `Acquire` loads of the state until `READY`; `call_once` panics if called twice.

## What breaks if changed
Weaker orderings let a reader see `READY` before the value is visible. A second writer path (or a reset) makes the shared reference returned by `stored_value` alias a mutation.

## Evidence
The unit tests in `kernel/src/once.rs` (`cargo test -p kernel -- --test-threads=1`).

## Status
verified.
