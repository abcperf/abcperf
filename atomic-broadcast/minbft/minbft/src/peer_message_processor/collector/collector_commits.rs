//! Defines the collector of messages of type Commit.
//! After a sufficient amount (`t + 1`) of Commits are received and collected,
//! the respective batch of client-requests is accepted.

use crate::{
    peer_message::usig_message::{UsigMessage, UsigMessageV},
    Config, Prepare,
};
use std::collections::{BTreeMap, HashMap, HashSet};

use tracing::trace;
use usig::{Count, Counter, ReplicaId};

use crate::peer_message::usig_message::view_peer_message::ViewPeerMessage;

/// Collects received Commits.
#[derive(Debug, Clone)]
pub(crate) struct CollectorCommits<P, Sig, Att> {
    /// For each Prepare received, the Commits with respect to the Prepare
    /// are collected.
    /// The receival of the [crate::Prepare] may be either indirect (through a
    /// Commit) or direct (actual [crate::Prepare] broadcast by primary).
    recv_commits: HashMap<Count, HashSet<ReplicaId>>,
    recv_commit_msgs: HashMap<Count, Vec<UsigMessage<P, Sig, Att>>>,
    /// The Prepares are ordered by their counter.
    prepare: BTreeMap<Count, Prepare<P, Sig, Att>>,
}

/// Defines the key for the collector.
/// The key must be the counter of the [crate::Prepare] to which the Commits
/// belong to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(dead_code)]
struct KeyCommits(Count);

impl<P: Clone, Sig: Counter + Clone, Att: Clone> CollectorCommits<P, Sig, Att> {
    /// Creates a new collector of Commits.
    pub(crate) fn new() -> CollectorCommits<P, Sig, Att> {
        CollectorCommits {
            recv_commits: HashMap::new(),
            recv_commit_msgs: HashMap::new(),
            prepare: BTreeMap::new(),
        }
    }
    /// Collects a [ViewPeerMessage] (Prepare or Commit).
    ///
    /// # Arguments
    ///
    /// * `msg` - The message to be collected (Prepare or Commit).
    /// * `config` - The configuration of the replica.
    ///
    /// # Return Value
    ///
    /// Returns the Prepare(s) that with the collection of the provided Commit
    /// can be regarded as accepted.
    #[allow(clippy::type_complexity)]
    pub(crate) fn collect(
        &mut self,
        msg: ViewPeerMessage<P, Sig, Att>,
        config: &Config,
    ) -> Vec<(Prepare<P, Sig, Att>, Vec<UsigMessage<P, Sig, Att>>)> {
        match msg {
            ViewPeerMessage::Prepare(prepare) => {
                trace!(
                    "Collecting Prepare (origin: {:?}, view: {:?}, counter: {:?}) ...",
                    prepare.origin,
                    prepare.view,
                    prepare.counter(),
                );

                let counter = prepare.counter();

                match self.recv_commits.get_mut(&counter) {
                    Some(collected_commit_origins) => {
                        collected_commit_origins.insert(prepare.origin);
                        self.recv_commit_msgs
                            .get_mut(&counter)
                            .unwrap()
                            .push(UsigMessageV::View(ViewPeerMessage::Prepare(
                                prepare.clone(),
                            )));
                    }
                    None => {
                        let mut collected_commit_origins = HashSet::new();
                        collected_commit_origins.insert(prepare.origin);
                        self.recv_commits.insert(counter, collected_commit_origins);

                        let collected_commit_msgs = vec![UsigMessageV::View(
                            ViewPeerMessage::Prepare(prepare.clone()),
                        )];
                        self.recv_commit_msgs.insert(counter, collected_commit_msgs);
                    }
                }
                self.prepare.insert(counter, prepare);
            }
            ViewPeerMessage::Commit(commit) => {
                trace!(
                    "Collecting Commit (origin: {:?}, counter: {:?}, Prepare: [origin: {:?}, view: {:?}, counter: {:?}]) ...",
                    commit.origin,
                    commit.counter(),
                    commit.prepare.origin,
                    commit.prepare.view,
                    commit.prepare.counter(),
                );

                let counter = commit.prepare.counter();

                match self.recv_commits.get_mut(&counter) {
                    Some(collected_commit_origins) => {
                        collected_commit_origins.insert(commit.origin);
                        self.recv_commit_msgs
                            .get_mut(&counter)
                            .unwrap()
                            .push(UsigMessageV::View(ViewPeerMessage::Commit(commit.clone())));
                    }
                    None => {
                        let mut collected_commit_origins = HashSet::new();
                        collected_commit_origins.insert(commit.origin);
                        self.recv_commits.insert(counter, collected_commit_origins);

                        let collected_commit_msgs =
                            vec![UsigMessageV::View(ViewPeerMessage::Commit(commit.clone()))];
                        self.recv_commit_msgs.insert(counter, collected_commit_msgs);
                    }
                }

                self.prepare.insert(counter, commit.prepare.clone());
            }
        }

        let mut vec = Vec::new();

        while let Some(entry) = self.prepare.first_entry() {
            let commits = self.recv_commits.get_mut(entry.key()).unwrap();

            if commits.len() <= config.t.try_into().unwrap() {
                break;
            }

            self.recv_commits.remove_entry(entry.key());
            let commit_msgs = self.recv_commit_msgs.remove(entry.key()).unwrap();
            vec.push((entry.remove(), commit_msgs));
        }
        vec
    }
}
/*
#[cfg(test)]
mod test {
    use rstest::rstest;
    use usig::Counter;

    use std::num::NonZeroU64;

    use rand::thread_rng;

    use crate::{
        client_request::test::create_batch,
        peer_message::usig_message::view_peer_message::{
            commit::test::create_commit, prepare::test::create_prepare, ViewPeerMessage,
        },
        peer_message_processor::collector::collector_commits::CollectorCommits,
        tests::{
            create_attested_usigs_for_replicas, create_default_configs_for_replicas,
            get_random_included_replica_id, get_random_replica_id, get_shuffled_remaining_replicas,
        },
        View,
    };

    /// Test if the collection of a single Commit succeeds.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of replicas.
    #[rstest]
    fn collect_commit_single(#[values(3, 4, 5, 6, 7, 8, 9, 10)] n: u64) {
        let n_parsed = NonZeroU64::new(n).unwrap();
        let mut rng = thread_rng();
        let t = n / 2;

        let primary_id = get_random_replica_id(n_parsed, &mut rng);
        let view = View(primary_id.as_u64());
        let request_batch = create_batch();

        let configs = create_default_configs_for_replicas(n_parsed, t);
        let mut usigs = create_attested_usigs_for_replicas(n_parsed, Vec::new());

        let config_primary = configs.get(&primary_id).unwrap();
        let usig_primary = usigs.get_mut(&primary_id).unwrap();

        let prepare = create_prepare(view, request_batch, config_primary, usig_primary);

        let backup_id = get_random_included_replica_id(n_parsed, primary_id, &mut rng);
        let usig_backup = usigs.get_mut(&backup_id).unwrap();

        let commit = create_commit(backup_id, prepare.clone(), usig_backup);

        let vp_msg = ViewPeerMessage::from(commit.clone());

        let mut collector = CollectorCommits::new();

        let acceptable_prepares = collector.collect(vp_msg, config_primary);

        assert!(acceptable_prepares.is_empty());
        assert!(collector
            .recv_commits
            .contains_key(&commit.prepare.counter()));
        let collected_commit_origins = collector
            .recv_commits
            .get(&commit.prepare.counter())
            .unwrap();
        assert!(collected_commit_origins.contains(&commit.origin));
        assert_eq!(collector.prepare.len(), 1);
        assert!(collector.prepare.contains_key(&prepare.counter()));
        let collected_prepare = collector.prepare.get(&prepare.counter()).unwrap();
        assert_eq!(collected_prepare.counter(), prepare.counter());
        assert_eq!(collected_prepare.origin, prepare.origin);
        assert_eq!(collected_prepare.view, prepare.view);
        assert_eq!(collected_prepare.request_batch, prepare.request_batch);
    }

    /// Test if the collection of sufficient Commits results in the retrieval
    /// of the correct Prepare.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of replicas.
    #[rstest]
    fn collect_sufficient_commits_single_prep(#[values(3, 4, 5, 6, 7, 8, 9, 10)] n: u64) {
        let n_parsed = NonZeroU64::new(n).unwrap();
        let mut rng = thread_rng();
        let t = n / 2;

        let primary_id = get_random_replica_id(n_parsed, &mut rng);
        let view = View(primary_id.as_u64());
        let request_batch = create_batch();

        let configs = create_default_configs_for_replicas(n_parsed, t);
        let mut usigs = create_attested_usigs_for_replicas(n_parsed, Vec::new());

        let usig_primary = usigs.get_mut(&primary_id).unwrap();
        let config_primary = configs.get(&primary_id).unwrap();

        let prepare = create_prepare(view, request_batch, config_primary, usig_primary);

        let shuffled_backup_reps =
            get_shuffled_remaining_replicas(n_parsed, Some(primary_id), &mut rng);

        let shuffled_set = shuffled_backup_reps.iter().take((t).try_into().unwrap());

        let mut collector = CollectorCommits::new();
        let mut acceptable_prepares =
            collector.collect(ViewPeerMessage::Prepare(prepare.clone()), config_primary);
        assert!(acceptable_prepares.is_empty());

        let mut counter_collected_commits = 1;
        for backup_rep_id in shuffled_set {
            let usig_backup = usigs.get_mut(backup_rep_id).unwrap();
            let commit = create_commit(*backup_rep_id, prepare.clone(), usig_backup);

            let vp_msg = ViewPeerMessage::from(commit.clone());

            acceptable_prepares = collector.collect(vp_msg, config_primary);
            counter_collected_commits += 1;

            if counter_collected_commits <= t {
                assert!(acceptable_prepares.is_empty())
            }
        }

        assert_eq!(collector.prepare.len(), 0);
        assert!(!acceptable_prepares.is_empty());
        assert_eq!(acceptable_prepares.len(), 1);
        assert!(acceptable_prepares.contains(&prepare));
    }

    /// Tests if the collection of sufficient Commits (for different Prepares)
    /// results in the retrieval of the two Prepares.
    #[rstest]
    fn collect_sufficient_commits_two_preps(#[values(3, 4, 5, 6, 7, 8, 9, 10)] n: u64) {
        let n_parsed = NonZeroU64::new(n).unwrap();
        let mut rng = thread_rng();
        let t = n / 2;

        let primary_id = get_random_replica_id(n_parsed, &mut rng);
        let view = View(primary_id.as_u64());
        let request_batch = create_batch();

        let configs = create_default_configs_for_replicas(n_parsed, t);
        let mut usigs = create_attested_usigs_for_replicas(n_parsed, Vec::new());

        let usig_primary = usigs.get_mut(&primary_id).unwrap();
        let config_primary = configs.get(&primary_id).unwrap();

        let prepare = create_prepare(view, request_batch, config_primary, usig_primary);

        let shuffled_backup_reps =
            get_shuffled_remaining_replicas(n_parsed, Some(primary_id), &mut rng);

        let shuffled_set = shuffled_backup_reps.iter().take((t).try_into().unwrap());

        let mut collector = CollectorCommits::new();
        let mut acceptable_prepares =
            collector.collect(ViewPeerMessage::Prepare(prepare.clone()), config_primary);
        assert!(acceptable_prepares.is_empty());

        let mut counter_collected_commits = 1;
        for backup_rep_id in shuffled_set {
            let usig_backup = usigs.get_mut(backup_rep_id).unwrap();
            let commit = create_commit(*backup_rep_id, prepare.clone(), usig_backup);

            let vp_msg = ViewPeerMessage::from(commit.clone());

            acceptable_prepares = collector.collect(vp_msg, config_primary);
            counter_collected_commits += 1;

            if counter_collected_commits <= t {
                assert!(acceptable_prepares.is_empty())
            }
        }

        assert!(collector.prepare.is_empty());
        assert!(!acceptable_prepares.is_empty());
        assert_eq!(acceptable_prepares.len(), 1);
        assert!(acceptable_prepares.contains(&prepare));

        // Creation and collection of second prepare and respective commits.
        let second_primary_id = get_random_included_replica_id(n_parsed, primary_id, &mut rng);
        let second_view = View(second_primary_id.as_u64());
        let second_request_batch = create_batch();

        let second_config_primary = configs.get(&second_primary_id).unwrap();
        let second_usig_primary = usigs.get_mut(&second_primary_id).unwrap();

        let second_prepare = create_prepare(
            second_view,
            second_request_batch,
            second_config_primary,
            second_usig_primary,
        );

        let second_shuffled_backup_reps =
            get_shuffled_remaining_replicas(n_parsed, Some(second_primary_id), &mut rng);

        let second_shuffled_set = second_shuffled_backup_reps
            .iter()
            .take((t).try_into().unwrap());

        let mut acceptable_prepares = collector.collect(
            ViewPeerMessage::Prepare(second_prepare.clone()),
            second_config_primary,
        );
        assert!(acceptable_prepares.is_empty());

        let mut counter_collected_commits = 1;
        for backup_rep_id in second_shuffled_set {
            let usig_backup = usigs.get_mut(backup_rep_id).unwrap();
            let commit = create_commit(*backup_rep_id, second_prepare.clone(), usig_backup);

            let vp_msg = ViewPeerMessage::from(commit.clone());

            acceptable_prepares = collector.collect(vp_msg, second_config_primary);
            counter_collected_commits += 1;

            if counter_collected_commits <= t {
                assert!(acceptable_prepares.is_empty())
            }
        }

        assert!(collector.prepare.is_empty());
        assert!(!acceptable_prepares.is_empty());
        assert_eq!(acceptable_prepares.len(), 1);
        assert!(acceptable_prepares.contains(&second_prepare));
    }
}
*/
