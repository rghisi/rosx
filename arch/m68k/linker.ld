/* linker.ld — RosX m68k linker script
 *
 * Memory layout (RAM starts at 0x00000000 on QEMU virt/m68000):
 *
 *   0x00000000  .vectors    — 256 × 4 = 1 KB exception vector table
 *   0x00000400  .bss        — zero-initialised kernel statics (NOBITS)
 *               .data       — initialised data
 *               .rodata     — read-only constants and format strings
 *               .text       — executable code  (~30 KB release build)
 *               .boot_stack — 64 KB initial supervisor stack (NOBITS)
 *               kernel_end  — heap starts here
 *
 * LLVM 18's m68k backend always emits PC16 (16-bit PC-relative) for data
 * references.  The 68000 PC16 range is ±32 KB.  By placing .bss/.data/.rodata
 * BEFORE .text we guarantee that all data symbols have small NEGATIVE offsets
 * from any instruction in .text, as long as (bss + data + rodata + text) fits
 * in ~64 KB.  The 64 KB boot stack is kept AFTER .text to avoid inflating this
 * distance.
 *
 * boot.S (GNU assembler) generates absolute R_68K_32 references for the
 * vector table entries, _boot_stack_top, and the BSS-zeroing loop — those
 * work regardless of layout.
 */

ENTRY(_start)

SECTIONS {
    . = 0x00000000;

    .vectors ALIGN(4) : {
        KEEP(*(.vectors))
    }

    /* Kernel statics before .text so PC16 offsets are small and negative */

    .bss ALIGN(4) : {
        _bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        _bss_end = .;
    }

    .data ALIGN(4) : {
        *(.data .data.*)
    }

    .rodata ALIGN(4) : {
        *(.rodata .rodata.*)
    }

    .text ALIGN(4) : {
        *(.text .text.*)
    }

    /* Boot stack AFTER .text — 64 KB, not between statics and code */
    .boot_stack ALIGN(4) : {
        *(.boot_stack)
    }

    kernel_end = .;
}
