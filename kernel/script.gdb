target remote localhost:1234
set architecture i386:x86-64
symbol-file build/debug/kernel.elf
break interrupts::exceptions::page_fault
