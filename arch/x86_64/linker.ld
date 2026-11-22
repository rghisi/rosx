ENTRY(_start)

SECTIONS
{
    . = 0xFFFF800000100000;

    .text : AT(. - 0xFFFF800000000000) {
        *(.text .text.*)
    }

    .rodata : AT(. - 0xFFFF800000000000) {
        *(.rodata .rodata.*)
    }

    .data : AT(. - 0xFFFF800000000000) {
        *(.data .data.*)
    }

    .got : AT(. - 0xFFFF800000000000) {
        *(.got .got.plt)
    }

    .bss : AT(. - 0xFFFF800000000000) {
        *(.bss .bss.*)
        *(COMMON)
    }

    kernel_end = .;
    kernel_phys_end = kernel_end - 0xFFFF800000000000;

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.comment)
        *(.note .note.*)
        *(.debug .debug.*)
    }
}
