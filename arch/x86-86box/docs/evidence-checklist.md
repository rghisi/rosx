# Evidence checklist — M5 items and their emulator analogs

One structured 86Box run produces the emulator-side analog of every AGENTS.md
hardware-checklist item that has one. **An emulator run never flips a silicon
checkbox** — `facts.md` opens with that sentence by construction. The tool
names below are MCP tools (`tools["86box"].*`); run held (server default).

## Run protocol

Machine: `configs/486sx-pvi486sp3.cfg` (486sp3c, i486SX-25, no FPU, 8 MB,
primed NVR). Media: the GRUB-Legacy HDD image from
`scripts/build-legacy-hdd.sh` (copy it into the VM directory as `hdd.img`).
Two boot scenarios, two run dirs:

- **run A — stock boot**: exercises items 2-7 and captures the stock
  Multiboot handoff.
- **run B — FLAGS-patched boot**: the mmap-less-bootloader scenario
  (item 1's fallback branch), via the boundary-probe stop.

Setup for both (and teardown, per gotchas.md): clean strays →
`vm_create(config=...)` → place media → `vm_start(held=true, binary=<86box-mcp>/docker/86box.sh)`
— the `binary` argument must be the container launcher, not the ELF path
(gotchas.md), and one `486sp3c` VM can serve both runs back-to-back
(`reset(kind:"hard")` between them; the entry breakpoint survives it).

Derived inputs, never hardcoded — read them from the exact shipped ELF:

```sh
nm target/rosx-i486/release/rosx-x86_32 | grep ' SYSTEM_TIME_MS$'   # PIT counter
readelf -h target/rosx-i486/release/rosx-x86_32 | grep Entry         # breakpoint addr
```

Run directory per §5 of the plan — `target/86box/runs/<UTC>-<machine>/`:

| file | produced by | when |
|---|---|---|
| `screen.txt` | `screen_text` | POST, bootloader banner, kernel banner, `rose>` |
| `screenshot.png` | `record_start(directory, sound:false)` → `step ms:600` → `record_stop`; copy one frame (`screenshot` alone returns the image inline, never a file) | at `rose>` |
| `vm-info.json` | `vm_info` | boot start and end (`hard_resets` both ends) |
| `mbi.hex` | `read_memory` at `%ebx` (run B only needs none — run A's stop) | first-instruction stop |
| `facts.md` | written last | observation table keyed to M5 items |

## Item → analog → tool sequence

### 1. Memory detection through a v1 bootloader (fallback branch)

Analog: a GRUB build that lacks the mmap buffer, synthesized at the true
handoff point. **run B sequence:**

```
vm_start(held=true, debugger=true, binary=<86box-mcp>/docker/86box.sh)
breakpoint_set(addr=<ELF entry>, kind="exec")
run_until_break  — repeat; each call covers ~10 s of emulated time, the
                   handoff lands near t≈15.5 s (Award POST + 5 s GRUB menu)
registers                      → %eax must be 0x2BADB002; %ebx = MBI pointer
read_memory(addr=%ebx, len=56) → decode FLAGS; record original (0x3E7)
write_memory(addr=%ebx, hex_bytes="01 00 00 00")   → sizes-only MBI
debug_continue
wait_for_text("rose>", timeout_ms=180000)
screen_text                    → [MEMORY] Region/Total sized from mem_upper
```

Pass: `[MEMORY] Total:` plausible for a 4-64 MB board and boot completes to
`rose>` (`mem` app: allocator healthy). run A records the complementary
fact — stock GRUB Legacy 0.97 passes `FLAGS=0x3E7` with a real e820 map, so
the same code takes the `mmap` branch; `mbi.hex` is that evidence.

### 2. PAUSE-as-NOP (no `#UD` at `Once` contention)

Analog: early boot contains the spin paths; a `#UD` on `f3 90` would
triple-fault the machine before `rose>`. run A: boot completes **and**
`vm_info.hard_resets` did not increment by itself. Record boot-to-`rose>`
completion + the hard_resets delta as the analog.

### 3. PIT 100 Hz / 8259A cascade timing

Analog: the SiS 496's 8253/8259A plumbing driven in held time. run A, at
`rose>`:

```
read_memory(addr=<SYSTEM_TIME_MS>, length=4)     → t1
step(ms=1000)
read_memory(addr=<SYSTEM_TIME_MS>, length=4)     → t2
```

Pass: `t2 - t1 == 1000` exactly (1:1). Then shell echo works:
`type_text("ls\n")`, `step ms:1500`, `screen_text` shows the listing.

### 4. 16-bit IDE PIO on a real drive

Analog: the era-chipset IDE controller. run A: the boot log shows
`[ATAD] ata0 drive0 ready: … sectors=…` (identify through 16-bit PIO);
then `type_text("ata\n")`, `step ms:3000`, `screen_text` — the `ata` app's
`mbr signature 55AA ok` line is the LBA-read evidence (the MBR was written
by sfdisk + GRUB setup, contents known).

### 5. VGA text console standalone (0xE9 writes harmless)

Analog: 86Box does not model the QEMU 0xE9 debug port — every kernel
debug-console write on this machine lands on an unclaimed port, so booting
to `rose>` with `screen_text` carrying the full banner **is** the check:
writes were harmless and VGA alone sufficed. Record with run A's captures.

### 6. The `rep bsf` idiom executes as BSF

Analog (weak): the kernel's bit-scan paths run continuously through boot
and the `mem` app (`type_text("mem\n")` → allocator healthy, no fault).
The static guard is `cargo xtask check-isa x86_32` (`rep bsf` prints as
`tzcnt`, legal F3-prefixed legacy); the emulator adds that the idiom
executes on a modeled 486 core without faulting. facts.md must label this
analog honestly.

### 7. 486SX sub-case — no FPU present

Analog: the baseline machine *is* the sub-case. run A:
`vm_info` shows `i486SX`, `fpu_registers` returns `no FPU`, and the kernel
ran FP-free end to end to `rose>`.

## facts.md template

```
Emulator runs never flip a silicon checkbox; the seven hardware items
close on silicon only. This file is emulator evidence for the 486-class
contract (machine 486sp3c, emulated i486SX-25, no FPU).

- POST: 80486SX CPU at 25MHz / Memory Test: 8192K OK         [screen.txt]
- hard_resets: <start> → <end> (delta explained: <who typed what>)
- PIT: SYSTEM_TIME_MS <t1> → <t2> over step ms:1000 (delta = <n>)
- ATAD: <identify line> ; ata app: <55AA line>
- mem app: <total/free>
- MBI: FLAGS=<hex> mem_lower=<h> mem_upper=<h> e820@<addr> (<n> entries)
- boot media matrix row: <GRUB Legacy 0.97-67 HDD | stock|FLAGS-patched>
- M5 analogs covered: <item numbers> — status per item above
```
