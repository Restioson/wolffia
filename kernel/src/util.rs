/// Round up integer division
pub const fn round_up_divide(x: u64, y: u64) -> u64 {
    (x + y - 1) / y
}

macro_rules! constant_unroll {
    (
        for $for_var:ident in [$($item:expr),*] {
            $iter_:ident = $iter:ident.$iter_fn:ident(move |$iter_var:ident| $block:block);
        }
    ) => {
        {
            $(
                let $iter = {
                    let $for_var = $item;
                    let $iter = $iter.$iter_fn(move |$iter_var| { $block });
                    $iter
                };
            )*

            $iter
        }
    }
}

pub unsafe fn memset_volatile_64bit(s: *mut u64, c: u64, n: usize) -> *mut u64 {
    assert_eq!(n & 0b111, 0, "n must a be multiple of 8");
    assert_eq!(s as usize & 0b111, 0, "ptr must be aligned to 8 bytes");
    let mut i = 0;
    while i < (n >> 3) {
        core::ptr::write_volatile(s.offset(i as isize), c);
        i += 1;
    }
    s
}