#![feature(asm)]

#![no_std]
#![no_main]

use libwolffia::prelude::*;

#[libwolffia::main]
fn main() {
    println!("Hello, world!");
    unsafe { asm!("xor rax, rax", lateout("rax") _); }
    println!("Halting...");
}
