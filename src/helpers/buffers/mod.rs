mod ordering_mpsc;
mod ordering_sender;
mod unordered_receiver;

pub use ordering_mpsc::{ordering_mpsc, OrderingMpscReceiver, OrderingMpscSender};
pub use ordering_sender::{OrderedStream, OrderingSender};
pub use unordered_receiver::UnorderedReceiver;

#[cfg(debug_assertions)]
#[allow(unused)] // todo(alex): make test world print the state again
mod waiting {
    use crate::{helpers::ChannelId, protocol::step::Gate};
    use std::collections::HashMap;

    pub(in crate::helpers) struct WaitingTasks<'a, G: Gate> {
        tasks: HashMap<&'a ChannelId<G>, Vec<u32>>,
    }

    impl<'a, G: Gate> WaitingTasks<'a, G> {
        pub fn new(tasks: HashMap<&'a ChannelId<G>, Vec<u32>>) -> Self {
            Self { tasks }
        }

        pub fn is_empty(&self) -> bool {
            self.tasks.is_empty()
        }
    }

    impl<G: Gate> std::fmt::Debug for WaitingTasks<'_, G> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "[")?;
            for (channel, records) in &self.tasks {
                write!(f, "\n    {channel:?}: {records:?}")?;
            }
            write!(f, "\n]")?;

            Ok(())
        }
    }
}
