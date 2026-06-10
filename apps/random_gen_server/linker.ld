ENTRY(_start)

SECTIONS
{
    . = 0;

    .text : {
        *(.text._start)
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data.rel.ro : {
        *(.data.rel.ro .data.rel.ro.*)
    }

    .dynamic : {
        *(.dynamic)
    }

    .got : {
        *(.got .got.*)
    }

    .got.plt : {
        *(.got.plt)
    }

    .data : {
        *(.data .data.*)
    }

    .bss : {
        *(.bss .bss.*)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
