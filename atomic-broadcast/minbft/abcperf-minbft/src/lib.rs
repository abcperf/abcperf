use std::{
    fmt::Debug,
    num::{NonZeroU64, NonZeroUsize},
    time::{Duration, Instant},
};

use abcperf::{
    atomic_broadcast::AtomicBroadcastInfo, config::AtomicBroadcastConfiguration,
    MessageDestination, MessageType,
};
use abcperf_abcwrapper::{ABCperfWrapper, AbcOutput, AbcWrapper, StopClass, TimeoutChanage};
use minbft::{
    config::BatchConfiguration,
    output::{TimeoutRequest, ViewInfo},
    timeout::TimeoutType,
    Config as MinBftConfig, Config, MinBft, Output, PeerMessage, WrappedRequestPayload,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use shared_ids::{hashbar::Hashbar, ClientId, ReplicaId, RequestId};
use tracing::warn;
use usig::Usig;

pub type ABCperfMinbft<P, U> = ABCperfWrapper<MinbftABCWrapper<P, U>>;

pub struct MinbftOutputWrapper<P, U: usig::Usig>(Output<P, U>);

impl<P, U: usig::Usig> AbcOutput for MinbftOutputWrapper<P, U>
where
    P: Clone + Serialize + DeserializeOwned + Debug + Hashbar + Send + 'static + Sync,
    U::Attestation: Clone + DeserializeOwned,
    U::Signature: Clone + Serialize + Send,
{
    type TimeoutType = TimeoutType;

    type PeerMessage = PeerMessage<U::Attestation, P, U::Signature>;

    type ClientRequest = P;

    type GenAtomicBroadcastInfo = AtomicBroadcastInfo;

    fn is_empty(&self) -> bool {
        false
    }

    fn sets_ready(&self) -> bool {
        self.0.ready_for_client_requests
    }

    fn do_backup(&self) -> bool {
        false
    }

    fn destructure(
        self,
        _ready: bool,
    ) -> (
        impl Iterator<Item = (Self::TimeoutType, TimeoutChanage)> + Send,
        impl Iterator<Item = (MessageDestination, Self::PeerMessage)> + Send,
        impl Iterator<Item = (ClientId, RequestId, Self::ClientRequest)> + Send,
        Self::GenAtomicBroadcastInfo,
        Vec<Instant>,
    ) {
        let Output {
            broadcasts,
            direct_messages,
            responses,
            errors,
            ready_for_client_requests: _,
            primary,
            view_info,
            round,
            round_times,
            timeout_requests,
        } = self.0;

        for error in errors.iter() {
            warn!("error during processing: {}", error);
        }

        let timeout_requests = Vec::from(timeout_requests).into_iter().map(|x| match x {
            TimeoutRequest::Start(timeout) => (
                timeout.timeout_type,
                TimeoutChanage::Set(timeout.duration, StopClass(timeout.stop_class_as_u64())),
            ),
            TimeoutRequest::Stop(timeout) => (
                timeout.timeout_type,
                TimeoutChanage::Remove(StopClass(timeout.stop_class_as_u64())),
            ),
            TimeoutRequest::StopAny(timeout_any) => {
                (timeout_any.timeout_type, TimeoutChanage::RemoveAll)
            }
        });

        let messages = Vec::from(broadcasts)
            .into_iter()
            .map(|m| (MessageDestination::Broadcast, m))
            .chain(
                Vec::from(direct_messages)
                    .into_iter()
                    .map(|(d, m)| (MessageDestination::Unicast(d), m)),
            );

        let new_info = AtomicBroadcastInfo {
            ready: matches!(view_info, ViewInfo::InView(_)),
            state: match view_info {
                ViewInfo::InView(view) => format!("round: {round}, in view {view}"),
                ViewInfo::ViewChange { from, to } => {
                    format!("round: {round}, view change from {from} to {to}")
                }
            },
            leader: primary,
        };

        let delivery = Vec::from(responses)
            .into_iter()
            .flat_map(|(client_id, payload)| {
                if let WrappedRequestPayload::Client(request_id, payload) = payload {
                    Some((client_id, request_id, payload))
                } else {
                    None
                }
            });

        (timeout_requests, messages, delivery, new_info, round_times)
    }
}

pub struct MinbftABCWrapper<P, U: Usig>
where
    P: Clone + Serialize + DeserializeOwned + Debug + Hashbar + Send + 'static + Sync,
    U::Attestation: Clone + DeserializeOwned,
    U::Signature: Clone + Serialize,
{
    minbft: MinBft<P, U>,
    initial_output: Option<MinbftOutputWrapper<P, U>>,
    config: Config,
}

impl<P, U> AbcWrapper for MinbftABCWrapper<P, U>
where
    U: Usig + 'static + Sync + Send,
    P: Clone + Serialize + DeserializeOwned + Debug + Hashbar + Send + Sync + 'static,
    U::Attestation: Clone + Serialize + DeserializeOwned + PartialEq + Hashbar + Sync,
    U::Signature: Clone + Serialize + DeserializeOwned + Send + Hashbar + Sync,
{
    type TimeoutType = TimeoutType;

    type PeerMessage = PeerMessage<U::Attestation, P, U::Signature>;

    type ClientRequest = P;

    type Output = MinbftOutputWrapper<P, U>;

    type Config = ConfigurationExtension;

    type Backup = ();

    fn ignore_timeouts_for(config: &AtomicBroadcastConfiguration<Self::Config>) -> Option<u64> {
        config.extension.ignore_timeouts_for
    }

    fn new(config: AtomicBroadcastConfiguration<Self::Config>) -> Self {
        let batcher_configuration = match config.extension.batching {
            BatchConfigurationExt::Trivial {
                batch_timeout,
                max_batch_size,
            } => {
                let batch_timeout = Duration::from_secs_f64(batch_timeout);
                BatchConfiguration::Trivial {
                    batch_timeout,
                    max_batch_size,
                }
            }
            BatchConfigurationExt::AESC {
                max_batch_size,
                max_inflight,
            } => BatchConfiguration::AESC {
                max_batch_size,
                max_inflight,
            },
            BatchConfigurationExt::EMA {
                commit_decay,
                arrival_decay,
            } => BatchConfiguration::EMA {
                commit_decay,
                arrival_decay,
            },
        };

        let config = MinBftConfig {
            id: config.replica_id,
            n: config.n,
            t: config.t,
            batcher_configuration,
            initial_timeout_duration: Duration::from_secs_f64(
                config.extension.initial_timeout_duration,
            ),
            checkpoint_period: config.extension.checkpoint_period,
            backoff_multiplier: config.extension.backoff_multiplier,
        };
        let (minbft, initial_output) = MinBft::new(U::new(), config.clone(), false).unwrap();
        let initial_output = Some(MinbftOutputWrapper(initial_output));
        Self {
            minbft,
            initial_output,
            config,
        }
    }

    fn init(&mut self) -> Self::Output {
        self.initial_output
            .take()
            .expect("initial output should only be called once")
    }

    fn handle_timeout(&mut self, typ: Self::TimeoutType) -> Self::Output {
        MinbftOutputWrapper(self.minbft.handle_timeout(typ))
    }

    fn handle_peer_message(
        &mut self,
        _typ: MessageType,
        from: ReplicaId,
        msg: Self::PeerMessage,
    ) -> Self::Output {
        MinbftOutputWrapper(self.minbft.handle_peer_message(from, msg))
    }

    fn handle_client_request(
        &mut self,
        (client_id, request_id, request): (ClientId, RequestId, Self::ClientRequest),
    ) -> Self::Output {
        MinbftOutputWrapper(
            self.minbft
                .handle_client_message(client_id, request_id, request),
        )
    }

    fn backup(&mut self) -> Self::Backup {}

    fn restore(&mut self, (): Self::Backup) -> Option<Self::Output> {
        let (new_minbft, initial_output) =
            MinBft::new(U::new(), self.config.clone(), true).unwrap();
        self.minbft = new_minbft;
        Some(MinbftOutputWrapper(initial_output))
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConfigurationExtension {
    backoff_multiplier: u8,
    initial_timeout_duration: f64,
    checkpoint_period: NonZeroU64,
    ignore_timeouts_for: Option<u64>,

    batching: BatchConfigurationExt,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum BatchConfigurationExt {
    #[serde(rename = "trivial")]
    Trivial {
        batch_timeout: f64,
        max_batch_size: Option<NonZeroUsize>,
    },
    #[allow(clippy::upper_case_acronyms)]
    #[serde(rename = "aesc")]
    AESC {
        max_batch_size: NonZeroUsize,
        max_inflight: NonZeroUsize,
    },
    #[allow(clippy::upper_case_acronyms)]
    #[serde(rename = "ema")]
    EMA {
        commit_decay: f32,
        arrival_decay: f32,
    },
}

#[cfg(test)]
mod tests {
    //use abcperf::config::NTConfig;

    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct DummyPayload;
    /*
    #[tokio::test]
    async fn exit_replica_first() {
        let abcperf_minbft = ABCperfMinbft::<DummyPayload, _>::new(usig::noop::UsigNoOp::default);

        let (from_replica_send, from_replica_recv) = mpsc::unbounded_channel();
        let (from_client_send, from_client_recv) = mpsc::channel(1000);
        let (to_client_send, _to_client_recv) = mpsc::channel(1000);
        let (update_info_send, _update_info_recv) = mpsc::channel(1000);
        let (_do_recover_send, do_recover_recv) = mpsc::channel(1000);

        let task = abcperf_minbft.start(
            AtomicBroadcastConfiguration {
                replica_id: ReplicaId::from_u64(0),
                extension: ConfigurationExtension {
                    batch_timeout: 1_000_000f64,
                    max_batch_size: None,
                    backoff_multiplier: 1,
                    initial_timeout_duration: 1_000_000f64,
                    checkpoint_period: NonZeroU64::new(2).unwrap(),
                    ignore_timeouts_for: None,
                },
                nt: NTConfig {
                    n: 1.try_into().expect("> 0"),
                    t: 0,
                },
            },
            ABCChannels {
                incoming_replica_messages: from_replica_recv,
                transaction_input: from_client_recv,
                transaction_output: to_client_send,
                update_info: update_info_send,
                do_recover: do_recover_recv,
            },
            || {},
            |_, _| async {},
        );

        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!task.is_finished());

        drop(from_replica_send);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!task.is_finished());

        drop(from_client_send);
        tokio::time::sleep(Duration::from_secs(1)).await;

        assert!(task.is_finished());

        task.await.unwrap();
    }

    #[tokio::test]
    async fn exit_client_first() {
        let abcperf_minbft = ABCperfMinbft::<DummyPayload, _>::new(usig::noop::UsigNoOp::default);

        let (from_replica_send, from_replica_recv) = mpsc::unbounded_channel();
        let (from_client_send, from_client_recv) = mpsc::channel(1000);
        let (to_client_send, _to_client_recv) = mpsc::channel(1000);
        let (update_info_send, _update_info_recv) = mpsc::channel(1000);
        let (_do_recover_send, do_recover_recv) = mpsc::channel(1000);

        let task = abcperf_minbft.start(
            AtomicBroadcastConfiguration {
                replica_id: ReplicaId::from_u64(0),
                extension: ConfigurationExtension {
                    batch_timeout: 1_000_000f64,
                    max_batch_size: None,
                    backoff_multiplier: 1,
                    initial_timeout_duration: 1_000_000f64,
                    checkpoint_period: NonZeroU64::new(2).unwrap(),
                    ignore_timeouts_for: None,
                },
                nt: NTConfig {
                    n: 1.try_into().expect("> 0"),
                    t: 0,
                },
            },
            ABCChannels {
                incoming_replica_messages: from_replica_recv,
                transaction_input: from_client_recv,
                transaction_output: to_client_send,
                update_info: update_info_send,
                do_recover: do_recover_recv,
            },
            || {},
            |_, _| async {},
        );

        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!task.is_finished());

        drop(from_client_send);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!task.is_finished());

        drop(from_replica_send);
        tokio::time::sleep(Duration::from_secs(1)).await;

        assert!(task.is_finished());

        task.await.unwrap();
    }
    */
}
