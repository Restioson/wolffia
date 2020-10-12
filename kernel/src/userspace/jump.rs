use crate::gdt::{GDT};

/// Jumps to usermode.
#[naked]
pub unsafe fn jump_usermode(stack_ptr: usize, instruction_ptr: usize) -> ! {
    let _ds = GDT.selectors.user_ds.0;
    let _cs = GDT.selectors.user_cs.0; // TODO

    asm!(
        "
        mov ax, 0x33
        mov ds, ax
        mov es, ax
        mov fs, ax
        mov gs, ax
        mov rsp, {0}

        mov rax, rsp
        push 0x33
        push rax
        pushfq

        push 0x2b
        mov rax, {1}
        push rax
        iretq,
        ",
        in(reg) stack_ptr,
        in(reg) instruction_ptr,
        lateout("rax") _
    );

    unreachable!()
}
