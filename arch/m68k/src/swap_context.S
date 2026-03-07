/* swap_context.S — m68k cooperative context switch
 *
 * swap_context(store: *mut usize, load: usize)
 *
 * m68k cdecl calling convention:
 *   [sp+4]  = store — address where current sp should be saved (may be null/0)
 *   [sp+8]  = load  — sp value to restore for the next task
 *
 * A suspended task's stack frame (from lowest address, sp points here):
 *   [sp+ 0]  D2
 *   [sp+ 4]  D3
 *   [sp+ 8]  D4
 *   [sp+12]  D5
 *   [sp+16]  D6
 *   [sp+20]  D7
 *   [sp+24]  A2
 *   [sp+28]  A3
 *   [sp+32]  A4
 *   [sp+36]  A5
 *   [sp+40]  A6
 *   [sp+44]  return address / task entry point  ← popped by rts
 */

        .text

/* __mulsi3 — 32×32→32 signed/unsigned multiply (lower 32 bits of product)
 *
 * The 68000 only has 16×16→32 multiply (mulu.w / muls.w). compiler_builtins'
 * Rust implementation creates a circular dependency (uses u64 ops which need
 * this function), so we provide a direct assembly version.
 *
 * cdecl: [sp+4] = a (u32), [sp+8] = b (u32), return in d0
 *
 * result = a_lo*b_lo + ((a_hi*b_lo + a_lo*b_hi) << 16)
 * where a_hi/b_hi are upper 16 bits, a_lo/b_lo are lower 16 bits.
 */
        .global __mulsi3
__mulsi3:
        move.w  6(%sp), %d0             /* d0 = a_lo */
        move.w  10(%sp), %d1            /* d1 = b_lo */
        mulu.w  %d1, %d0                /* d0 = a_lo * b_lo (32-bit result) */
        move.w  4(%sp), %d1             /* d1 = a_hi */
        mulu.w  10(%sp), %d1            /* d1 = a_hi * b_lo */
        swap    %d1
        clr.w   %d1                     /* d1 = (a_hi * b_lo) << 16 */
        add.l   %d1, %d0
        move.w  6(%sp), %d1             /* d1 = a_lo */
        mulu.w  8(%sp), %d1             /* d1 = a_lo * b_hi */
        swap    %d1
        clr.w   %d1                     /* d1 = (a_lo * b_hi) << 16 */
        add.l   %d1, %d0
        rts

        .global swap_context
swap_context:
        ori.w   #0x0700, %sr                    /* disable interrupts (IPL=7) */
        move.l  4(%sp), %a0                     /* a0 = store ptr */
        move.l  8(%sp), %a1                     /* a1 = load value */
        movem.l %d2-%d7/%a2-%a6, -(%sp)        /* push 11 callee-saved regs (44 bytes) */
        cmpa.l  #0, %a0
        beq.s   1f
        move.l  %sp, (%a0)                      /* *store = sp */
1:
        move.l  %a1, %sp                        /* switch to next task's stack */
        movem.l (%sp)+, %d2-%d7/%a2-%a6        /* pop 11 callee-saved regs */
        andi.w  #0xF8FF, %sr                    /* re-enable interrupts (IPL=0) */
        rts                                     /* jump to task's saved entry point */
