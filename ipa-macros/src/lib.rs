mod derive_step;
mod tree;
use proc_macro::TokenStream;

#[proc_macro_derive(Step)]
pub fn derive_step(input: TokenStream) -> TokenStream {
    derive_step::expand(input)
}
