# RxFS VFS — `Stat` Query on the Wire — Implementation Plan

> **Date:** 2026-11-02. **Baseline:** branch `rxfs-vfsd-memory-backend` @ `b57e4b0` (PR #105 — this plan stacks on the RxFS-backed vfsd; the flat `/{name}` bridge and hardened `valid_name` are preconditions).
> **Scope:** `protocol/vfs/src/wire.rs`, `protocol/vfs/src/client.rs`, `apps/vfsd/src/fs.rs`, `apps/vfsd/src/main.rs` (one dispatch arm), `apps/shell/src/launcher.rs`, `apps/vfs/src/main.rs`, `tests-integration/tests/ipc_vfs.rs` (EXPECTED manifest), `AGENTS.md`. **The `rxfs` crate is not touched** — `lookup` already answers exactly this query.
> **Nature:** treaty growth, minimal: one read-only query opcode, one reply, zero new error codes. Decided in discussion: **Stat query only** (no `Mkdir`/`Rmdir` — directories stay off the wire until their own theme); real consumers ship in the same theme (repo symmetry rule).

---

## 1. Locked decisions

| # | Decision |
|---|---|
| D1 | **`Reply::Stat { kind, size }` is the primitive**, not a boolean `Exists`. One call answers exists + what + how big; a later `Exists` would be a strict subset. `kind` is wire enum `EntityKind { File = 0, Directory = 1 }`; unknown kind bytes fail `Reply::decode` |
| D2 | **Missing name answers `Reply::Err(NotFound)`** — "no such thing" rides the existing error vocabulary, consistent with `Open` for the same name. No bespoke `NotExists` tag |
| D3 | **`size` only, no `blocks`, no `mtime`.** `mtime` is a txg stamp (RosX has no clock — exposing it would invite ABI regret); `blocks` is an engine-internal debug concern (user-visible APIs expose no kernel internals) |
| D4 | **Opcode/tag assignment:** `STAT = 7` (next free after `LIST = 6`); `REPLY_STAT = 0x17` (next free after `REPLY_LISTED = 0x16`). Request frame `[7, name_len u8, name…]` — identical name rule to `Open`/`Delete` (`valid_name` 1..=57, now also rejecting `/`, `.`, `..`), enforced by `decode_name` on the server. Reply frame `[0x17, kind u8, size u64 LE]` — 10 of 60 bytes |
| D5 | **Daemon-side `stat` takes no owner argument** — it holds no state (consistent with `Delete`), consumes **zero** rxfs open slots (it is `lookup`, not `open`), and never `sync()`s (read-only). Adapter returns wire `EntityKind` + `size`; the rxfs `Kind`→wire mapping lives in `fs.rs` |
| D6 | **Consumers ship now:** (a) `vfs` app scripted checks + `ipc_vfs` EXPECTED manifest growth; (b) shell launcher stat-first flow (§4). No consumer-less treaty surface |
| D7 | **Launcher read becomes size-bounded:** `stat` → `open` → read exactly `size` bytes. Short read ⇒ "truncated" error instead of silently launching a mutated image; the unbounded until-EOF loop disappears from the launch path. A zero-size file needs no special case — the existing ELF-magic check rejects the empty image exactly as today |
| D8 | No new `VfsError` codes. `lookup` errors route through the existing total `map_fs_error` |

Repo law applies: no comments, no `unsafe`, `#[cfg(test)]` modules, byte-pinned wire codec tests, no test-only production surface, no-prefix commit subjects, one commit per step (§6).

---

## 2. Wire (`protocol/vfs`)

```
pub const STAT: u8 = 7;
pub const REPLY_STAT: u8 = 0x17;

pub enum EntityKind { File, Directory }          // to_byte / from_byte, total: unknown -> Option::None
Request::Stat { name: &'a str }                  // encode reuses the Open/Delete name rule
Reply::Stat { kind: EntityKind, size: u64 }      // [0x17, kind, size LE]
```

`Reply::decode` maps a bad kind byte to `Err(())` (its existing error type — total, no new vocabulary). `client.rs`:

```
pub use crate::wire::EntityKind;
pub struct Entity { pub kind: EntityKind, pub size: u64 }
impl Vfs { pub fn stat(&self, name: &str) -> Result<Entity, ClientError> }
```

`Reply::Stat` → `Ok(Entity)`; `Reply::Err(e)` → `Err(ClientError::Vfs(e))` (so missing = `Err(Vfs(NotFound))`); any other tag → `Protocol`. Mirrors the existing method shapes exactly.

## 3. Daemon (`apps/vfsd`)

`VfsStore::stat(&mut self, name: &str) -> Result<(EntityKind, u64), VfsError>`: `valid_name` guard → `fs.lookup(&volume_path(name))` → `map_fs_error` on error; on `Ok(stat)` map `rxfs Kind::Regular → EntityKind::File`, `Kind::Directory → EntityKind::Directory`, pass `size` through. No sync, no table touched.

`main.rs` gains one `dispatch` arm: `Request::Stat { name } => match self.fs.stat(name) { Ok((kind, size)) => Reply::Stat { kind, size }, Err(error) => Reply::Err(error) }`.

## 4. Shell launcher (`apps/shell/src/launcher.rs`)

Current flow: `open(name, 0)` → `NotFound` ⇒ "Unknown command"; unbounded `read_whole_file` (4 KiB chunks until `Ok(0)`); ELF-magic check; launch.

New flow:
1. `vfs.stat(name)` — `Err(Vfs(NotFound))` ⇒ "Unknown command: {name}" (message unchanged); other errors ⇒ "Cannot stat {name}: {err}" (new failure surface, printed like the existing "Cannot open" line).
2. `entity.kind == Directory` ⇒ "Not an executable: {name}" — same message family as the non-ELF case (arm is unreachable until the directory theme lands; documented, accepted: one match arm on a protocol enum).
3. `open` as today; `read_whole_file(&mut file, name, entity.size)` loops until exactly `size` bytes collected or `Ok(0)`; **collected < size ⇒ "Cannot read {name}: truncated image"** — reachable today against daemon misbehavior, and strictly better than the old loop which happily launched a file that changed mid-read (the daemon is serial, but a *different client* can interleave writes between launcher requests at request granularity; the bounded read turns that corruption path into a hard error).
4. ELF-magic check and launch unchanged.

## 5. `vfs` app + integration manifest

New scripted checks in output order (static `vfs ok:` lines — the EXPECTED manifest matches lines verbatim, so dynamic values never appear in the printed line; the app asserts values internally):

| Script position | Printed line | Internal check |
|---|---|---|
| before creating `notes.txt` | `vfs ok: stat ghost denied` | `stat("notes.txt")` ⇒ `Err(Vfs(NotFound))` |
| after the 9216-byte hole-written state | `vfs ok: stat notes.txt 9216` | `Ok(File, 9216)` |
| after the 128 KiB chunked blob | `vfs ok: stat blob.bin 131072` | `Ok(File, 131072)` |
| any time after connect | `vfs ok: stat randomd rom intact` | ROM-materialized file: `Ok(File, size > 0)` — byte-exact engine proof |
| after `delete notes.txt` | `vfs ok: stat after delete denied` | name gone ⇒ `Err(Vfs(NotFound))` |

`tests-integration/tests/ipc_vfs.rs`: `EXPECTED_OK: [&str; 19]` → `[&str; 24]`, entries inserted at the matching output positions. No other change to the harness.

## 6. Steps (one commit each; every step green before the next)

| # | Deliverable | Gate |
|---|---|---|
| 1 | `protocol/vfs`: `STAT`/`REPLY_STAT`, `EntityKind`, `Request::Stat` + `Reply::Stat` codecs, `client::Vfs::stat` + `Entity` + byte-pinned tests | `cargo test -p vfs-protocol` |
| 2 | `apps/vfsd`: `VfsStore::stat`, dispatch arm, unit suite (§7.2) | `cargo test -p vfsd && cargo xtask apps` |
| 3 | Consumers: launcher stat-first + bounded `read_whole_file`; `vfs` app steps; `ipc_vfs` EXPECTED growth | `cargo test -p tests-integration --test ipc_vfs` locally green |
| 4 | `AGENTS.md`: VFS architecture bullet gains the stat sentence (§8) | grep reads true |

## 7. Test strategy

**7.1 Wire:** pinned `Request::Stat` encode/decode (incl. `a/b`/`.`/`..` rejection at encode and decode); pinned `Reply::Stat` little-endian size offsets + `EntityKind` byte pins; `Reply::decode` rejects unknown kind bytes; roundtrips for `File` and `Directory`.
**7.2 Adapter unit:** missing ⇒ `NotFound`; created-empty ⇒ `(File, 0)`; after hole write at 4096 ⇒ `(File, 4097)`; ROM file ⇒ exact ROM length; invalid name ⇒ `InvalidPath`; deleted name ⇒ `NotFound`; **64 open handles saturated ⇒ `stat` still succeeds** (pins zero slot consumption); `stat` performs no `sync` (observable: free-space counters unchanged after stat burst on a dirty volume).
**7.3 Integration:** `ipc_vfs` with the 24-line manifest on x86_64 QEMU. The launcher change is validated by *every other suite that launches by name* (`boot_and_shell`, `app_lifecycle`, `elf_arguments`, games) — no launcher-specific test added.

## 8. AGENTS.md sweep

VFS architecture bullet gains: a read-only `stat` query (name → kind + size, answered by rxfs `lookup`, zero open slots, `NotFound` for misses) and the note that `kind` is `Directory`-capable in the treaty while directory *creation* stays off-wire until the directory theme.

## 9. Acceptance (repo root, all green)

- **A1** `cargo test -p vfs-protocol && cargo test -p vfsd` — exit 0.
- **A2** `cargo xtask test --skip-integration` — exit 0.
- **A3** `cargo xtask build` and `cargo xtask build x86_32` — exit 0.
- **A4** `cargo test -p tests-integration -- --test-threads=1` on x86_64 — expanded `ipc_vfs` green; all other suites green unmodified.
- **A5** CI Integration (x86_32) green — the launcher's stat-first flow boots every app there.
- **A6** Δ-new warnings = 0.

## 10. Explicitly out of scope (later themes)

- `Mkdir`/`Rmdir`/`ListDir{path}` — directory creation on the wire; until then `EntityKind::Directory` is decode-legal but daemon-unproducible for user names.
- Path-carrying names, 255-byte name growth (needs chunked name transport past the 60-byte frame), `blocks`/`mtime` on the wire (blocked on a RosX clock).
- `rename()`/`truncate()` exposure; the ATA `BlockDevice` adapter; `xtask fsimg`.

## 11. Risks and accepted costs

1. **The `Directory` launcher arm is unreachable today** — shipped as the treaty's honest handler for the day directories exist; one arm, no engine path lies in the interim (user names are always `Regular`).
2. **Bounded launch read trusts `stat.size` across the stat→open→read window.** The daemon is serial per request, not across a client's request sequence — the plan makes the mutation-mid-launch case a hard "truncated" error rather than silent garbage. Accepted as strictly safer than the current unbounded loop.
3. **EXPECTED_OK order fragility** — pre-existing manifest pattern; new lines carry the same ordering contract.
4. **Reply-frame growth is zero** — `Reply` stays a fixed 60-byte encode; `Stat` uses 10 bytes; no client buffer sizes change.
