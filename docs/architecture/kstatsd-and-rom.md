# Kernel-hosted services and the ROM

## Invariants

- `kstatsd` runs as a kernel `FunctionTask` calling `IpcManager` directly; its `ProcessTable` reply rides the IPC data family and snapshots the task arena and per-group memory under one `IrqCell` section.
- The wire contract for kstats lives only in `system::kstats`; user-space clients own rendering.
- The ROM is an immutable image exposed through the opaque `RomRead` syscall; it is never a VFS volume and `init` is its only user-space reader.
- Both ports pack `BOOTSTRAP_APPS` into the ROM.

## Design

Kernel-hosted IPC services — `kstatsd` is the first: kernel code runs a service loop as a `FunctionTask` (launched by each arch at boot), calling `IpcManager` directly and blocking on futures via `Kernel::wait`. Two read-only operations share the `"KSTATSD"` binding: `MemorySnapshot` (op 0) answers in the 60-byte message; `ProcessTable` (op 1) rides the IPC data family — the reply message carries a data token (`system::ipc::encode_data_token`) and the table blob is a versioned fixed-stride buffer (`system::kstats::encode_process_table` / `decode_process_entry`), because a process list never fits 60 bytes. kstatsd snapshots the live task arena and per-group memory under one `IrqCell` section (the timer IRQ's resched path mutably borrows the task manager, so an unguarded walk would panic), then joins them: one row per loaded-image group, `pid` = the monotonic never-reused `GroupId`, one thread rolled up to the group's most-active state, memory = `MemoryManager::group_bytes` (that group's `Group` + `Stack` blocks). Metrics and identity stay domain-owned (`MemoryManager::stats`/`group_bytes`, `TaskManager::snapshot_processes`); the wire contract lives only in `system::kstats`; user-space clients (`mem`, `ps`) own rendering.

`rom/` — boot-registered immutable ROM image exposed via the opaque `RomRead` syscall; the kernel stays VFS-agnostic and the ROM is never a VFS volume — `init` is its only user-space reader, resolving every `rom:` plan token and the shell fallback from its own `read_image()`. The roster is per-arch (`ArchSpec.rom_apps`) and both ports pack `BOOTSTRAP_APPS` — the eight programs their plan names with `rom:` or needs before `/` exists, the shell included; the kernel's own copy of init is a separate `include_bytes!`. Its roster names stay underscore-free (open work: `docs/todo/rom-app-names-revert-underscore-free.md`).
