use crate::gdt::{GDT, TSS};
use x86_64::VirtAddr;

/// Jumps to usermode.
#[naked]
pub unsafe fn jump_usermode(stack_ptr: usize, instruction_ptr: usize) -> ! {
    let ds = GDT.selectors.user_ds.0;
    let cs = GDT.selectors.user_cs.0;

    let kernel_rsp: u64;
    asm!("mov {}, rsp", out(reg) kernel_rsp);

    TSS.wait().unwrap().lock().tss.get_mut().privilege_stack_table[0] = VirtAddr::new(kernel_rsp);

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
