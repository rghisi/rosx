ENTRY(_start)

SECTIONS
{
    . = 0x7E00;

    .text : {
        *(.text.entry)
        *(.entry_rodata)
        *(.text .text.*)
    }

    .longmode.text : {
        *(.longmode.text)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data : {
        *(.data .data.*)
    }

    .bss : {
        _bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        _bss_end = .;
    }

    ASSERT(. <= 0x30000, "stage2 image exceeds the 0x7E00..0x2FFFF reservation")

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note .note.*)
        *(.debug .debug.*)
    }
}
