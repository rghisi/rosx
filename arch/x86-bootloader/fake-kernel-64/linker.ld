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

    .bss : AT(. - 0xFFFF800000000000) {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note .note.*)
        *(.debug .debug.*)
    }
}
