# TODO

## Future Registry — Phase 2

After phase 1 (auto-remove orphaned completion futures on task termination), the
remaining known issue is that the `FutureRegistry` is a fixed-size arena of 1024
slots. High task churn can still exhaust it if futures accumulate faster than they
are cleaned up.

### Open questions / next steps

- **Registry size**: Is 1024 slots sufficient in practice, or should it be larger /
  dynamically sized?
- **Leak audit**: Are there other future types (e.g. `TimeFuture`) that can be
  orphaned and need similar cleanup?
- **`wait_future` error handling**: Some call sites do `.unwrap()` on the result of
  `wait_future`. If a future is consumed before the waiter resumes this panics —
  consider making those call sites handle `Err` gracefully.
- **`schedule()` registry-full path**: When `future_registry.register()` returns
  `None` (arena full), the future handle is silently dropped and the task runs
  without a completion future. This should either be an error or block until a slot
  is free.
