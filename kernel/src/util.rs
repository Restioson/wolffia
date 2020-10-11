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
