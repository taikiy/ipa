use super::{convert_triples_to_shares, random_bits_triples, RandomBits, Step};
use crate::{
    error::Error,
    ff::PrimeField,
    protocol::{
        context::{Context, MaliciousContext},
        step::{BitOpStep, Gate},
        RecordId,
    },
    secret_sharing::replicated::malicious::{
        AdditiveShare as MaliciousReplicated, ExtendableField,
    },
    seq_join::SeqJoin,
};
use async_trait::async_trait;

#[async_trait]
impl<F: PrimeField + ExtendableField, G: Gate> RandomBits<F> for MaliciousContext<'_, F, G> {
    type Share = MaliciousReplicated<F>;

    /// Generates a sequence of `l` random bit sharings in the target field `F`.
    async fn generate_random_bits(self, record_id: RecordId) -> Result<Vec<Self::Share>, Error> {
        let triples = random_bits_triples::<F, G, _>(&self, record_id);

        // Upgrade the replicated shares to malicious, in parallel,
        let c = self.narrow(&Step::UpgradeBitTriples);
        let ctx = &c;
        let malicious_triples = ctx
            .parallel_join(triples.into_iter().enumerate().map(|(i, t)| async move {
                ctx.narrow(&BitOpStep::from(i))
                    .upgrade_for(record_id, t)
                    .await
            }))
            .await?;

        convert_triples_to_shares(
            self.narrow(&Step::ConvertShares),
            record_id,
            &malicious_triples,
        )
        .await
    }
}
