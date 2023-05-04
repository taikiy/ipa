mod check_zero;
mod if_else;
pub(crate) mod mul;
mod reshare;
mod reveal;
mod share_known_value;
mod sum_of_product;

pub use check_zero::check_zero;
pub use if_else::if_else;
pub use mul::{MultiplyZeroPositions, SecureMul, ZeroPositions};
pub use reshare::Reshare;
pub use reveal::Reveal;
pub use share_known_value::ShareKnownValue;
pub use sum_of_product::SumOfProducts;

use crate::{
    ff::Field,
    protocol::{
        context::{Context, MaliciousContext, SemiHonestContext},
        RecordId,
    },
    secret_sharing::{
        replicated::{
            malicious::{AdditiveShare as MaliciousAdditiveShare, ExtendableField},
            semi_honest::AdditiveShare,
        },
        SharedValue,
    },
};

use super::step::Gate;

pub trait BasicProtocols<C: Context<G>, G: Gate, V: SharedValue>:
    Reshare<C, G, RecordId>
    + Reveal<C, G, RecordId, Output = V>
    + SecureMul<C, G>
    + ShareKnownValue<C, G, V>
    + SumOfProducts<C, G>
{
}

impl<'a, F: Field, G: Gate> BasicProtocols<SemiHonestContext<'a, G>, G, F> for AdditiveShare<F> {}

impl<'a, F: Field + ExtendableField, G: Gate> BasicProtocols<MaliciousContext<'a, F, G>, G, F>
    for MaliciousAdditiveShare<F>
{
}
