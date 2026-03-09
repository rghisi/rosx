/* boot.S — 68000 reset vector table and startup code
 *
 * The 68000 reads vector[0] as the initial SSP and vector[1] as the reset PC
 * before executing any instruction. The full 1 KB vector table must be the
 * very first bytes of the binary (placed in .vectors by the linker script).
 *
 * Interrupt auto-vectors:
 *   vector 25 (0x64): IRQ level 1
 *   vector 26 (0x68): IRQ level 2
 *   vector 27 (0x6c): IRQ level 3
 *   vector 28 (0x70): IRQ level 4
 *   vector 29 (0x74): IRQ level 5
 *   vector 30 (0x78): IRQ level 6  ← Goldfish RTC timer (via PIC #5)
 *   vector 31 (0x7c): IRQ level 7
 *   vector 32 (0x80): TRAP #0      ← syscall
 */

        .section .vectors, "a"
        .global _vectors
_vectors:
        .long   _boot_stack_top         /* vector  0: initial SSP                */
        .long   _start                  /* vector  1: reset PC                   */
        .long   _default_exception      /* vector  2: bus error                  */
        .long   _default_exception      /* vector  3: address error              */
        .long   _default_exception      /* vector  4: illegal instruction        */
        .long   _default_exception      /* vector  5: zero divide                */
        .long   _default_exception      /* vector  6: CHK                        */
        .long   _default_exception      /* vector  7: TRAPV                      */
        .long   _default_exception      /* vector  8: privilege violation        */
        .long   _default_exception      /* vector  9: trace                      */
        .long   _default_exception      /* vector 10: line A emulator            */
        .long   _default_exception      /* vector 11: line F emulator            */
        .rept   13
        .long   _default_exception      /* vectors 12-24: reserved/other         */
        .endr
        .long   _default_exception      /* vector 25: IRQ level 1 autovector     */
        .long   _default_exception      /* vector 26: IRQ level 2 autovector     */
        .long   _default_exception      /* vector 27: IRQ level 3 autovector     */
        .long   _default_exception      /* vector 28: IRQ level 4 autovector     */
        .long   _default_exception      /* vector 29: IRQ level 5 autovector     */
        .long   _timer_isr              /* vector 30: IRQ level 6 autovector     */
        .long   _default_exception      /* vector 31: IRQ level 7 autovector     */
        .long   _trap0_handler          /* vector 32: TRAP #0 (syscall)          */
        .rept   223
        .long   _default_exception      /* vectors 33-255: default               */
        .endr

/* ── Initial boot stack — placed after .text to keep .bss small ─────────────
 *
 * Uses its own section so the linker script can position it AFTER .text.
 * This keeps the distance between .bss (kernel statics) and .text within
 * the 68000's ±32 KB PC16 range.
 */
        .section .boot_stack, "aw", @nobits
        .balign 4
_boot_stack_bottom:
        .skip   65536
        .global _boot_stack_top
_boot_stack_top:

/* ── Entry point ────────────────────────────────────────────────────────────── */

        .section .text
        .global _start
_start:
        lea.l   _boot_stack_top, %a7    /* reinitialise SSP (redundant but explicit) */

        /* zero BSS */
        lea.l   _bss_start, %a0
        lea.l   _bss_end,   %a1
.zero_bss:
        cmpa.l  %a1, %a0
        bge.s   .bss_done
        clr.l   (%a0)+
        bra.s   .zero_bss
.bss_done:
        jsr     kernel_main

.hang:
        stop    #0x2700
        bra.s   .hang

/* ── Default exception handler — halt the machine ──────────────────────────── */

        .global _default_exception
_default_exception:
        stop    #0x2700
        bra.s   _default_exception

/* ── Timer ISR trampoline (IRQ level 6) ─────────────────────────────────────
 *
 * On IRQ entry the CPU has already pushed [SR (2 bytes), PC (4 bytes)] = 6
 * bytes onto the supervisor stack. We save the caller-saved registers, call
 * the Rust timer handler, restore them, and return via rte.
 */
        .global _timer_isr
_timer_isr:
        movem.l %d0-%d1/%a0-%a1, -(%sp)    /* save caller-saved regs (16 bytes) */
        jsr     timer_interrupt_handler_rs
        movem.l (%sp)+, %d0-%d1/%a0-%a1    /* restore caller-saved regs */
        rte

/* ── TRAP #0 syscall trampoline ─────────────────────────────────────────────
 *
 * Calling convention (matching usrlib/src/arch/m68k.rs):
 *   d0 = syscall number
 *   d1 = arg1
 *   d2 = arg2
 *   d3 = arg3
 *   return value: d0
 */
/* ── Absolute-address helper for linker-script symbol 'kernel_end' ──────────
 *
 * 'kernel_end' is an external declaration from the Rust perspective. LLVM 18's
 * m68k backend generates PC16 for external declarations, which overflows in
 * large binaries. GNU assembler uses R_68K_32 (absolute) for undefined symbols
 * by default, bypassing the LLVM PC16 limitation.
 *
 * Returns: address of kernel_end in d0 (m68k cdecl return register)
 */
        .global get_kernel_end_addr
get_kernel_end_addr:
        lea.l   kernel_end, %a0
        move.l  %a0, %d0
        rts

        .global _trap0_handler
_trap0_handler:
        movem.l %d1-%d3/%a0-%a1, -(%sp)    /* save caller-saved regs */
        move.l  %d3, -(%sp)                 /* cdecl arg3 */
        move.l  %d2, -(%sp)                 /* cdecl arg2 */
        move.l  %d1, -(%sp)                 /* cdecl arg1 */
        move.l  %d0, -(%sp)                 /* cdecl num  */
        jsr     syscall_handler
        add.l   #16, %sp                    /* discard 4 cdecl args */
        movem.l (%sp)+, %d1-%d3/%a0-%a1
        rte
