/// Jumps to usermode.
pub unsafe fn jump_usermode(stack_ptr: usize, instruction_ptr: usize) -> ! {
    asm!(
        "
        mov ax, 0x2b
        mov ds, ax
        mov es, ax
        mov fs, ax
        mov gs, ax

        push 0x2b // stack segment
        push {0} // stack pointer
        pushfq // push RFLAGS
        push 0x33 // code segment
        push {1} // instruction pointer
        iretq
        ",
        in(reg) stack_ptr,
        in(reg) instruction_ptr,
    );

    unreachable!()
}
