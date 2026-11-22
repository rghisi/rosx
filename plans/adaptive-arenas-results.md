# Adaptive arenas — measurement results

> Companion to `plans/adaptive-arenas.md`. Step 0 (baseline on the unchanged
> tree) and Step 5 (after) measured with the same disposable harness (a
> `#[cfg(test)]` counting `#[global_allocator]` plus `std::time` loops in
> `kernel/src/arena_bench/`, kept as a patch in the scratchpad, never
> committed). Host: 4 cores. Byte counts are the evidence; timings only show
> relative cost and are the median of 5 runs.

## How to read the tables

- **EAGER** — live heap bytes requested right after constructing each manager
  with no entries (the boot cost the arena pays up front). `IpcManager` is
  composite (it owns `MailboxManager` + `IpcDataManager`), so its row is the
  union, not additive with them.
- **CURVE** — live heap bytes at N entries added through the manager's own
  `add` path (includes per-entry payload: the future `Box`, the 32-byte data
  payload, the `String`/BTreeMap key, etc.). The arena delta is the slope from
  the eager base; a flat line at the eager base means the entry cost nothing.
- **SIZE** — `size_of` for the slot/entry types.
- **TIME** — nanoseconds, median of 5 runs, dev and release profiles.

---

## Step 0 — baseline (before)

Commit `origin/main@1f10d56` (arena code identical to `7c4a5f1`).

### Eager footprint (bytes)

| Manager | live bytes |
|---|---:|
| `FutureRegistry` | 61 440 |
| `IpcDataManager` | 11 264 |
| `MailboxManager` | 9 216 |
| `TaskManager` | 3 072 |
| `IpcManager` (composite: mailboxes + data + bindings + connections) | 27 648 |

The five eager arenas total 61 440 (futures) + 11 264 + 9 216 + 3 072 +
27 648 = 112 640 B ≈ 110 KiB before any entry exists. The future registry's
61 440 = 60 KiB is the arena (`items` 16 384 + `generations` 2 048 +
`VecDeque` free list 2 048 = 20 480) plus the two parallel tables
(`owners` 16 B × 1024 = 16 384 + `parents_of` `Vec` header 24 B × 1024 =
24 576, the latter carrying no inner allocation when empty).

### Slot / entry sizes (bytes)

| Type | size |
|---|---:|
| `Handle` | 4 |
| `FutureOwner` | 16 |
| `ParentLink` | 8 |
| `DataObject` | 40 |
| `Task` | 96 |
| `Box<Task>` | 8 |
| `TaskName` | 40 |
| `Option<Box<dyn Future + Send + Sync>>` | 16 |
| `GenerationalArena` struct (the three containers) | 80 |

### Growth curve (live bytes at N entries)

| Arena | 0 | 32 | 100 | 256 | cap |
|---|---:|---:|---:|---:|---|
| futures (registry) | 0* | 448 | 832 | 1 344 | 1 024 (4 416 at 1 024) |
| data objects | 0* | 2 288 | 6 768 | 16 160 | 256 |
| mailboxes | 0* | 0 | 0 | 0 | 256 |
| bindings (+server mailbox) | 0* | 1 878 | 5 998 | 14 738 | 256 |
| connections (+client mailbox) | 0* | 2 208 | 7 328 | 18 096 at 255 | 255 |
| tasks | 0* | 3 072 | 9 600 | 24 576 | 256 |

\* measured inside a window whose base is the manager's own eager cost, so
"0" here means "no arena growth beyond what the entry itself costs". The
futures row grows only by the `Box<dyn Future>` payload (the arena slots were
all preallocated eagerly); mailboxes stay flat because `Mailbox::new` holds an
empty `VecDeque` (no heap); the rest grow by their entry payload plus BTreeMap
side-index nodes, none of which the arena avoids today because every slot is
already resident.

### Timings (ns, median of 5)

dev profile:

| metric | raw arena | registry |
|---|---:|---:|
| borrow/get @16 / @128 / @1024 | 25 / 23 / 23 | 35 / 33 / 33 |
| add/register base 0 / 16 / 128 / 512 | 80 / 75 / 73 / 73 | 256 / 246 / 242 / 243 |
| remove/consume from 16 / 128 / 1024 | 68 / 60 / 59 | 569 / 2 248 / 15 563 |
| churn (ns per op) | 89 | 6 293 |
| worst single add / register | 6 760 | 9 842 |
| `visit` @16 / @1024 | 12 063 / 19 660 | — |
| `any` absent @16 | 15 323 | — |
| `reclaim_group` of 8 / of 512 | — | 19 421 / 4 124 727 |

release profile:

| metric | raw arena | registry |
|---|---:|---:|
| borrow/get @16 / @128 / @1024 | 1 / 1 / 1 | 3 / 3 / 3 |
| add/register base 0 / 16 / 128 / 512 | 9 / 6 / 5 / 4 | 26 / 23 / 20 / 21 |
| remove/consume from 16 / 128 / 1024 | 7 / 5 / 5 | 39 / 78 / 317 |
| churn (ns per op) | 14 | 146 |
| worst single add / register | 89 | 7 416 |
| `visit` @16 / @1024 | 771 / 1 091 | — |
| `any` absent @16 | 472 | — |
| `reclaim_group` of 8 / of 512 | — | 1 151 / 95 492 |

Reading of the baseline: `add`/`borrow` are already O(1) and flat with
occupancy (the eager slots are all warm), so there is nothing to win on the
hot path — the cost is entirely the 110 KiB resident up front. `reclaim_group`
is an O(capacity) scan of `owners`, hence the 4 ms (dev) / 95 µs (release) at
512 live futures even though only the owning group's slots are freed.

### Real boot footprint

From `tests-integration/tests/boot_footprint_probe.rs` (disposable): boot to
the shell prompt, `mem` three times; every sample identical each run.

| Arch | idle `used` KiB at the shell prompt |
|---|---:|
| x86_64 | 1 696 |
| x86_32 | 1 455 |

Timing preconditions for every run: `load average` ≈ 0.1–0.5 on 4 cores,
`pgrep -x qemu-system-x86` empty.
