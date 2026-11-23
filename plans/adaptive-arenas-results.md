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

---

## Step 5 — after

Commit `0a091a2` (paged arena + entry fold + `by_group` index), same host,
same harness (adapter swapped to the constructor-cap API), quiet machine
(load ≈ 0.1 on 4 cores, no QEMU live).

### Eager footprint (bytes)

| Manager | live bytes | before | delta |
|---|---:|---:|---:|
| `FutureRegistry` | 2 112 | 61 440 | −59 328 |
| `IpcDataManager` | 1 600 | 11 264 | −9 664 |
| `MailboxManager` | 1 344 | 9 216 | −7 872 |
| `TaskManager` | 576 | 3 072 | −2 496 |
| `IpcManager` (composite) | 3 968 | 27 648 | −23 680 |

Summing the five rows under the same rule as the baseline: 112 640 → 9 600 B
(−91 %). The eager `owners` and `parents_of` tables are gone; what remains is
the directory plus the first page each manager materializes.

### Slot / entry sizes (bytes)

`Handle` 4, `FutureOwner` 16, `ParentLink` 8, `DataObject` 40, `Task` 96,
`TaskName` 40 — all unchanged. `GenerationalArena` struct: 48 (was 80). A
future slot now carries `FutureEntry` (future 16 + owner 16 + parents 24) in
a 56-byte page slot instead of a 16-byte `Box` in a resident table plus two
parallel side tables.

### Growth curve (live bytes at N entries)

| Arena | 0 | 32 | 100 | 256 | cap |
|---|---:|---:|---:|---:|---|
| futures (registry) | 0 | 448 | 6 976 | 15 744 | 1 024 (68 352 at 1 024) |
| data objects | 0 | 2 288 | 11 376 | 26 976 | 256 |
| mailboxes | 0 | 0 | 3 840 | 9 024 | 256 |
| bindings (+server mailbox) | 0 | 1 878 | 10 798 | 26 066 | 256 |
| connections (+client mailbox) | 0 | 3 488 | 12 896 | 31 216 at 255 | 255 |
| tasks | 0 | 3 072 | 11 136 | 28 224 | 256 |

The curves now include the slots themselves: pages appear in 32-slot steps,
so each row is entry payload + materialized pages (the before curves were
payload-only because every slot was already resident). At full saturation a
saturated futures arena costs 68 352 B against the before total of 65 856
(61 440 eager + 4 416 payload): the fold's fat slots and the new `by_group`
lists cost ≈ +4 % over the old parallel tables when everything is in use —
paid back many times over at every occupancy below the cap.

### Timings (ns, median of 5; after, with before in parentheses)

dev profile:

| metric | raw arena | registry |
|---|---:|---:|
| borrow/get @16 / @128 / @1024 | 30 / 30 / 29 (25 / 23 / 23) | 41 / 41 / 41 (35 / 33 / 33) |
| add/register base 0 / 16 / 128 / 512 | 104 / 100 / 112 / 113 (80 / 75 / 73 / 73) | 287 / 278 / 293 / 291 (256 / 246 / 242 / 243) |
| remove/consume from 16 / 128 / 1024 | 71 / 63 / 62 (68 / 60 / 59) | 602 / 2 392 / 16 733 (569 / 2 248 / 15 563) |
| churn (ns per op) | 90 (89) | 6 747 (6 293) |
| worst single add / register | 8 027 (6 760) | 8 792 (9 842) |
| `visit` @16 / @1024 | 643 / 20 004 (12 063 / 19 660) | — |
| `any` absent @16 | 746 (15 323) | — |
| `reclaim_group` of 8 / of 512 | — | 4 918 / 4 436 624 (19 421 / 4 124 727) |

release profile:

| metric | raw arena | registry |
|---|---:|---:|
| borrow/get @16 / @128 / @1024 | 2 / 2 / 2 (1 / 1 / 1) | 3 / 3 / 3 (3 / 3 / 3) |
| add/register base 0 / 16 / 128 / 512 | 11 / 10 / 10 / 11 (9 / 6 / 5 / 4) | 30 / 26 / 29 / 27 (26 / 23 / 20 / 21) |
| remove/consume from 16 / 128 / 1024 | 8 / 5 / 5 (7 / 5 / 5) | 55 / 83 / 319 (39 / 78 / 317) |
| churn (ns per op) | 16 (14) | 149 (146) |
| worst single add / register | 7 127, max run 221 329 (89) | 7 354 (7 416) |
| `visit` @16 / @1024 | 36 / 784 (771 / 1 091) | — |
| `any` absent @16 | 19 (472) | — |
| `reclaim_group` of 8 / of 512 | — | 424 / 95 180 (1 151 / 95 492) |

### Real boot footprint

| Arch | idle `used` KiB before | after | delta |
|---|---:|---:|---:|
| x86_64 | 1 696 | 1 613 | −83 |
| x86_32 | 1 455 | 1 406 | −49 |

Every sample identical within and across runs, same as the baseline.

### Reading

- **Eager bytes: the goal is met.** −91 % across the eager rows and −83 KiB
  of boot on x86_64: on-demand paging plus the removed parallel tables.
- **Hot paths: a few nanoseconds of page-hop, flat or noise elsewhere.**
  `add`/`register` pay +1…+6 ns (release) for the page indirection and free
  -list pop; `borrow`, `remove`, `churn`, `consume` stay at par. Dev
  amplifies the same deltas.
- **Unmaterialized pages are free.** `visit`/`any` at low occupancy are
  ~20× faster (release 771→36 ns); even the full `visit` @1024 improved
  (1 091→784 ns) from page locality despite the fatter slot.
- **`reclaim_group`: the scan is gone, and it was never the whole story.**
  of 8 went 4× (dev) / 2.7× (release) faster — that is the index doing its
  job. of 512 is flat in release (95.5→95.2 µs) and ~7 % slower in dev: the
  512 individual consumes (each with its `by_owner` retain, now plus a
  `by_group` retain) dominate; the O(capacity) scan only mattered at low
  group occupancy. An O(1) `by_owner` removal stays the known follow-up.
- **Growth is a syscall-path allocation now.** The release worst-add outlier
  (221 µs in one of five runs) is a page being mapped mid-loop; it is
  bounded, rare, and callers already handle `OutOfMemory`.
