# VFS Iteration 1 — Plan Validation (binding resolutions)

> **Date:** 2026-10-17. **Validator:** architect (read-only; this file is the only write).
> **Plan under validation:** `plans/vfs-iteration-1.md` (273 lines).
> **Tree under validation:** branch `vfs-first-iteration`, HEAD `bb8c09f` ("Record VFS iteration 1 implementation plan") on code baseline `2603690` ("Add IPC Data Transfers") — confirmed via `.git/HEAD:1` and `.git/logs/HEAD:279-284`.
> **Locked inputs honored:** the seven user-locked decisions and repo law (no comments, unprefixed commits, one commit per step) are **not reopened**. No amendment below changes any locked decision, the six-step ordering, or any wire byte offset.

---

## A. §0 context-map re-verification (read-only, against the tree at HEAD)

Every claim spot-checked by the delegation, with evidence. "exact" = claim and tree agree on fact and line; "drift" = fact correct, line citation wrong (corrected in AM-4); "STALE" = claim factually wrong (corrected in AM-3/AM-4).

| # | §0 claim | Evidence | Verdict |
|---|---|---|---|
| 1 | `MAX_DATA_BYTES = 64 * 1024`, `DATA_BUDGET_BYTES = 256 * 1024` at `:8-9` | `kernel/src/ipc/data_manager.rs:8-9` | exact |
| 2 | size check strict `>` at `:40` (exactly 65536 accepted) | `data_manager.rs:40` (`if payload.len() > MAX_DATA_BYTES`); green test `write_over_max_data_bytes_is_too_large` :206-215 accepts `MAX_DATA_BYTES` | exact |
| 3 | per-writer budget `:43-46`; arena-full → `BudgetExceeded` `:53` | `data_manager.rs:43-46`, `:53` | exact |
| 4 | `write` accepts empty payload (no lower bound) | `data_manager.rs:34-57` — only upper-bound check; `Vec::from(&[][..])` legal | exact |
| 5 | `locate` rejects writer with `NotReadable` `:65-66` | `data_manager.rs:65-66`; test :280-284 | exact |
| 6 | `copy_out` cursor-advancing, 0 = end `:71-80` | `data_manager.rs:71-80`; tests :180-203 (incl. empty-dst → 0) | exact |
| 7 | `release` writer-only, non-writer → `NotFound` `:88-90` | `data_manager.rs:88-90`; test `release_by_non_writer_is_not_found` :269-277 | exact |
| 8 | `gc_connection` reclaim+refund `:98-114`, fires on explicit disconnect | `data_manager.rs:98-114`; call site `kernel/src/ipc/ipc_manager.rs:123` | exact |
| 9 | `write_data` `:169-179`; `read_data` membership gate `:192`; `release_data` `:198-204` | `ipc_manager.rs:169-179`; `:192` (`if caller != conn.client_task && writer != conn.client_task` → `NotReadable` :193); `:198-204` | exact |
| 10 | `ipc_data_write` `usrlib/src/syscall.rs:161-175`; `ipc_data_read(token: usize, dst)` `:177-189`; `wait_for_message` `:42` | `syscall.rs:161-175`, `:177-189` (bare `usize` token, `Ok(raw)` on `is_token`), `:42-55` | exact |
| 11 | `ReceivedMessage::{connection,data}` `usrlib/src/ipc.rs:19-25`; `ipc_send` `:121`; receive/accept `:139, :151` | `ipc.rs:19-25` ✓; `ipc_send` is `syscall.rs:121-124` and receive/accept are `syscall.rs:139-149, :151-159` — the bare `:121/:139/:151` cites resolve to **syscall.rs**, not ipc.rs (ipc.rs:121 is inside `receive_data`). Facts correct | exact (file attribution contextual) |
| 12 | `UserConnectionHandle` `Copy+Clone+PartialEq+Eq`, `packed()` `pub(crate)`, no `Ord`, `:38-55`; Display `"{:08x}"` `:63-67` | `usrlib/src/handle.rs:38-55` (derives `:38`, `pub(crate) fn packed` `:44`, no `Ord` derive anywhere in file); Display `:63-67` | exact |
| 13 | `UserDataHandle` release idempotent via `take()` `:122-133`, Drop `:136-140`, Display hex/"released" `:151-157` | `handle.rs:122-133, :136-140, :151-157`; tests :206-220 | exact |
| 14 | Display-hex → `from_str_radix` precedent | `apps/ping_server/src/main.rs:41-43` verbatim (`u32::from_str_radix(&format!("{}", handle), 16)`), production-proven | exact |
| 15 | usrlib conventions not reused: NE-token `[0..4]` pinning | `usrlib/src/ipc.rs:64-75` (`DATA_TOKEN_BYTES = 4`, `to_ne_bytes`/`from_ne_bytes`) ✓ for `send_data`/`receive_data`; **`ipc_send_value` (`:28-32`) pins a `usize` value over `[0..8]`, not a data token** | STALE (minor — AM-4f) |
| 16 | `usrlib/src/lib.rs:1` verbatim `#![cfg_attr(not(test), no_std)]` | `usrlib/src/lib.rs:1` verbatim | exact |
| 17 | `[[bin]] bench=false test=false` law; `cargo test --workspace` proves test=false bins never host-built/linked | `apps/bulk_echo/Cargo.toml:8-11`; fingerprint artifact Gate 2 host-binary roster (9 host binaries, no app bin) `.opencode/artifacts/ipc-ping-verify-final.md:59` | exact |
| 18 | bulk_echo skeleton: build.rs unconditional link-arg+`--pie`; `linker.ld`; `SyscallAllocator :12-25`; accept loop `:36-46`; `_start :58-63`; panic `:65-71` | `apps/bulk_echo/build.rs:1-5`; `apps/bulk_echo/linker.ld` exists; `main.rs:12-25, :36-46, :58-63, :65-71` | exact |
| 19 | `rosx-user.json` `"os": "none"` | `arch/x86_64/rosx-user.json:8` | exact (also validates the BR9 `CARGO_CFG_TARGET_OS == "none"` gate discriminator) |
| 20 | boot roster `arch/x86_64/src/main.rs:74-82`, Shell last | `main.rs:74-81` four `include_bytes!`+`schedule` pairs, `:82` `FunctionTask::new("Shell", shell::shell::main)` | exact |
| 21 | `arch/x86_64/build.rs:4-15` rerun triplets; `:17-29` existence panics | `build.rs:4-15` ✓ (12 triplet lines); existence panics span `:17-32` (4th check `ping_server` ends :32) | drift → `:17-32` (AM-4b) |
| 22 | shell `PING_ELF` `:31` verbatim; block-form `COMMANDS` "`:33-49`" with cfg'd ping insert; spawn form `:140-145` | `apps/shell/src/shell.rs:30-31` ✓ verbatim; `lazy_static!` block is `:35-54`, cfg'd insert `:50-51`; `ping()` `:140-145` ✓ exact | drift → `:35-54`, insert `:50-51` (AM-4a) |
| 23 | shell/build.rs ping triplet + existence check inside `if arch == "x86_64"` | `apps/shell/build.rs:9-11` (unconditional triplet), `:27-32` (x86_64 block) | exact |
| 24 | `xtask/src/main.rs:5` verbatim `USER_APPS [&str; 9]`; `run_tests()` calls `build_apps()` first; cmd shape `cargo test --workspace [--exclude tests-integration] -- --test-threads=1` cited "`:74-84`" | `:5` verbatim match (all 9 names, in order); `run_tests` body `:75-87`, `build_apps()` first `:76`, cmd `:78-85` | roster exact; citation drift → `:75-87` (AM-4c) |
| 25 | Root `Cargo.toml:2-22` members, `:23-31` default-members = system, kernel, usrlib, apps/shell, apps/dummy, apps/test_suite, xtask | `Cargo.toml:2-22`, `:23-31` exact incl. ranges | exact |
| 26 | ci.yml never runs tests-integration | `.github/workflows/ci.yml:31` `cargo xtask test --skip-integration`; `build-artifacts.yml:28` builds only | exact |
| 27 | qemu helpers: `spawn :29-31`, `expect_output :114`, `mark :118`, `wait_until_since :126-133`, `content_since :135-137` (ANSI-stripped), `send_line :175-178` | `tests-integration/src/qemu.rs:31, :114, :118, :126-133, :135-137` (`strip_ansi(&…)`), `:175-178` | exact |
| 28 | template `ipc_ping.rs`: mark-first, `[IPC Server] Ping` + `rose>` expects, `send_line`, `wait_until_since` + panic-with-tail, forbidden `["fail", "Panic"]` | `tests-integration/tests/ipc_ping.rs:70-82, :12` | exact |
| 29 | warning fingerprint: kernel lib 60; kernel lib-test 83 (49 dup); shell lib 3 at `shell.rs:148/168/182`; rosx bin 26 + manifest 1; rosx-x86 bin 1 (`arch/x86_32/src/main.rs:66:22`); tests-integration lib 1 | `.opencode/artifacts/ipc-ping-verify-final.md:145-152, :75, :111-112`; sites consistent with current tree (`shell.rs:148` fn-cast, `:168` unused `result`, `:182` `;;`) | exact |
| 30 | `grep -ri vfs` over apps/ kernel/ system/ usrlib/ xtask/ tests-integration/ arch/ = 0 | all seven greps returned zero matches | exact — `vfs`, `ipc_vfs`, `vfs_server`, `vfs_client`, `vfs-protocol` all free |
| 31 | clean task exit `kernel/src/task.rs:229-241` → `terminate_and_yield` | `elf_task_wrapper` `:229-241`, `terminate_and_yield` `:240` | exact |
| 32 | `MESSAGE_PAYLOAD_BYTES = 60` at `system/src/ipc.rs:2`; index-assert test pattern `:33-42` | `system/src/ipc.rs:2`; tests `:32-42` | exact |
| 33 | step commit subjects match repo style, no prefix | `.git/logs/HEAD:271-277`: "Add ping demo server and client applications", "Schedule ping server at x86_64 boot", "Add ping shell command spawning ping client", "Add QEMU integration test for ping data roundtrip", "List ping demo apps in AGENTS.md structure tree" — imperative, unprefixed | exact — the six plan subjects mirror this grammar verbatim |
| 34 | baseline identity "master @ 2603690" (plan header line 3) | repo branch is `main` (`pull origin main` `.git/logs/HEAD:279`); working branch `vfs-first-iteration`; HEAD `bb8c09f` | STALE (naming — AM-4e) |

No §0 claim turned out substantively load-bearing-wrong except items 15 and 33-of-step-4 (the `ls` sort claim — see AM-3); the rest are citation drift, corrected in AM-4.

---

## B. Internal consistency audit

### B.1 Wire math (BR3) — confirmed
- OPEN fills `[0..60]` exactly at `name_len = 57` (`3+57 = 60`) ✓; DELETE max end `2+57 = 59` ✓; READ/WRITE end at 17 ✓. No frame exceeds 60; no gap ambiguities.
- `valid_name` 1..=57 enforced by both edges (BR3) — consistent with `decode_request` strict rejects (unknown op, `name_len` 0 or ≥58) and with the §2.2 matrix (57 ok, 58 rejected).
- Chunking: 131072 = exactly 2 × 65536 ✓; server drain 65536/4096 = ≤ 16 chunks ✓; exactly-65536 accepted by kernel (strict `>`, item A2) ✓.

### B.2 EOF/zero edges — confirmed with one precision (AM-1)
- `read_data` on an empty object returns `Ok(0)` on the first call (empty payload → `copy_out` remaining 0), so the server's zero-length-WRITE drain terminates immediately ✓ (risk 6 consistent with `data_manager.rs:71-80`).
- READ `count = 0` → EOF **before** fs (BR8) and WRITE `bytes_written = 0` replies **OK-status** — so `decode_reply` must permit `Written(0)`; a blanket "demanded field zero → Err" reading would wrongly poison the zero-write path and (per B.3) live tokens.
- READ at/past EOF → `start=min(offset,len)`, `n=0` → empty → EOF ✓ (BR8 ↔ scenario 5).

### B.3 Data-token zero — the one real trap (AM-1)
`Handle::pack` is `generation << 16 | index` (`kernel/src/handle.rs:22-29`); the arena starts all slots at generation 0 (`kernel/src/collections/arena.rs:24-28`); `is_token(0)` is true (`usrlib/src/handle.rs:184` asserts `from_token(0).is_some()`). Therefore **token `0x00000000` is a legal live data token** for the first object in slot 0 of a fresh kernel. The `ipc_vfs` session can reach the first VFS READ/LIST reply before the boot-time bulk exchange (~t+40 s, per plan §5 step-5 note), so the first VFS reply object **can be** the kernel's first data object. BR3's zero-field rule must not be read as demanding a non-zero *token* (only Opened.handle / ReadData.count / Listed.total carry the ≥ 1 rule). AM-1 binds this.

### B.4 `decode_reply` Expect discrimination — under-specified, now bound (AM-1)
The `Reply` enum variants were never enumerated and the EOF/error status dispatch under each `Expect` was unstated. Worse, the OK-status shapes `Opened` and `Written` are byte-identical shapes (`[1..5]` u32, rest zero) and **cannot be distinguished by any decode rule** — §2.2's "wrong-shape → `Err`" and risk 2's "fail loudly" are over-promised for that pair and would make the coder write an unpassable test at the Step 1 gate. AM-1 defines the exact detectable Err-guarantee set.

### B.5 Steps — dependencies resolve at turn, ordering confirmed
1. Step 1 (`vfs-protocol`) depends on merged usrlib/system only ✓.
2. Step 2 (`vfs_server`) depends on Step 1 + merged surface ✓; the `CARGO_CFG_TARGET_OS == "none"` gate discriminates correctly (`rosx-user.json:8` "os": "none"; host = "linux"); `--lib` gate is the falsifiable probe for risk 1.
3. Step 3 depends on Step 2's ELF (via Step 2's xtask roster bump — verified `build_apps()` builds the whole roster and `xtask build` runs it first, `xtask/src/main.rs:32-41, :76`) ✓.
4. Step 4 depends on Steps 1-2 (client crate path) ✓; `VFS_ELF` include_bytes is cfg'd to x86_64 so the x86 build gains no precondition; host `cargo test --workspace` compiles shell with `target_arch = "x86_64"` true, so the `vfs_client` ELF must exist for host test builds — satisfied because `cargo xtask test` runs `build_apps()` first (`xtask/src/main.rs:76`) — the identical dependency ping proved (ping gate 2).
5. Step 5 touches only `tests-integration/tests/ipc_vfs.rs` ✓; roster 9 → 10 confirmed by structural count (`app_lifecycle` 2 `app_lifecycle.rs:16,57`; `boot_and_shell` 4 `boot_and_shell.rs:5,12,19,27`; `ipc_bulk` 1 `:81`; `ipc_random` 1 `:82`; `ipc_ping` 1 `:67`).
6. Step 6 anchor verified verbatim at `AGENTS.md:41` ✓.

### B.6 Blast radius — confirmed neutral
- `boot_and_shell.rs` needles: `"snake"`, `"tetris"`, `"rose>"`, `"Unknown command: foo"` (`:32-37`) — additive `vfs` key cannot collide (no needle is a substring of the new line or vice versa).
- `app_lifecycle.rs` needles: `"TETRIS"`, `"SNAKE"`, `"rose>"` — no collision with `[IPC Server] VFS`.
- `ipc_vfs` forbidden sweep `["fail", "Panic"]`: the ipc_ping baseline ran green with the same sweep over a window that already includes full boot output (`ipc_ping.rs:70-71,192-198`), so boot lines are sweep-safe; the new `[IPC Server] VFS` line is `fail`/`Panic`-free; `vfs error:` lines contain "error", not "fail" ✓.
- Warning Δ-new gate form (location-first, Δ against recorded fingerprint, both `warning:` and `warning[EXXXX]:` forms, totals corroborate only) is correct as written — except the `--> vfs` counting filter, which only catches `vfs-protocol/...` and silently misses `--> apps/vfs_server/...` and `--> apps/vfs_client/...` (AM-2).

### B.7 Scenario matrix (§2.4) ↔ client API (BR10) ↔ server dispatch (BR11) — confirmed
19 `vfs ok:` lines = 19 numbered mechanics = `EXPECTED_OK: [&str; 19]` ✓ (enumerated and counted). `seek` client-side only ✓ zero wire surface ✓. Scenario 3 hole (4096→8192 zero fill) ↔ BR8 resize-then-splice ✓. Scenario 4 single RPC (`min(9216, 65536)`) ✓; scenario 12 exactly two RPCs ✓; scenario 19 empty list = server EOF path ✓ matches BR8/BR11 and `list()` → empty Vec ✓. Server `None`-decode → silent ignore has no client-side hang exposure in a single synchronous client (BR11 ↔ risk 5) ✓. Drain-ack release ordering (release-held-for-conn *before* dispatch) is pinned by BR11's stated order ✓.

### B.8 Acceptance table (§5)
Commands 3,4,6 are ping-artifact-proven forms (`ipc-ping-verify-final.md:85-137, :52-73`); command 5 proven at gate 4 (`:98-116`). Commands 1,2 are new-package `-p` scoping honestly labeled `unverified` → routed to the orchestrator baseline pass; neither carries hidden ELF preconditions (`-p` scoping excludes shell, whose `include_bytes!` demands the rosx-user ELFs). Gate 6's host-total phrasing is Δ-form ("baseline + the new §2.1/§2.2 tests") — no count-equality trap ✓.

### B.9 Gate-coverage chain note
Step 1's gate (`cargo test -p vfs-protocol`, host) does **not** compile `client.rs` for the `rosx-user` bare-metal target. The first falsifier for client.rs on-target is Step 2's `cargo xtask apps` (vfs_server depends on `vfs-protocol`; a lib crate compiles all its modules for the target). AM-7 records the adjudication rule so a Step-2-gate compile error inside `vfs-protocol/src/client.rs` is fixed in Step 2's commit (no later commit may touch Step 1's files).

---

## C. Binding amendments (AM-1 … AM-8 — the coder follows these verbatim; they override the plan text where they differ)

**AM-1 (supersedes the BR3 codec paragraph and scopes §2.2 / risk 2).** `decode_reply` contract is bound as follows; `wire.rs` defines `pub enum Reply { Ack, Opened { handle: u32 }, ReadData { token: u32, count: u32 }, Written { bytes: u32 }, Listed { token: u32, total: u32 }, Eof, Err(VfsError) }` and `pub enum Expect { Ack, Opened, ReadData, Written, Listed }` with this dispatch: `[0] == 2` → `Ok(Reply::Eof)` under **any** `Expect` (the remaining 59 bytes are ignored; the EOF frame shape test pins them zero on encode). `[0] == 1` → `Ok(Reply::Err(VfsError::from_byte([1])))` under any `Expect`. `[0] == 0` → shape-checked against `Expect`: `Ack` requires bytes `[1..60]` all zero; `Opened` requires `handle ≥ 1`; `ReadData` requires `count ≥ 1`; `Listed` requires `total ≥ 1`; `Written` imposes **no** non-zero requirement (the zero-write path replies `Written(0)`, risk 6). Any demanded-field violation → `Err(())`. `[0] ∉ {0,1,2}` → `Err(())`. **Data tokens (`ReadData.token`, `Listed.token`) are any u32 including 0** — a live token can legitimately be `0x00000000` (kernel `Handle::pack` = `gen<<16|index`, `kernel/src/handle.rs:22-29`; arena starts gen 0 at slot 0, `kernel/src/collections/arena.rs:24-28`; `is_token(0)` true, `usrlib/src/handle.rs:184`), and the VFS service can be the first data-object writer in a fresh QEMU session. The non-zero rule **never** applies to data tokens. Because OK-status `Opened` and `Written` frames are byte-identical shapes, cross-decoding that pair is structurally undetectable: §2.2's wrong-shape battery tests only the detectable cases (status-byte mismatches, demanded-field-zero violations, non-zero payload under `Ack`), and risk 2's "fail loudly" guarantee is scoped to those.

**AM-2 (supersedes the `--> vfs` filter in §5's warning gate).** The new-artifact zero-warning grep is bound as `grep -E -- '--> (vfs-protocol/|apps/vfs_server/|apps/vfs_client/|tests-integration/tests/ipc_vfs)'` over the full build/test logs → **0 matches**. The plan's `--> vfs` literal matches only `vfs-protocol/...` paths and silently undercounts, because diagnostics for the two apps render as `--> apps/vfs_server/src/...` and `--> apps/vfs_client/src/...`. All other warning-gate wording stands (Δ-new = 0 vs recorded fingerprint, location-first arbitration, shell trio shift-attributed, both `warning:` and `warning[EXXXX]:` forms, totals corroborate never convict).

**AM-3 (corrects the Step-4 blast-radius sentence and risk 9's sibling claim).** "`vfs` sorts between `tests` and `tetris` in `ls`" is false: in `BTreeMap<String>` order the registry prints `clear, conway, ls, ping, pi, random, rose, sleep, snake, tetris, tests, vfs` (`'r' < 's'` puts `tetris` before `tests`; `vfs` sorts **after `tests`, at the tail**). The neutrality analysis is unchanged: `boot_and_shell.rs:32-37` asserts only `"snake"`, `"tetris"`, `"rose>"`, `"Unknown command: foo"`; none interacts with a trailing `vfs` entry.

**AM-4 (citation corrections — facts re-verified against the tree at HEAD; no behavioral consequence).**
a. shell COMMANDS block: "`:33-49`" → `apps/shell/src/shell.rs:35-54`; the cfg'd ping insert precedent is `:50-51`.
b. boot build.rs existence panics: "`:17-29`" → `arch/x86_64/build.rs:17-32` (fourth check, `ping_server`, ends `:32`); triplets `:4-15` stands.
c. xtask `run_tests`: "`:74-84`" → `xtask/src/main.rs:75-87` (`build_apps()` call at `:76`, command assembly `:78-85`).
d. BR10's Ok(0)-is-EOF convention cite "`syscall.rs:132-141`" → `usrlib/src/syscall.rs:184-188` (the `is_token` result convention inside `ipc_data_read`).
e. Plan header "`master` @ `2603690`" → branch **`main`** @ `2603690`; work happens on `vfs-first-iteration` (HEAD `bb8c09f` = step-0 baseline).
f. §0 usrlib-conventions line: the "`[0..4]` (NE)" token pinning holds for `send_data`/`receive_data` (`usrlib/src/ipc.rs:64-75`); `ipc_send_value` (`usrlib/src/ipc.rs:28-32`) pins a `usize` **value** over `[0..8]` and carries no data token. The operative conclusion (VFS assembles every frame itself; only `ipc_data_write`/`ipc_data_read`/`ipc_send`/`ipc_receive_message`/`wait_for_message` are used) stands.

**AM-5 (binds BR9's module law concretely).** `vfs-protocol`: `wire.rs` imports **neither** `system` nor `usrlib` — frame type is the literal `[u8; 60]` (BR3's cite of `system/src/ipc.rs:2` is provenance, not an import). `client.rs` may import `usrlib` + `alloc` + `wire`. The `system` entry in `vfs-protocol/Cargo.toml`'s `[dependencies]` remains exactly as BR9 specifies (declared, unused — cargo emits no warning for it). This keeps the §2.2/§2.1 host-test purity claim provable by grep: `grep -nE 'use (system|usrlib)' vfs-protocol/src/wire.rs` → no output.

**AM-6 (completes BR10 for the large-buffer scenario).** `vfs_client` scenario buffers ≥ 64 KiB (the 131072-byte write payload and read buffer, §2.4 steps 12-13) must be heap-allocated `Vec<u8>`/`vec![...]` through the package's `SyscallAllocator`; per-RPC object drain scratch uses a stack `[u8; 4096]` chunk buffer (usrlib precedent `usrlib/src/ipc.rs:65, :117-118`). `vfs_byte` is indexed by **absolute file offset**, so the position-based pattern continues contiguously across chunk boundaries in both directions.

**AM-7 (gate-coverage adjudication).** Step 1's host gate does not compile `client.rs` for `rosx-user`; Step 2's `cargo xtask apps` gate is the first falsifier of that code on-target (a lib crate compiles all modules for every target it is built for). If that gate fails inside `vfs-protocol/src/client.rs`, the fix rides **Step 2's commit** (Step 1's commit message is already minted); this is the only permitted cross-step repair and it touches no file outside Step 1's list.

**AM-8 (resolves §2.6 "closing docs").** The §2.6 limitations text is reproduced verbatim **only in the run's closing verification artifact** (an `.opencode/artifacts/` document produced by the verifier stage). It must not be pasted into any repo file: Step 6 locks AGENTS.md to tree lines only, and the comment law forbids it in Rust sources. "Documented, accepted, not mitigated" (decision 6) is satisfied by §2.6 + this plan + the closing artifact.

### Follow-up item (recorded, deliberately not fixed — Step 6's edit set is user-locked)
- `AGENTS.md:60` — "User-space ELF apps (hello_elf, random_gen_server, snake, tetris, conway) are workspace members" is a stale inventory: it already omits `bulk_echo`, `bulk_client`, `ping_server`, `ping_client` (pre-existing staleness) and will additionally omit `vfs_server`, `vfs_client`. Out of the locked Step-6 edit set (tree lines only). File a separate docs correction post-merge.

---

## D. Commit-message style confirmation

The six plan subjects — "Add VFS protocol crate with wire codec and client", "Add VFS server with in-memory file store", "Schedule VFS server at x86_64 boot", "Add VFS client app and shell vfs command", "Add QEMU integration test for VFS roundtrip", "List VFS protocol and apps in AGENTS.md structure tree" — carry no prefix and are grammatically the ping-run templates verbatim ("Add ping demo server and client applications", "Schedule ping server at x86_64 boot", "Add ping shell command spawning ping client", "Add QEMU integration test for ping data roundtrip", "List ping demo apps in AGENTS.md structure tree"; `.git/logs/HEAD:272-277`). **Confirmed.**

---

## E. Verdict

**VALIDATED-WITH-AMENDMENTS.**

The plan's architecture, locked decisions (all seven preserved untouched), step ordering, dependency direction, wire byte-offsets, EOF/zero semantics matrix, chunking math, blast-radius analysis, acceptance-command set, warning-gate form (modulo AM-2's filter fix), and commit style are sound. The amendments are contracts-completions and precision fixes a coder would otherwise have to guess (AM-1, AM-5, AM-6, AM-8), one gate-filter correctness fix (AM-2), one factual sort-order correction with no test impact (AM-3), a citation-drift bundle (AM-4), and a cross-step repair rule (AM-7). No step depends on later code; no gate contradicts an earlier step; the two `unverified` gates are correctly routed to the orchestrator baseline pass.

**Amendments:** AM-1 … AM-8 as above (§C), plus one recorded follow-up (AGENTS.md:60).

---

## F. Coder notes — three riskiest implementation points

1. **Zero-token legality (AM-1).** The instinct "handle 0 means none, reject it everywhere" is right **only for VFS file handles** (BR6, server-assigned from 1) and for the Opened-reply decode. For IPC **data tokens** on the wire, `0x00000000` is a live value that a fresh kernel can hand out first (`gen<<16|index` with slot 0/gen 0). Never filter or reject a data token by value; pass it opaquely to `Syscall::ipc_data_read(token as usize, …)`.
2. **`decode_reply` Expect battery (AM-1).** Write the §2.2 wrong-shape tests exactly against the detectable-Err set; do **not** write `decode_reply(Written-frame, Expect::Opened) → Err` — that pair is byte-identical by construction and the test cannot pass without a protocol change. If you feel the urge to add a discriminator byte to replies, stop: the protocol is one-request-in-flight and the Expect enum is the discriminator (risk 2, as scoped by AM-1).
3. **`build.rs` link-arg gate (Step 2, risk 1).** Emit `cargo::rustc-link-arg=-T…` + `--pie` iff `CARGO_CFG_TARGET_OS` == `"none"` **exactly** (`arch/x86_64/rosx-user.json:8`); compare the env var string, do not invert the test. A leaked bare-metal arg makes Step 2's `cargo test -p vfs_server --lib` fail at link; an over-tight gate makes `cargo xtask apps` fail to produce a bootable `vfs_server` ELF (caught by Step 3's `cargo xtask build`, but the fix belongs to Step 2's commit only if caught before Step 3 commits).
