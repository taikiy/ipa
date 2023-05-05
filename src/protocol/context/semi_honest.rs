use crate::{
    ff::Field,
    helpers::{ChannelId, Gateway, Message, ReceivingEnd, Role, SendingEnd, TotalRecords},
    protocol::{
        context::{
            Context, InstrumentedIndexedSharedRandomness, InstrumentedSequentialSharedRandomness,
            MaliciousContext,
        },
        malicious::MaliciousValidatorAccumulator,
        prss::Endpoint as PrssEndpoint,
        step::{Gate, Step, StepNarrow},
        RecordId,
    },
    secret_sharing::replicated::{
        malicious::ExtendableField, semi_honest::AdditiveShare as Replicated,
    },
    seq_join::SeqJoin,
    sync::Arc,
};
use std::{
    fmt::{Debug, Formatter},
    num::NonZeroUsize,
};

/// Context for protocol executions suitable for semi-honest security model, i.e. secure against
/// honest-but-curious adversary parties.
#[derive(Clone)]
pub struct SemiHonestContext<'a, G: Gate> {
    /// TODO (alex): Arc is required here because of the `TestWorld` structure. Real world
    /// may operate with raw references and be more efficient
    pub(super) inner: Arc<ContextInner<'a, G>>,
    pub(super) step: G,
    pub(super) total_records: TotalRecords,
}

impl<'a, G: Gate> SemiHonestContext<'a, G> {
    pub fn new(participant: &'a PrssEndpoint, gateway: &'a Gateway<G>) -> Self {
        Self::new_complete(
            participant,
            gateway,
            G::default(),
            TotalRecords::Unspecified,
        )
    }

    pub fn new_with_total_records(
        participant: &'a PrssEndpoint,
        gateway: &'a Gateway<G>,
        total_records: TotalRecords,
    ) -> Self {
        Self::new_complete(participant, gateway, G::default(), total_records)
    }

    pub(super) fn new_complete(
        participant: &'a PrssEndpoint,
        gateway: &'a Gateway<G>,
        step: G,
        total_records: TotalRecords,
    ) -> Self {
        Self {
            inner: ContextInner::new(participant, gateway),
            step,
            total_records,
        }
    }

    /// Upgrade this context to malicious.
    /// `malicious_step` is the step that will be used for malicious protocol execution.
    /// `upgrade_step` is the step that will be used for upgrading inputs
    /// from `replicated::semi_honest::AdditiveShare` to `replicated::malicious::AdditiveShare`.
    /// `accumulator` and `r_share` come from a `MaliciousValidator`.
    #[must_use]
    pub fn upgrade<S: Step + ?Sized, F: Field + ExtendableField>(
        self,
        malicious_step: &S,
        accumulator: MaliciousValidatorAccumulator<F>,
        r_share: Replicated<F::ExtendedField>,
    ) -> MaliciousContext<'a, F, G> {
        MaliciousContext::new(&self, malicious_step, accumulator, r_share)
    }
}

impl<'a, G: Gate> Context<G> for SemiHonestContext<'a, G> {
    fn role(&self) -> Role {
        self.inner.gateway.role()
    }

    fn step(&self) -> &G {
        &self.step
    }

    fn narrow<S: Step + ?Sized>(&self, step: &S) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            step: self.step.narrow(step),
            total_records: self.total_records,
        }
    }

    fn set_total_records<T: Into<TotalRecords>>(&self, total_records: T) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            step: self.step.clone(),
            total_records: self.total_records.overwrite(total_records),
        }
    }

    fn is_last_record<I: Into<RecordId>>(&self, record_id: I) -> bool {
        self.total_records.is_last(record_id)
    }

    fn prss(&self) -> InstrumentedIndexedSharedRandomness {
        let prss = self.inner.prss.indexed(self.step());

        InstrumentedIndexedSharedRandomness::new(prss, &self.step, self.role())
    }

    fn prss_rng(
        &self,
    ) -> (
        InstrumentedSequentialSharedRandomness<'_>,
        InstrumentedSequentialSharedRandomness<'_>,
    ) {
        let (left, right) = self.inner.prss.sequential(self.step());
        (
            InstrumentedSequentialSharedRandomness::new(left, self.step(), self.role()),
            InstrumentedSequentialSharedRandomness::new(right, self.step(), self.role()),
        )
    }

    fn send_channel<M: Message>(&self, role: Role) -> SendingEnd<M, G> {
        self.inner
            .gateway
            .get_sender(&ChannelId::new(role, self.step.clone()), self.total_records)
    }

    fn recv_channel<M: Message>(&self, role: Role) -> ReceivingEnd<M> {
        self.inner
            .gateway
            .get_receiver(&ChannelId::new(role, self.step.clone()))
    }
}

impl<G: Gate> SeqJoin for SemiHonestContext<'_, G> {
    fn active_work(&self) -> NonZeroUsize {
        self.inner.gateway.config().active_work()
    }
}

impl<G: Gate> Debug for SemiHonestContext<'_, G> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "SemiHonestContext")
    }
}

pub(super) struct ContextInner<'a, G: Gate> {
    pub prss: &'a PrssEndpoint,
    pub gateway: &'a Gateway<G>,
}

impl<'a, G: Gate> ContextInner<'a, G> {
    fn new(prss: &'a PrssEndpoint, gateway: &'a Gateway<G>) -> Arc<Self> {
        Arc::new(Self { prss, gateway })
    }
}
