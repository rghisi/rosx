# Evidence checklist — hardware-checklist analogs on the production chain

One structured 86Box run produces the emulator-side analog of every AGENTS.md
hardware-checklist item that has one. **An emulator run never flips a silicon
checkbox** — `facts.md` opens with that sentence by construction. The tool
names below are MCP tools (`tools["86box"].*`); run held (server default).

## Run protocol

Machine: a template from `configs/` (baseline `486sx-pvi486sp3.cfg` —
486sp3c, i486SX-25, no FPU, primed NVR). Media: the **production unified
disk** — `target/rosx-i486-boot-disk.img` from `cargo xtask build x86_32`,
copied into the VM directory as the file the cfg names in `hdd_01_fn`. One
run per board covers every item; phase V measured AMI (`vect486vl`),
Phoenix (`ecs486`) and Award (`486sp3c`, grown 512 MB **and** the un-grown
64 MiB artifact booting as-is).

Derived inputs, never hardcoded — read them from the exact shipped ELF:

```sh
nm target/rosx-i486/release/rosx-x86_32 | grep SYSTEM_TIME_MS   # PIT counter
```

The symbol is Rust-mangled (trailing `17<hash>` part) — match without an
end anchor and take the printed address.

Run directory per the plan — `target/86box/runs/<UTC>-<machine>/`:

| file | produced by | when |
|---|---|---|
| `screen.txt` | `screen_text` | POST, stage banner burst, kernel banner, `rose>` |
| `screenshot.png` | `screenshot` (inline frame; `record_start` is broken in the server venv — see gotchas.md) | at `rose>` |
| `vm-info.json` | `vm_info` | boot start and end (`hard_resets` both ends) |
| `facts.md` | written last | observation table keyed to the items below |

## Item → analog → tool sequence

### 1. Memory detection with synthesis fallbacks

The production chain probes E820, then E801, then INT 88h; the BIOS decides
which branch runs. Pass on every board: the `[BOOTINFO] regions=… usable=…`
banner is plausible for the machine's RAM and boot completes to `rose>`.
Measured branch coverage: Award accepts E820; Phoenix refuses E820 (E801
synthesis proven); AMI Vectra refuses E820 **and** E801 (INT 88h synthesis
proven) — the fallback branches need no patching trick, the firmware walks
them natively.

### 2. PAUSE-as-NOP (no `#UD` at `Once` contention)

Analog: early boot contains the spin paths; a `#UD` on `f3 90` would
triple-fault the machine before `rose>`. Boot completes **and**
`vm_info.hard_resets` did not increment by itself. Record boot-to-`rose>`
completion + the hard_resets delta as the analog.

### 3. PIT 100 Hz / 8259A cascade timing

Analog: the era chipset's 8253/8259A plumbing driven in held time. At
`rose>`:

```
read_memory(addr=<SYSTEM_TIME_MS>, length=4)     → t1
step(ms=1000)
read_memory(addr=<SYSTEM_TIME_MS>, length=4)     → t2
```

Pass: `t2 - t1 == 1000` exactly (1:1). Then shell echo works:
`type_text("ls\n")`, `step ms:1500`, `screen_text` shows the listing.

### 4. 16-bit IDE PIO on a real drive

Analog: the era-chipset IDE controller. The boot log shows
`[ATAD] ata0 drive0 ready: … sectors=…` (identify through 16-bit PIO);
then `type_text("ata\n")`, `step ms:3000`, `screen_text` — the `ata` app's
`mbr signature 55AA ok` line is the LBA-read evidence (the MBR is stage1,
written by the unified-disk build, contents known).

### 5. VGA text console standalone (0xE9 writes harmless)

Analog: 86Box does not model the QEMU 0xE9 debug port — every kernel
debug-console write on this machine lands on an unclaimed port, so booting
to `rose>` with `screen_text` carrying the full banner **is** the check:
writes were harmless and VGA alone sufficed. Record with the run's captures.

### 6. The `rep bsf` idiom executes as BSF

Analog (weak): the kernel's bit-scan paths run continuously through boot
and the `mem` app (`type_text("mem\n")` → allocator healthy, no fault).
The static guard is `cargo xtask check-isa x86_32` (`rep bsf` prints as
`tzcnt`, legal F3-prefixed legacy); the emulator adds that the idiom
executes on a modeled 486 core without faulting. facts.md must label this
analog honestly.

### 7. 486SX sub-case — no FPU present

Analog: the baseline machine *is* the sub-case.
`vm_info` shows `i486SX`, `fpu_registers` returns `no FPU`, and the kernel
ran FP-free end to end to `rose>`.

## facts.md template

```
Emulator runs never flip a silicon checkbox; the seven hardware items
close on silicon only. This file is emulator evidence for the 486-class
contract (machine <id>, emulated i486SX-25, no FPU).

- POST: 80486SX CPU at 25MHz / Memory Test: <n>K OK              [screen.txt]
- chain: [STAGE1] … [DISK] media=<chs|lba> … [BOOTINFO] regions=… usable=…
- memory branch: <e820 | e801-synthesized | e88-synthesized>
- hard_resets: <start> → <end> (delta explained: <who typed what>)
- PIT: SYSTEM_TIME_MS <t1> → <t2> over step ms:1000 (delta = <n>)
- ATAD: <identify line> ; ata app: <55AA line>
- mem app: <total/free>
- media: target/rosx-i486-boot-disk.img <as-shipped | grown to <bytes>>
- analogs covered: <item numbers> — status per item above
```
