ENTRY(_start)

SECTIONS
{
    . = 0xFFFF800000100000;

    .text : AT(. - 0xFFFF800000000000) {
        *(.text .text.*)
        *(.ltext .ltext.*)
    }

    .rodata : AT(. - 0xFFFF800000000000) {
        *(.rodata .rodata.*)
        *(.lrodata .lrodata.*)
    }

    .data : AT(. - 0xFFFF800000000000) {
        *(.data .data.*)
        *(.ldata .ldata.*)
    }

    .got : AT(. - 0xFFFF800000000000) {
        *(.got .got.plt)
    }

    .bss : AT(. - 0xFFFF800000000000) {
        *(.bss .bss.*)
        *(COMMON)
        *(.lbss .lbss.*)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note .note.*)
        *(.debug .debug.*)
    }
}
