use crate::{
    error::Error,
    ff::Field,
    protocol::{
        context::{Context, MaliciousContext, SemiHonestContext},
        step::Gate,
        RecordId,
    },
    secret_sharing::replicated::{
        malicious::{AdditiveShare as MaliciousReplicated, ExtendableField},
        semi_honest::AdditiveShare as Replicated,
    },
};
use async_trait::async_trait;

pub(crate) mod malicious;
mod semi_honest;

#[async_trait]
pub trait SumOfProducts<C: Context<G>, G: Gate>: Sized {
    async fn sum_of_products<'fut>(
        ctx: C,
        record_id: RecordId,
        a: &[Self],
        b: &[Self],
    ) -> Result<Self, Error>
    where
        C: 'fut;
}

#[async_trait]
impl<'a, F: Field, G: Gate> SumOfProducts<SemiHonestContext<'a, G>, G> for Replicated<F> {
    async fn sum_of_products<'fut>(
        ctx: SemiHonestContext<'a, G>,
        record_id: RecordId,
        a: &[Self],
        b: &[Self],
    ) -> Result<Self, Error>
    where
        'a: 'fut,
    {
        semi_honest::sum_of_products(ctx, record_id, a, b).await
    }
}

#[async_trait]
impl<'a, F: Field + ExtendableField, G: Gate> SumOfProducts<MaliciousContext<'a, F, G>, G>
    for MaliciousReplicated<F>
{
    async fn sum_of_products<'fut>(
        ctx: MaliciousContext<'a, F, G>,
        record_id: RecordId,
        a: &[Self],
        b: &[Self],
    ) -> Result<Self, Error>
    where
        'a: 'fut,
    {
        malicious::sum_of_products(ctx, record_id, a, b).await
    }
}
