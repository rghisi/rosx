# Phase 0 — The Boot ABI (`system::boot_info`)

The boot handoff treaty lives in the treaty crate: a new module `system/src/boot_info.rs`,
exposed as `system::boot_info`. Per repo law, `system` holds **numbers and layouts only** —
this is the boundary contract between loader and kernel, exactly like `system::rom` and
`system::boot_plan`. The kernel edge encodes/writes; the loader's client kernels decode.
User-space apps never see it. Unit tests (size, alignment, enum values) live in the module's
`#[cfg(test)]` block like every other `system` treaty.

## 1. Types

```rust
pub const BOOT_INFO_MAGIC: u32 = 0x524F_5358;
pub const BOOT_INFO_VERSION: u16 = 1;
pub const HIGHER_HALF_OFFSET: u64 = 0xFFFF_8000_0000_0000;
pub const MEMORY_MAP_MAX_ENTRIES: usize = 64;

#[repr(u16)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BootArch {
    X86_32 = 1,
    X86_64 = 2,
}

#[repr(u16)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BootMethod {
    Bios = 1,
    Uefi = 2,
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    Usable = 1,
    Reserved = 2,
    BootloaderReclaimable = 3,
    KernelAndModules = 5,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryKind,
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    None = 0,
    VgaText = 1,
    LinearFramebuffer = 2,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayInfo {
    pub kind: DisplayKind,
    pub addr: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u8,
    pub red_mask_shift: u8,
    pub green_mask_shift: u8,
    pub blue_mask_shift: u8,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct BootInfo {
    pub magic: u32,
    pub version: u16,
    pub arch: BootArch,
    pub boot_method: BootMethod,
    pub memory_map_ptr: u64,
    pub memory_map_len: u32,
    pub memory_map_entry_size: u32,
    pub initrd_addr: u64,
    pub initrd_size: u64,
    pub display: DisplayInfo,
}
```

Notes on the design versus the original sketch:

- **`rsdp_addr` / `smbios_addr` are dropped.** The tree has zero ACPI/SMBIOS consumers
  (grep-verified), and repo law forbids production surface without a real user. Future fields
  append at the tail with a `BOOT_INFO_VERSION` bump; readers gate on `version` +
  `memory_map_entry_size`-style stride discipline.
- **`AcpiReclaimable = 4` is dropped** (no consumer); the normalizer folds E820 ACPI entries
  into `Reserved`.
- `MemoryKind` values keep their original numbering so `4` stays visibly retired.
- `DisplayKind::LinearFramebuffer` is declared but **never produced** by this plan's stage2
  (VgaText only); it exists so the integration plan's framebuffer decision (§9 of the overview)
  cannot require a re-layout.
- Unknown enum values decode honestly at the kernel edge (a variant outside the set is a
  contract violation the kernel refuses, mirroring `system::exit` reserved-reason rules).

## 2. Field semantics

| Field | Contract |
|-------|----------|
| `magic` | Always `BOOT_INFO_MAGIC`. Asserted by the kernel **before** any field read. |
| `version` | `BOOT_INFO_VERSION` at write time; kernel accepts `== BOOT_INFO_VERSION`. |
| `arch` | Matches the loaded ELF class; the kernel asserts equality with its own build. |
| `boot_method` | `Bios` for this plan. |
| `memory_map_ptr` / `memory_map_len` / `memory_map_entry_size` | A sorted, non-overlapping array of `memory_map_len` records of `memory_map_entry_size` bytes each. Address is **physical** for `X86_32`, **higher-half virtual** for `X86_64`. |
| `initrd_addr` / `initrd_size` | The loaded initrd bytes; same address-space rule as `memory_map_ptr`. `size == 0` ⇒ `addr == 0`. |
| `display` | `VgaText`: `addr = 0xB8000`, `width = 80`, `height = 25`, `pitch = 160`, `bpp = 0`, shifts `0` — text mode, the remaining fields carry no meaning under this contract. BIOS never mode-sets in this plan. |

`BootInfo` is written 16-byte aligned. The `u16` trio (`version`, `arch`, `boot_method`) plus
implicit trailing padding is the only padding in the struct; alignment/size unit tests pin it.

## 3. Handoff calling convention

| | 32-bit (`ELF32`, `EM_386`) | 64-bit (`ELF64`, `EM_X86_64`) |
|---|---|---|
| CPU state | Protected mode, paging **off** (`CR0.PG = 0`), `CLI`, A20 on, flat GDT per §4 | Long mode: `CR0.PG=1`, `CR4.PAE=1`, `EFER.LME=1`, `CLI`, A20 on, GDT per §4 |
| Register magic | `EAX = BOOT_INFO_MAGIC` | `RAX = BOOT_INFO_MAGIC` |
| BootInfo pointer | `EBX = &BootInfo` (physical) | `RDI = &BootInfo` (higher-half virtual) |
| Stack | `ESP` 16-byte aligned, physical | `RSP` 16-byte aligned, higher-half virtual |
| Transfer | `jmp` entry point (never `call`) | `jmp` entry point |
| Paging maps | none — identity by construction | identity `0..4 GiB` **and** `0xFFFF_8000_0000_0000..0xFFFF_8001_0000_0000` both live at entry and stay live |

The register-level magic (the Multiboot-proven pattern, deliberately different in value from
Multiboot's `0x2BADB002`) is what lets the real kernels discriminate boot paths during the
future coexistence migration — a struct-only magic forecloses that. This table is **frozen** at
plan completion (overview §8).

## 4. GDT contract

Stage2 installs the GDT; the kernel may install its own immediately, but until it does, the
selector layout below is the contract. The x86_64 layout is not free-form: the current kernel's
`STAR` MSR (`arch/x86_64/src/cpu.rs`) hardcodes `0x08` kcode / `0x10` kdata / `0x18` udata /
`0x20` ucode, and the kernel never installs its own GDT today — the integration plan inherits
this table verbatim.

| Sel | 32-bit GDT | 64-bit GDT |
|-----|------------|------------|
| `0x00` | null | null |
| `0x08` | code, D=1, 4 GiB limit | code, **L=1, D=0**, 4 GiB limit |
| `0x10` | data, 4 GiB limit | data |
| `0x18` | 16-bit code (thunk) | user data |
| `0x20` | 16-bit data (thunk) | user code |

All data selectors load into `DS/ES/FS/GS/SS` before entry. Interrupt state at entry: IF=0,
PIC unremapped and unmasked-as-shipped-by-the-BIOS (the kernel's own `interrupts.rs` remaps),
IDT untouched (IVT still live in low memory; the kernel installs its own before `STI`).

## 5. Low-memory reservation map (handed to the kernel)

The normalized map handed over at handoff carves the first megabyte exactly as follows; the
loader's scratch contract is identical (later phases never contradict this table):

| Range | Content | `MemoryKind` at handoff |
|-------|---------|--------------------------|
| `0x00000–0x003FF` | IVT | `Reserved` |
| `0x00400–0x004FF` | BDA | `Reserved` |
| `0x00500–0x005FF` | real-mode scratch | `BootloaderReclaimable` |
| `0x00600–0x007FF` | 16-bit thunk trampoline | `BootloaderReclaimable` |
| `0x00800–0x07BFF` | real-mode stack window (top `0x7A00`) | `BootloaderReclaimable` |
| `0x07C00–0x07DFF` | stage1 (MBR copy) | `BootloaderReclaimable` |
| `0x07E00–0x2FFFF` | stage2 image (code+data+BSS) | `BootloaderReclaimable` |
| `0x30000–0x30FFF` | `BootInfo` + normalized map array | `Reserved` |
| `0x31000–0x3FFFF` | unused | `Usable` |
| `0x40000–0x7BFFF` | bounce buffer | `BootloaderReclaimable` |
| `0x7C000–0x7FFFF` | stage2 PM stack + handoff stack | `Reserved` |
| `0x80000–0x9FFFF` | EBDA / top of conventional RAM | `Reserved` (clamped) |
| `0xA0000–0xFFFFF` | VGA VRAM / video BIOS / system ROM | `Reserved` (clamped) |

Above 1 MiB: `[kernel_phys_start .. page_table_end]` (64-bit) or
`[kernel_phys_start .. initrd_end]` (32-bit) is `KernelAndModules`; everything else follows the
BIOS-detected map. The `Reserved` BootInfo window means the kernel must copy what it wants
before reusing it — the integration plan may retire that window to `BootloaderReclaimable` once
it consumes eagerly; this plan keeps the conservative side.
