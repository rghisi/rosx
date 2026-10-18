# VFS Wire Typed Codec — Refactor Plan (`Request`/`Reply` inherent methods + retagged replies)

> **Date:** 2026-10-18. **Baseline:** branch `vfs-implementation-analysis-and-refactoring` @ `8e37508` ("Add VFS iteration 1"), tree clean.
> **Scope:** exactly three code files — `vfs-protocol/src/wire.rs`, `vfs-protocol/src/client.rs`, `apps/vfs_server/src/main.rs`. `kernel/`, `system/`, `usrlib/`, `arch/`, `xtask/`, `tests-integration/`, `apps/vfs_client/`, `apps/vfs_server/src/fs.rs`, `AGENTS.md` receive **ZERO changes**.
> **User-locked decisions (restate verbatim in every downstream delegation):**
> - **LD1 — methods on the existing enums, no trait.** No `VfsMessage` trait, no per-message structs. The sum types `Request<'a>` and `Reply` stay; serialization becomes inherent methods `Request::encode/decode` and `Reply::encode/decode`.
> - **LD2 — retagged replies; `Expect` deleted.** Each of the seven reply kinds gets a unique leading tag byte `0x10..=0x16`. `Expect` is removed from the crate. Request opcode bytes `1..=6` and every request frame layout are **unchanged**.
> - **LD3 — per-variant decode invariants kept:** `Opened` with `handle == 0` → `Err(())`; `ReadData` with `count == 0` → `Err(())`; `Listed` with `total == 0` → `Err(())`; `Ack`/`Eof` with any nonzero byte after the tag → `Err(())`; `Written` with `bytes == 0` is **accepted**; `ReadData` with `token == 0` is **accepted**.
> - **LD4 — `Reply::decode` error type stays `Result<Reply, ()>`** (user: "keep as it is for now"). No new error type.
> - **LD5 — breaking wire change accepted:** nothing persists frames (`RamFs` is RAM-only), the kernel only ferries opaque 60-byte payloads (`system/src/ipc.rs:2` `pub const MESSAGE_PAYLOAD_BYTES: usize = 60;`), both endpoints build from this tree.
> - **LD6 — repo law:** no comments in any Rust file (AGENTS.md); commit subjects carry **no prefix**; one commit per step.
> **The plan document itself** is committed separately from all code commits, subject `Add VFS wire typed codec refactor plan`.

---

## 0. Verified-baseline facts (re-verified this session against 8e37508)

- **Green baseline (this session):** `timeout 300s cargo test -p vfs-protocol` @repo root → exit 0, lib-test target **44 passed; 0 failed**, doc-test target 0. The 44 are the tests listed in §2.
- **Green baseline (prior run, `.opencode/artifacts/vfs-final-verify.md`):** gate 2 `cargo test -p vfs_server --lib` exit 0, **32 passed** (fs.rs only, wire-untouched); gate 3 `cargo xtask apps` (11 ELFs); gate 4 `cargo xtask build`; gate 6 `cargo xtask test` exit 0 (289 s wall) — host **369 passed / 0 failed**, QEMU roster **10** incl. `ipc_vfs::shell_vfs_filesystem_roundtrip ... ok`, grand total 379.
- **Warning fingerprint (`.opencode/artifacts/vfs-final-verify.md`):** kernel lib **60**; kernel lib-test **83 (49 dup)**; shell lib **3**; rosx bin **26** + rosx manifest **1**; rosx-x86 bin **1** (`arch/x86_32/src/main.rs:66:22`); tests-integration lib **1**. `vfs-protocol` host lib+lib-test compiles **zero-warning** (vfs-final-verify.md:24); `vfs_server` host lib-test **zero-warning** (`:39`); vfs app crates **zero-warning** (`:54`).
- **`cargo fmt --check` FAILS on the untouched baseline** for both touched packages (this session: `cargo fmt --check -p vfs-protocol` exit 1 — variant/line-format diffs in `wire.rs`, `client.rs`, `lib.rs`; `cargo fmt --check -p vfs_server` exit 1 — `fs.rs:53`). The crate is hand-formatted compact house style; see BR8.
- **Codec consumer inventory (complete, `git grep wire::`):** the encode/decode API is used **only** in `vfs-protocol/src/client.rs` (7 sites: `:52, :71, :86, :97, :129, :154, :183`) and `apps/vfs_server/src/main.rs` (19 sites: `:67, :69, :93, :94, :107, :108, :111, :112, :127, :131, :135, :137, :146, :150, :152, :163, :181, :185, :186`). `apps/vfs_client/src/main.rs:10` imports **only** `wire::VfsError`; `apps/vfs_server/src/fs.rs` uses **only** `valid_name`, `MODE_CREATE`, `MODE_WRITE`, `RPC_MAX_DATA_BYTES`, `VfsError` — neither file changes. No consumer outside `vfs-protocol/` and `apps/vfs_server/` exists (`git grep -nE 'Expect|STATUS_OK|STATUS_ERR|STATUS_EOF|encode_|decode_request|decode_reply'` outside these paths matches only unrelated text: `apps/test_suite` "Expected:" strings, `kernel/src/handle.rs` test names, `usrlib/src/ipc.rs` `encode_data_token`).
- **Current signatures the plan replaces (verbatim quotes):**
  - `vfs-protocol/src/wire.rs:243` — `pub fn decode_reply(frame: &[u8; FRAME_BYTES], expect: Expect) -> Result<Reply, ()> {`
  - `vfs-protocol/src/wire.rs:191` — `pub fn decode_request(frame: &[u8; FRAME_BYTES]) -> Result<Request<'_>, VfsError> {`
  - `vfs-protocol/src/wire.rs:16-18` — `pub const STATUS_OK: u8 = 0;` / `pub const STATUS_ERR: u8 = 1;` / `pub const STATUS_EOF: u8 = 2;` (retired in step 4)
  - `vfs-protocol/src/wire.rs:9-14` — `OPEN = 1, READ = 2, WRITE = 3, CLOSE = 4, DELETE = 5, LIST = 6` (unchanged)
  - `vfs-protocol/src/client.rs:41-45` — `fn rpc(connection: UserConnectionHandle, frame: &[u8; wire::FRAME_BYTES], expect: Expect,) -> Result<Reply, ClientError>`
  - `vfs-protocol/src/client.rs:1` — `use crate::wire::{self, Expect, Reply, VfsError};`
  - `apps/vfs_server/src/main.rs:160` — `        frame_with: fn(u32, u32) -> [u8; wire::FRAME_BYTES],` (the `wire::encode_read_data` / `wire::encode_listed` seam passed at `:137` / `:152`)
  - `apps/vfs_server/src/main.rs:11` — `use vfs_protocol::wire::{self, Request, VfsError};`
- **Client error path today (preserved exactly):** `client.rs:71-73` — `encode_open` `None` maps to `ClientError::Vfs(VfsError::InvalidPath)`; the new `Result`-typed `encode` carries the same error (`:92` same for delete).
- **Wrong-shape cross-check already lives in the caller matches:** `client.rs:81, :92, :109, :142, :166, :187` are all `_ => Err(ClientError::Protocol)` arms; deleting `Expect` relocates no lost safety (BR5).
- **Integration surface untouched:** `tests-integration/tests/ipc_vfs.rs` asserts only console-line grammar (`EXPECTED_OK` 19 strings, `:15`; test fn `:58`) — no byte-level expectations. Zero edits.
- **Stale doc line (explicit follow-up item, NOT edited):** `plans/branch-comparison-vfs-iteration-1.md:129` names `wire::encode_*` as the iteration-1 codec API; after this refactor the line describes a historical state. Historical comparison doc — record, never silently.
- **`AGENTS.md:36`** — `├── vfs-protocol/# VFS wire codec + typed client (user-space RPC treaty)` — stays accurate; no doc change.

---

## 1. Binding resolutions (the coder follows these verbatim)

**BR1 — file law.** The diff of this work contains exactly three pre-existing files: `vfs-protocol/src/wire.rs`, `vfs-protocol/src/client.rs`, `apps/vfs_server/src/main.rs` (plus this plan document in its own commit). `Cargo.lock` unchanged (no dependency edits). Any other path in the diff is a defect.

**BR2 — wire ABI, frozen tables.** Frames are `[u8; 60]`, all integers little-endian. Requests keep today's table verbatim (op byte 1..6; layouts per `wire.rs:90-142`, byte-for-byte). New reply constants, verbatim values and order, replacing `STATUS_OK/ERR/EOF`:

```rust
pub const REPLY_ACK: u8 = 0x10;
pub const REPLY_ERROR: u8 = 0x11;
pub const REPLY_EOF: u8 = 0x12;
pub const REPLY_OPENED: u8 = 0x13;
pub const REPLY_READ_DATA: u8 = 0x14;
pub const REPLY_WRITTEN: u8 = 0x15;
pub const REPLY_LISTED: u8 = 0x16;
```

Reply frames after the tag (tail beyond the named fields is zero on encode):

| reply | frame |
|---|---|
| Ack | `[0]=0x10`, rest zero |
| Err | `[0]=0x11, [1]=error code` |
| Eof | `[0]=0x12`, rest zero |
| Opened | `[0]=0x13, [1..5]=handle u32 (≥ 1)` |
| ReadData | `[0]=0x14, [1..5]=token u32, [5..9]=count u32 (≥ 1)` |
| Written | `[0]=0x15, [1..5]=bytes u32` (0 accepted) |
| Listed | `[0]=0x16, [1..5]=token u32, [5..9]=total u32 (≥ 1)` |

The reply tag range is disjoint from the request opcode range by construction (the decoder is per-direction; disjointness is still pinned by test, guarding future opcode growth and log readability).

**BR3 — `Request` codec contract (additive in step 1, free functions retired in step 4).**
- `pub fn encode(&self) -> Result<[u8; FRAME_BYTES], VfsError>` on `impl Request<'_>`: one `match self`, bodies moved verbatim from `encode_open/encode_read/encode_write/encode_close/encode_delete/encode_list`; the name-validity failure returns `Err(VfsError::InvalidPath)` (was `Option::None`); frame-zeroing and LE copies unchanged.
- `pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Request<'_>, VfsError>`: body moved verbatim from `decode_request` (`wire.rs:191-218`) — unknown op byte → `Err(VfsError::Protocol)` **takes precedence over** any field validation (pinned by existing test `decode_request_unknown_op_outranks_bad_name`).
- Private helpers `decode_name`, `read_u32`, `read_u64` stay as they are (fixed offsets inside a 60-byte array — the slicing cannot fail; the `unwrap()`s are infallible-by-invariant).

**BR4 — `Reply` codec contract.**
- `pub fn encode(&self) -> [u8; FRAME_BYTES]` on `impl Reply`: one `match self` writing tag + LE fields per the BR2 table; `Reply::Err` keeps `frame[1] = error.to_byte()`.
- `pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Reply, ()>`: `match frame[0]` over the seven tags implementing LD3 exactly:
  - `REPLY_ACK` / `REPLY_EOF` → all bytes `frame[1..]` must be zero, else `Err(())`;
  - `REPLY_ERROR` → `Ok(Reply::Err(VfsError::from_byte(frame[1])))` (total map, never fails);
  - `REPLY_OPENED` → `handle == 0` → `Err(())`;
  - `REPLY_READ_DATA` → `count == 0` → `Err(())` (token 0 fine);
  - `REPLY_WRITTEN` → always `Ok` (bytes 0 fine);
  - `REPLY_LISTED` → `total == 0` → `Err(())`;
  - any other byte → `Err(())`.
- One new private helper: `fn zero_tail(frame: &[u8; FRAME_BYTES], from: usize) -> Result<(), ()>` (the `(frame[from..]).iter().any(|b| *b != 0)` predicate today at `wire.rs:247,254`).
- `VfsError`, `to_byte`, `from_byte`, `valid_name`, `FRAME_BYTES`, `RPC_MAX_DATA_BYTES`, `MAX_NAME_BYTES`, `SERVICE_NAME`, `MODE_*`, opcode constants: untouched.

**BR5 — `Expect` death and the relocation of its cross-check.** The `Expect` enum (`wire.rs:62-69`) is deleted in step 4. Its discrimination job is redundant twice over: (a) unique tags make reply decoding context-free; (b) the wrong-shape cross-check already exists at every call site as the `_ => Err(ClientError::Protocol)` arm (`client.rs:81, :92, :109, :142, :166, :187`), which now catches a semantically-mismatched but well-formed reply exactly as `Err(())` did. The old test `decode_reply_eof_decodes_under_every_expect` / `decode_reply_err_carries_code_under_every_expect` universals are replaced by tag-based single-shape tests (Eof/Err decode identically everywhere — there is no context anymore).

**BR6 — client contract (step 2).**
- `use crate::wire::{self, Reply, VfsError};` (drop `Expect`; `wire::` stays for `SERVICE_NAME`, `FRAME_BYTES`, `RPC_MAX_DATA_BYTES`).
- `rpc` loses the `expect` parameter; last line becomes `Reply::decode(received.data()).map_err(|_| ClientError::Protocol)`.
- Call-site mapping (six sites; `encode` failure maps through `.map_err(ClientError::Vfs)?`, preserving today's `InvalidPath` behavior):

| today | after step 2 |
|---|---|
| `:71` `let Some(frame) = wire::encode_open(mode, name) else { return Err(ClientError::Vfs(VfsError::InvalidPath)) };` | `let frame = Request::Open { mode, name }.encode().map_err(ClientError::Vfs)?;` |
| `:86` delete form | `let frame = Request::Delete { name }.encode().map_err(ClientError::Vfs)?;` |
| `:97` `let frame = wire::encode_list();` | `let frame = Request::List.encode().map_err(ClientError::Vfs)?;` |
| `:129` `wire::encode_read(self.handle, self.offset, requested)` | `Request::Read { handle: self.handle, offset: self.offset, count: requested }.encode().map_err(ClientError::Vfs)?` |
| `:154` `wire::encode_write(self.handle, self.offset, data_token(&object))` | `Request::Write { handle: self.handle, offset: self.offset, token: data_token(&object) }.encode().map_err(ClientError::Vfs)?` |
| `:183` `wire::encode_close(self.handle)` | `Request::Close { handle: self.handle }.encode().map_err(ClientError::Vfs)?;` |

- All `Expect::*` arguments at the six `rpc(...)` calls vanish. The `Reply::*` match arms at each site stay byte-for-byte unchanged. Behavior is identical, including the reply-kind match per operation.

**BR7 — server contract (step 3): typed replies, encoding once at the boundary.**
- `use vfs_protocol::wire::{self, Reply, Request, VfsError};`
- `serve` decodes with `Request::decode` and encodes once at the syscall edge: `let _ = Syscall::ipc_send(connection, &reply.encode());` — the local `reply` is a `Reply`, and the decode-failure arm builds `Reply::Err(error)`.
- `dispatch`, `read_reply`, `list_reply`, `write_reply` return `Reply`, never frames. Per-arm mapping (semantics byte-identical): `wire::encode_opened(handle)` → `Reply::Opened { handle }`; `wire::encode_ack()` → `Reply::Ack`; `wire::encode_err(e)` → `Reply::Err(e)`; `wire::encode_eof()` → `Reply::Eof`; `wire::encode_written(w as u32)` → `Reply::Written { bytes: w as u32 }`.
- `attach_payload` keeps the injection seam, retyped: `reply_with: fn(u32, u32) -> Reply` (args: data token, payload length as u32). The two callers at `:137` / `:152` pass non-capturing closures coercing to that fn pointer: `|token, total| Reply::ReadData { token, count: total }` and `|token, total| Reply::Listed { token, total }` (the `Reply` variants are braced, not tuple — constructor coercion does not apply; closures do).
- The early-return guards keep their shape: `count == 0 → Reply::Eof` (`:126-128`), empty data → `Reply::Eof`, `drain_incoming` `None → Reply::Err(VfsError::BadHandle)`, `ipc_data_write` `Err → Reply::Err(VfsError::ServerResource)`; the held-object push (`self.held.push((owner, object))`) stays before returning the frame-carrying reply.
- `wire::` self-import stays (still used: `wire::SERVICE_NAME` `:50`, `wire::RPC_MAX_DATA_BYTES` `:129`). `wire::FRAME_BYTES` is no longer named in `main.rs` after step 3 — allowed (no warning: it is a crate item, not an import). **Unused-import rule:** zero-warning targets mean step 3 must leave `VfsError` used (it is: `ServerResource`/`BadHandle` arms) and no `Expect` residue anywhere — an unused import violates the warning gate.

**BR8 — formatting law.** `cargo fmt --check` fails on the untouched baseline for `vfs-protocol` and `vfs_server` (§0). Touched files keep the surrounding hand-compact house style (single-line short struct variants, compact matches); do **not** run `cargo fmt` over these files and do **not** inject reformat churn into a behavior-preserving diff. Report the baseline fmt failure per Coder rules; the fmt gate is **waived for the three touched files** and enforced as "no new formatting deviation beyond baseline style".

**BR9 — comment law.** No comments in any of the three files, before or after (`grep -nE '(^|[[:space:]])//[^!]'` clean on the final tree).

---

## 2. Wire unit-test migration matrix (`vfs-protocol/src/wire.rs`, host tests)

Baseline inventory: **44 tests** (§0 gate). Dispositions:

**KEEP-VERBATIM (4)** — no encode/decode reference: `valid_name_accepts_one_to_fifty_seven_bytes`, `valid_name_rejects_empty_and_oversize_and_forbidden_bytes`, `wire_error_codes_roundtrip`, `unknown_wire_error_bytes_map_to_protocol`.

**RETARGET (20)** — step 4 renames the call sites from free functions to methods (`encode_open(m, n).unwrap()` → `Request::Open { mode: m, name: n }.encode().unwrap()`; `decode_request(&f)` → `Request::decode(&f)`); every byte assertion stays identical (request format unchanged): `open_frame_pins_offsets_for_short_name`, `delete_frame_pins_offsets_for_short_name`, `read_frame_pins_little_endian_offsets`, `write_frame_pins_little_endian_offsets`, `close_frame_pins_offsets`, `list_frame_uses_op_byte_only`, `open_request_roundtrips`, `open_request_roundtrips_maximum_name`, `read_request_roundtrips`, `write_request_roundtrips`, `close_request_roundtrips`, `delete_request_roundtrips`, `list_request_roundtrips`, `decode_request_rejects_unknown_op_bytes`, `decode_request_rejects_zero_name_len`, `decode_request_rejects_name_len_beyond_limit`, `decode_request_rejects_declared_name_past_frame_end`, `decode_request_rejects_invalid_utf8_name`, `decode_request_unknown_op_outranks_bad_name`, `encode_open_rejects_invalid_names` (renamed **`request_encode_rejects_invalid_names`**, asserts `Err(VfsError::InvalidPath)`).

**DELETE in step 4 (20)** — old reply framing + `Expect` semantics, fully superseded by the ADD set: `listed_reply_pins_little_endian_offsets`, `read_data_reply_pins_little_endian_offsets`, `eof_frame_has_status_byte_and_zero_tail`, `ack_frame_is_all_zero`, `decode_reply_accepts_ack_shape_under_ack_expect`, `decode_reply_accepts_each_success_shape_under_its_expect`, `decode_reply_written_accepts_zero_bytes_for_empty_write`, `decode_reply_read_data_accepts_zero_token`, `decode_reply_eof_decodes_under_every_expect`, `decode_reply_eof_rejects_nonzero_tail`, `decode_reply_err_carries_code_under_every_expect`, `decode_reply_rejects_unknown_status_byte`, `decode_reply_rejects_ack_shape_under_data_expectations`, `decode_reply_rejects_zero_field_replies_under_data_expectations`, `decode_reply_opened_with_zero_handle_is_err`, `decode_reply_rejects_zero_count_read_data_shape`, `decode_reply_rejects_zero_total_listed_shape`, `decode_reply_listed_with_zero_total_is_err`, `decode_reply_rejects_nonzero_tail_for_ack`, `protocol_error_frame_carries_byte_255_and_decodes_back`.

**ADD in step 1 (18)** — new names (no collision with the live old names until step 4):

| # | test | pins |
|---|---|---|
| A1 | `reply_tag_constants_are_pinned` | the seven tag constants equal `0x10..=0x16` verbatim; the six opcodes equal `1..=6`; the two sets are disjoint |
| A2 | `ack_reply_pins_tag_and_zero_tail` | `Reply::Ack.encode()` → `[0]=0x10`, rest zero |
| A3 | `err_reply_pins_tag_and_error_byte` | `Reply::Err(BadHandle)` → `[0]=0x11, [1]=4`; `Protocol` → `[1]=255` (absorbs deleted #43) |
| A4 | `eof_reply_pins_tag_and_zero_tail` | `[0]=0x12`, rest zero |
| A5 | `opened_reply_pins_tag_and_handle_offset` | `[0]=0x13`, `[1..5]` LE golden `0x0102_0304` → `[0x04,0x03,0x02,0x01]`, tail zero |
| A6 | `read_data_reply_pins_tag_and_field_offsets` | `[0]=0x14`, token/count LE goldens at `[1..5]`/`[5..9]`, tail zero |
| A7 | `written_reply_pins_tag_and_bytes_offset` | `[0]=0x15`, LE bytes at `[1..5]`, tail zero |
| A8 | `listed_reply_pins_tag_and_total_offset` | `[0]=0x16`, token/total LE goldens, tail zero |
| A9 | `reply_all_variants_roundtrip` | all seven variants: `Reply::decode(&r.encode()).unwrap() == r` (valid values: handle≥1, count≥1, total≥1) |
| A10 | `reply_decode_rejects_unknown_tag` | byte `0x00`, `0x0F`, `0x17`, `0xFF` in `[0]` → `Err(())` (absorbs deleted #34) |
| A11 | `reply_decode_rejects_nonzero_ack_tail` | ack frame with `[7]=0x40` → `Err(())` (from deleted #41) |
| A12 | `reply_decode_rejects_nonzero_eof_tail` | eof frame with `[9]=1` → `Err(())` (from deleted #32) |
| A13 | `reply_decode_rejects_zero_handle` | tag `0x13`, `[1..5]=0` → `Err(())` (from deleted #37) |
| A14 | `reply_decode_rejects_zero_count` | tag `0x14`, token 7, count 0 → `Err(())` (from deleted #38) |
| A15 | `reply_decode_rejects_zero_total` | tag `0x16`, token 123, total 0 → `Err(())` (from deleted #39/#40) |
| A16 | `reply_decode_accepts_zero_bytes_written` | `[0x15,0,0,0,0,...]` → `Ok(Written { bytes: 0 })` (from deleted #29) |
| A17 | `reply_decode_accepts_zero_read_token` | tag `0x14`, token 0, count 1 → `Ok` (from deleted #30) |
| A18 | `reply_decode_accepts_err_under_protocol_byte` | `[0x11, 255, 0…]` → `Ok(Err(VfsError::Protocol))`; unknown code bytes → `Protocol` already covered by #44 |

**Count derivation (binding):** baseline 44 = KEEP 4 + RETARGET 20 + DELETE 20. After step 1: 44 + 18 = **62**. Steps 2–3 add no wire tests. After step 4: 62 − 20 = **42** = KEEP 4 + RETARGET 20 + ADD 18. Workspace host total moves 369 → 387 (step 1) → 367 (step 4); `vfs_server --lib` stays 32.

---

## 3. Implementation steps

### Step 0 — Commit the plan document
**Files:** `plans/vfs-wire-typed-codec.md` only.
**Commit:** `Add VFS wire typed codec refactor plan`

### Step 1 — Add typed encode/decode methods to `wire.rs` (purely additive)
**Files:** `vfs-protocol/src/wire.rs`.
**Contract:** add the seven `REPLY_*` constants (BR2); add `impl Request<'_> { pub fn encode... pub fn decode... }` (BR3) and `impl Reply { pub fn encode... pub fn decode... }` (BR4); add helper `zero_tail`; add the 18 tests (§2 ADD). **Delete nothing** — free functions, `Expect`, `STATUS_*`, and all 44 baseline tests stay (nothing calls the new methods yet; a library's pub items trip no dead-code warnings). Request byte output of `Request::encode` must equal the free functions' output byte-for-byte (both alive; A5–A8 vs old pins prove it).
**Why first:** everything downstream compiles against the method names; the commit is behavior-neutral at runtime.
**Gate:** `timeout 300s cargo test -p vfs-protocol` @repo root — expected **62 passed; 0 failed** (§2 count derivation). Green form: same invocation proven at this session's baseline (44 passed).
**Commit:** `Add typed encode and decode methods to VFS wire enums`

### Step 2 — Move `client.rs` onto the typed methods
**Files:** `vfs-protocol/src/client.rs`.
**Contract:** BR6 in full — import line, `rpc` signature (`frame: &[u8; wire::FRAME_BYTES]`, no `expect`), six call sites per the mapping table, all `Expect::*` arguments removed, all `Reply::*` match arms untouched. No wire.rs changes in this commit.
**Compilability:** depends only on step 1. Host-compiles as part of `vfs-protocol` (usrlib host-shim pattern, proven at baseline).
**Runtime note (risk 1):** between this commit and step 3 the on-the-wire protocol mismatches (new client vs old server); only host unit tests gate this window, which is why the QEMU evidence gate lands at step 4, not here.
**Gate:** `timeout 300s cargo test -p vfs-protocol` @repo root — expected **62 passed; 0 failed** (no test-count change; proves client still compiles and wire tests are untouched).
**Commit:** `Move VFS client onto typed wire methods`

### Step 3 — Move `vfs_server` onto typed replies
**Files:** `apps/vfs_server/src/main.rs`.
**Contract:** BR7 in full — `dispatch`/`read_reply`/`list_reply`/`write_reply`/`attach_payload` return `Reply`; `serve` decodes via `Request::decode`, builds `Reply::Err(error)` on decode failure, and performs the single `reply.encode()` inside `Syscall::ipc_send(...)`; `attach_payload` seam becomes `fn(u32, u32) -> Reply` with the two non-capturing closures; import line gains `Reply`. No wire.rs / client.rs changes. After this commit both endpoints speak the new wire — the first point where a QEMU run is meaningful.
**Compilability:** depends on step 1 only (methods already merged); bare-metal build proves it via the gate.
**Gate:** `timeout 900s cargo xtask apps` @repo root — expected: exit 0, 11 ELFs incl. a freshly rebuilt `target/rosx-user/release/vfs_server`. Green form: proven by `vfs-final-verify.md` gate 3. Optional mid-flow witness (orchestrator's discretion, not a binding gate): `timeout 1200s cargo xtask test` here would already exercise `ipc_vfs`.
**Commit:** `Move VFS server onto typed replies`

### Step 4 — Retire free encoders, `STATUS_*`, and `Expect`
**Files:** `vfs-protocol/src/wire.rs`.
**Contract:** delete the 12 `encode_*` free functions, `decode_request`, `decode_reply`, the `Expect` enum, `STATUS_OK/STATUS_ERR/STATUS_EOF`; retarget the 20 §2-RETARGET tests to the method API; delete the 20 §2-DELETE tests; `decode_name`, `read_u32`, `read_u64` stay (methods use them). Nothing outside wire.rs changes in this commit (consumers migrated in steps 2–3; §0 consumer inventory is exhaustive, so no other file can reference a deleted item — a compile error here would itself falsify the inventory).
**Gate:** `timeout 300s cargo test -p vfs-protocol` @repo root — expected **42 passed; 0 failed**; plus the absence gate (§5, expected empty post-change; pre-change inventory non-empty by construction — definitions + 26 external call sites, verifier dry-runs at baseline).
**Commit:** `Retire free encoders and Expect from VFS wire`

### Step 5 — Full verification (no new files)
**Gates:** §5 acceptance rows 2–5 in order. QEMU roster stays **10** incl. `ipc_vfs::shell_vfs_filesystem_roundtrip ... ok` (file untouched — its console grammar must survive a changed wire, which is exactly what this run proves).

---

## 4. Risks and edge cases (coder must handle / verifier must know)

1. **Transient protocol mismatch (commits 2→3).** After step 2 the client speaks new reply tags while the server still answers `STATUS_*` shapes. Host unit tests stay green (they never mix endpoints); no QEMU or integration evidence is claimable for that window. Binding runtime proof is the step-5 `cargo xtask test` against the fully-migrated tree. Do not "fix" the window by touching step order.
2. **Deliberate wire break.** No persistence (`RamFs`), kernel treats message payloads as opaque 60-byte transfers (`system/src/ipc.rs:2`), no third-party consumer exists (§0 inventory). Frames produced by older binaries are not read by newer ones anywhere in the system.
3. **Zero-frame divergence.** Today `decode_reply(&[0u8; 60], Expect::Opened)` rejects via the zero-handle rule; after step 4 an all-zero frame rejects earlier (tag `0x00` is not a reply tag) — same `Err(())`, different internal path. A13 covers the zero-handle rule on a valid tag, so the LD3 invariant stays pinned independently of the tag lookup.
4. **`Ack` is no longer all-zero** (`0x10` at `[0]`). No code path assumes an all-zero Ack (the only such assertion, old test #10, is in the DELETE set); `serve`/`rpc` treat frames opaquely.
5. **Unused-import discipline (warning gate).** Step 2 must drop `Expect` from `client.rs:1`; step 3 must not strand imports. Both targets are zero-warning today (§0) — one unused import breaks the Δ-new gate.
6. **`Result` vs `Option` encode failure.** `Request::encode` returns `Err(VfsError::InvalidPath)` where free functions returned `None`; the client maps via `.map_err(ClientError::Vfs)?` — `open`/`delete` error behavior byte-identical to `client.rs:71-73/:86-88` today. `Request::List.encode()`/`Read`/`Write`/`Close` never fail; the `?` remains uniform (no special-casing).
7. **Closure-coercion seam.** `Reply::ReadData`/`Listed` are braced variants — they are **not** fn pointers; the `attach_payload` seam needs the two non-capturing closures of BR7 (coercion to `fn(u32, u32) -> Reply` is inference-driven at the parameter position — if inference complains, annotate the closure binding; do not "simplify" the seam away).
8. **Semantics preserved at server guards.** `count == 0 → Eof` before fs (`main.rs:126-128`), empty read result → `Eof`, empty list blob → `Eof`, bad drain token → `Err(BadHandle)`, data-object failure → `Err(ServerResource)`, held-object bookkeeping unchanged. The refactor moves encoding to the boundary; it must not move or drop a single guard.
9. **Console grammar frozen.** `[IPC Server] VFS` boot line, the 19 `vfs ok:` lines, `vfs done` — unchanged; `ipc_vfs.rs` needs zero edits. No new console output anywhere.
10. **fmt baseline failure is pre-existing** (§0, BR8): never "fix" it with a reformat commit or in-line churn; report it.
11. **Stale doc line** `plans/branch-comparison-vfs-iteration-1.md:129` (`wire::encode_*` mention): explicit follow-up item, not edited by this plan (historical record).

---

## 5. Acceptance criteria (exact commands)

| # | Command | CWD | Expected |
|---|---|---|---|
| 1 | `timeout 300s cargo test -p vfs-protocol` | repo root | exit 0; **42 passed; 0 failed** (+0 doc) on the final tree; 62 at steps 1–3 — `green form` (this session: 44 at baseline) |
| 2 | `timeout 300s cargo test -p vfs_server --lib -- --test-threads=1` | repo root | exit 0; **32 passed** (unchanged) — `green form` |
| 3 | `timeout 900s cargo xtask apps` | repo root | exit 0; 11 ELFs incl. rebuilt `vfs_server` — `green form` (vfs-final-verify gate 3) |
| 4 | `timeout 900s cargo xtask build` | repo root | exit 0; reaches the disk-image step; `^error\|error\[` grep = 0 — `green form` |
| 5 | `timeout 1200s cargo xtask test` | repo root | exit 0; host **367 passed / 0 failed** (= 369 − 20 + 18, §2 derivation); QEMU roster **exactly 10** incl. `ipc_vfs::shell_vfs_filesystem_roundtrip ... ok` — `green form` (vfs-final-verify gate 6, 289 s wall) |

**Warning gate — Δ-new = 0** against the recorded fingerprint (`.opencode/artifacts/vfs-final-verify.md`: kernel lib 60; kernel lib-test 83 (49 dup); shell lib 3; rosx bin 26; rosx manifest 1; rosx-x86 bin 1; tests-integration lib 1), arbitrated location-first; the three touched files contribute **zero** unique warnings (`--> vfs-protocol` / `--> vfs_server` new-site grep = 0 against the §0 zero-warning fact); counting filter matches `warning:` and `warning[EXXXX]:`; totals corroborate, never convict.

**Absence gate (portable POSIX grep, binding after step 4):**
`git grep -nE 'Expect|STATUS_OK|STATUS_ERR|STATUS_EOF|encode_(open|read|write|close|delete|list|ack|eof|err|opened|read_data|written|listed)\(|decode_(request|reply)\(' -- vfs-protocol/ apps/vfs_server/ apps/vfs_client/` → **no output**. Pre-change inventory (verifier dry-run, expected non-empty): `client.rs` 7 external sites + `main.rs` 19 external sites + the wire.rs definitions and internal/test call sites; `--text` note: `Request::decode`/`Reply::decode` method spellings do not match the patterns.

**Comment law gate:** `grep -nE '(^|[[:space:]])//[^!]' vfs-protocol/src/wire.rs vfs-protocol/src/client.rs apps/vfs_server/src/main.rs` → no output (also true at baseline).

**Diff-boundary gate:** `git diff --name-only 8e37508..HEAD` contains only `plans/vfs-wire-typed-codec.md` (step 0 commit), `vfs-protocol/src/wire.rs`, `vfs-protocol/src/client.rs`, `apps/vfs_server/src/main.rs`. No run-artifact or spec file inside the code commits.

**Commit-style gate:** four code-commit subjects, exactly: `Add typed encode and decode methods to VFS wire enums` · `Move VFS client onto typed wire methods` · `Move VFS server onto typed replies` · `Retire free encoders and Expect from VFS wire`; no prefix on any subject (repo law; matches `git log` style — "Add VFS iteration 1", "Migrate vfs apps to the shared usrlib runtime").
