#![feature(proc_macro_diagnostic, proc_macro_quote)]

use proc_macro::{TokenStream, TokenTree, Diagnostic, Level, Span, quote};

#[proc_macro_attribute]
pub fn main(_args: TokenStream, input: TokenStream) -> TokenStream {
    let mut iter = input.clone().into_iter();
    check_next_ident(&mut iter, "fn");
    check_next_ident(&mut iter, "main");

    let tokens: TokenStream = iter.collect();

    quote!(
        #[no_mangle]
        pub fn _start() {
            __main();
            ::libwolffia::syscall::halt();
        }

        #[inline(always)]
        fn __main $tokens
    )
}

fn check_next_ident(iter: &mut impl Iterator<Item = TokenTree>, expected: &str) {
    match iter.next() {
        None => Diagnostic::spanned(Span::call_site(), Level::Error, "expected `fn main`").emit(),
        Some(TokenTree::Ident(ident)) if ident.to_string() == expected => {},
        Some(tt) => tt.span().error("expected `fn main`").emit(),
    }
}