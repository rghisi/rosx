# vfsd Integration: FAT Volumes in the Mount Table — Plan Sketch

> **Status:** Planned, **not active**. Written 2026-11-05. Opens with questions that
> need owner answers before any code. Depends on `rxfat-read-engine.md` being done.

---

## Goal

Make a FAT volume a first-class citizen of the existing attach/mount flow so the
shell executes ELF files that live on a real FAT medium: `attach <dev-id> <service>
<device>` → `mount <dev-id> <dir>` → `rose> snake` runs the file off the disk, with
no rebuild in between. The shell launcher path needs **no change** — it already
streams ELFs by path through VFS (`LoadElf`); what changes is only what engine sits
behind the mount.

## Current shape the change lands on (verified 2026-11-05)

- `VfsStore<E: Engine, D: BlockDevice>` (`apps/vfsd`): root engine fixed to rxfs over
  `MemoryDevice` (`ActiveStore = VfsStore<ActiveVolume, RemoteBlock>`), mounted
  volumes are a `Vec<MountedVolume<D>>` table where **each volume is an rxfs
  `FileSystem<RemoteBlock>`** — today's table is single-engine.
- `mount` re-dials + probes the device, rejects geometries rxfs can't adapt
  (`VfsError::Unsupported` byte 13), validates the mount point through
  `Engine::lookup`, and drives the volume through `RemoteBlock`
  (`apps/vfsd/src/blk.rs`) which maps 4 KiB fs blocks onto device sectors.
- Error bytes: `NotFormatted` exists for an unformatted/foreign volume.
- rxfat's device trait is read-only-shaped; `RemoteBlock` can implement both that
  and rxfs's (plan D5 notes the eventual fs-core trait lift).

## Open questions (owner input required)

1. **How does `mount` choose the engine?** Auto-detect by boot sector (a volume is
   rxfs iff its superblock validates, FAT iff the FAT geometry validates — they are
   mutually exclusive signatures) vs a new wire flag (`mount --fat`). Auto-detect is
   the zero-wire-change option; a flag is explicit but grows the ABI.
2. **Heterogeneous mount table.** `MountedVolume` must stop assuming one engine type.
   Options: a `Box<dyn ...>` object-safe wrapper (needs `MetadataView`/handle
   unification — `Engine` is not object-safe today), or duplicating the per-op
   dispatch as an enum (`enum Volume { Rxfs(...), Fat(...) }`) over the existing
   table. The enum keeps zero-cost dispatch and no dyn; recommended.
3. **Format semantics.** `format` today means "lay down an rxfs superblock". A FAT
   volume arrives pre-formatted (from the host with mtools/`mformat`), so v1 is:
   `format` stays rxfs-only; FAT volumes just `attach` + `mount`. Is a RosX-side
   `mkfs` for FAT ever wanted (needs the FAT write path — separate arc)?
4. **ROM overlay vs FAT app directories.** With a FAT volume mounted at e.g. `/apps`,
   ROM names still resolve on the root volume; name collisions between the ROM
   name-set and the FAT files are decided by mount routing (mount prefix wins),
   which the existing routing already gives for free. Confirm that's the desired
   precedence.
5. **Exec policy.** The read-only `stat`/size-bounded-launch guards already exist;
   confirm no additional policy for FAT-sourced ELFs.

## Sketch of the work once answered

1. `apps/vfsd`: `enum Volume { Rxfs(FileSystem<RemoteBlock>), Fat(rxfat::FileSystem<RemoteBlock>) }`
   in the mount table; dispatch the twelve ops (mechanical); engine pick per Q1.
2. `RemoteBlock` gains the rxfat read-only trait impl.
3. `mount` probe order: rxfs superblock → FAT boot sector → `NotFormatted`.
4. Integration test (x86_64 QEMU harness): synthesize a FAT image host-side with
   mtools containing a small ELF (e.g. `echo`), boot, `attach`/`mount`, exec by
   name, assert output. mtools presence in CI needs checking before committing to
   this test (`unverified`).
5. Docs: AGENTS.md VFS architecture paragraph gains the second engine; `format`/
   `mount` behavior notes.

## Non-goals

- Writing FAT from RosX (needs the write arc; `Unsupported` is the honest answer).
- Partition carving (D4 in `README.md`): v1 volumes are superfloppy images presented
  whole to the block service, or the future carved-device work.
- GRUB booting from the same FAT volume — that is `boot-packaging.md`, parked.
