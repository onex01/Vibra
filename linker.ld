OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)

ENTRY(_start)

PHDRS
{
    text    PT_LOAD FLAGS(5); /* Read + Execute */
    rodata  PT_LOAD FLAGS(4); /* Read */
    data    PT_LOAD FLAGS(6); /* Read + Write */
}

SECTIONS
{
    . = 0xffffffff80000000;

    __text_start = .;
    .text : {
        *(.text .text.*)
    } :text
    __text_end = .;

    . += CONSTANT(MAXPAGESIZE);

    __rodata_start = .;
    .rodata : {
        *(.rodata .rodata.*)
    } :rodata
    __rodata_end = .;

    . += CONSTANT(MAXPAGESIZE);

    __data_start = .;
    .data : {
        *(.data .data.*)
    } :data

    __bss_start = .;
    .bss : {
        *(COMMON)
        *(.bss .bss.*)
    } :data
    __bss_end = .;

    /DISCARD/ : {
        *(.eh_frame)
        *(.note .note.*)
    }
}
