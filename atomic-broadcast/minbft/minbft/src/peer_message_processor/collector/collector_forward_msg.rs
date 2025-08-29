use crate::peer_message::usig_message::UsigMessage;
use crate::Config;
use std::{collections::HashMap, fmt::Debug, marker::PhantomData};
use tracing::trace;
use usig::{Count, Counter, ReplicaId};

use crate::peer_message::usig_message::view_peer_message::ViewPeerMessage;

#[derive(Debug, Clone)]
pub(crate) struct CollectorForwards<P, Sig, Att> {
    //for every replica, store a vector with collected counters
    #[allow(clippy::type_complexity)]
    recv_forwards: Vec<Option<Vec<Option<(Count, Vec<UsigMessage<P, Sig, Att>>)>>>>,
    _phantom: PhantomData<(P, Sig)>,
}

impl<P: Clone + Debug, Sig: Counter + Clone + Debug, Att: Clone + Debug>
    CollectorForwards<P, Sig, Att>
{
    pub(crate) fn new(config: &Config) -> CollectorForwards<P, Sig, Att> {
        let n = config.n.get() as usize;
        let mut recv_forwards = vec![Some(vec![None; n]); n];
        recv_forwards[config.id.as_u64() as usize] = None;

        CollectorForwards {
            recv_forwards,
            _phantom: PhantomData,
        }
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn collect(
        &mut self,
        forwarder: ReplicaId,
        msg: ViewPeerMessage<P, Sig, Att>,
        pending_new_views: Vec<UsigMessage<P, Sig, Att>>,
        config: &Config,
    ) -> Vec<(ReplicaId, (Count, Vec<UsigMessage<P, Sig, Att>>))> {
        {
            trace!(
                "Inserted forward message into forward message collector: {:?}",
                msg
            );

            let original_sender = {
                match &msg {
                    ViewPeerMessage::Prepare(prepare) => prepare.origin,
                    ViewPeerMessage::Commit(commit) => commit.origin,
                }
            };

            if let Some(Some(received)) = self
                .recv_forwards
                .get_mut(original_sender.as_u64() as usize)
            {
                received[forwarder.as_u64() as usize] = Some((msg.counter(), pending_new_views));
            }
        }
        {
            let mut vec: Vec<(ReplicaId, (Count, Vec<UsigMessage<P, Sig, Att>>))> = Vec::new();

            //get all replicas for which a quorum of f+1 matching counters has been found
            for (replica_id, counts_entry) in self.recv_forwards.iter().enumerate() {
                if let Some(counts) = counts_entry {
                    let mut h: HashMap<Count, u64> = HashMap::new();
                    let mut nv: HashMap<&Count, Vec<UsigMessage<P, Sig, Att>>> = HashMap::new();

                    for (count, new_view) in counts.iter().flatten() {
                        h.insert(*count, h.get(count).unwrap_or(&0) + 1);
                        nv.insert(count, new_view.clone()); //TODO also check that the new view message matches
                    }

                    for (counter, count) in h.iter() {
                        if *count > config.t {
                            vec.push((
                                ReplicaId::from_u64(replica_id as u64),
                                (*counter, nv.remove(counter).unwrap()),
                            ));
                        }
                    }
                }
            }

            //remove collected counters from collector, so they don't get collected again
            for (replica_id, _) in &vec {
                self.recv_forwards[replica_id.as_u64() as usize] = None;
            }

            trace!(
                "collected recovered counters from forward message collector: {:?}",
                vec
            );
            vec
        }
    }
}
