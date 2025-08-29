use abcperf::atomic_broadcast::AtomicBroadcastInfo;
use abcperf::config::AtomicBroadcastConfiguration;
use abcperf::MessageDestination;
use abcperf_abcwrapper::ABCperfWrapper;
use abcperf_abcwrapper::AbcOutput;
use abcperf_abcwrapper::AbcWrapper;
use abcperf_abcwrapper::GenAtomicBroadcastInfo;
use abcperf_abcwrapper::StopClass;
use abcperf_abcwrapper::TimeoutChanage;
use either::Either;
use hashbar::Hashbar;
use hashbar::Hasher;
use nxbft::output::Broadcasts;
use nxbft::output::Consensus;
use nxbft::output::Output;
use nxbft::output::StateMessage;
use nxbft::output::Unicasts;
use nxbft::output::VertexTimeout;
use nxbft::Backup;
use nxbft::Round;
use nxbft::Wave;
use serde::{Deserialize, Serialize};
use shared_ids::RequestId;
use std::fmt::Debug;
use std::hash::Hash;
use std::iter;
use std::num::NonZeroU64;
use std::time::Duration;
use std::time::Instant;
use tracing::debug;
use tracing::error;
use tracing::warn;

use nxbft::Enclave;
use nxbft::NxBft;
use nxbft::PeerMessage;
use transaction_trait::Transaction;

use shared_ids::ClientId;

pub type ABCperfNxbft<Tx, Enc> = ABCperfWrapper<NxbftABCWrapper<Tx, Enc>>;

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct MyTransaction<Tx> {
    client_id: ClientId,
    request_id: RequestId,
    transaction: Tx,
}

impl<Tx> From<(ClientId, RequestId, Tx)> for MyTransaction<Tx> {
    fn from((client_id, request_id, transaction): (ClientId, RequestId, Tx)) -> Self {
        Self {
            client_id,
            request_id,
            transaction,
        }
    }
}

impl<Tx> From<MyTransaction<Tx>> for (ClientId, RequestId, Tx) {
    fn from(
        MyTransaction {
            client_id,
            request_id,
            transaction,
        }: MyTransaction<Tx>,
    ) -> Self {
        (client_id, request_id, transaction)
    }
}

impl<Tx> Transaction for MyTransaction<Tx> {
    fn client_id(&self) -> ClientId {
        self.client_id
    }

    fn request_id(&self) -> RequestId {
        self.request_id
    }
}

impl<Tx: Hashbar> Hashbar for MyTransaction<Tx> {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.client_id.as_u64().to_le_bytes());
        hasher.update(&self.request_id.as_u64().to_le_bytes());
        self.transaction.hash(hasher);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConfigurationExtension {
    vertex_timeout: f64,
    min_vertex_size: u64,
    max_vertex_size: Option<NonZeroU64>,
}

pub struct NxbftOutputWrapper<Tx, Enc>
where
    Tx: Hashbar,
    Enc: Enclave,
{
    broadcasts: Broadcasts<MyTransaction<Tx>, Enc>,
    unicasts: Unicasts<MyTransaction<Tx>, Enc>,
    delivery: Vec<MyTransaction<Tx>>,
    backup_enclave: bool,
    decision_times: Vec<Instant>,
    empty: bool,
    ready: bool,
    round: Option<Round>,
    wave: Option<Wave>,
    vertex_timeout: VertexTimeout,
}

impl<Tx, Enc> NxbftOutputWrapper<Tx, Enc>
where
    Tx: Hashbar,
    Enc: Enclave,
{
    fn new(output: Output<MyTransaction<Tx>, Enc>) -> Self {
        let empty = output.is_empty();
        let mut ready = false;
        let mut round = None;
        let mut wave = None;
        let Output {
            broadcasts,
            unicasts,
            state_messages,
            delivery,
            backup_enclave,
            decision_times,
            vertex_timeout,
        } = output;
        for msg in state_messages {
            match msg {
                StateMessage::Error(e) => {
                    error!("ERROR: {:?}", e);
                }
                StateMessage::PeerError(peer_error) => warn!("Peer Error: {:?}", peer_error),

                StateMessage::Consensus(consensus) => {
                    match &consensus {
                        Consensus::Decided {
                            waves,
                            leaders: _,
                            order: _,
                        } => {
                            wave = Some(*waves.last().expect("always at least one element"));
                        }
                        Consensus::RoundTransition { new_round } => {
                            round = Some(*new_round);
                        }
                        Consensus::CatchUp { delayed } => {
                            warn!("Catch-up: delayed by {} rounds", delayed);
                            continue;
                        }
                        _ => {}
                    }
                    debug!("Consensus: {:?}", consensus);
                }

                StateMessage::Broadcast(broadcast) => {
                    debug!("{:?}", broadcast);
                }

                StateMessage::Ready => {
                    ready = true;
                }
                StateMessage::Verbose(_) => continue,
            }
        }
        Self {
            broadcasts,
            unicasts,
            delivery,
            backup_enclave,
            decision_times,
            empty,
            ready,
            round,
            wave,
            vertex_timeout,
        }
    }
}

pub struct NxbftAtomicBroadcastInfoGen {
    round: Option<Round>,
    wave: Option<Wave>,
    ready: bool,
}

impl GenAtomicBroadcastInfo for NxbftAtomicBroadcastInfoGen {
    fn generate_info(&mut self, prev: Option<&Self>) -> Option<AtomicBroadcastInfo> {
        if let Some(prev) = prev {
            self.round = self.round.or(prev.round);
            self.wave = self.wave.or(prev.wave);
            if prev.round == self.round && prev.wave == self.wave && prev.ready == self.ready {
                return None;
            }
        }

        Some(AtomicBroadcastInfo {
            ready: self.ready,
            state: format!(
                "round {}, wave {}",
                self.round.unwrap_or(1.into()).as_u64(),
                self.wave.unwrap_or(1.into()).as_u64()
            ),
            leader: None,
        })
    }
}

impl<Tx, Enc> AbcOutput for NxbftOutputWrapper<Tx, Enc>
where
    Tx: Hashbar + Send,
    Enc: Enclave,
    Enc::Attestation: Send,
    Enc::Handshake: Send,
    Enc::Signature: Send,
{
    type TimeoutType = NxbftTimeoutType;

    type PeerMessage =
        PeerMessage<MyTransaction<Tx>, Enc::Attestation, Enc::Handshake, Enc::Signature>;

    type ClientRequest = Tx;

    fn is_empty(&self) -> bool {
        self.empty
    }

    fn sets_ready(&self) -> bool {
        self.ready
    }

    fn do_backup(&self) -> bool {
        self.backup_enclave
    }

    fn destructure(
        self,
        ready: bool,
    ) -> (
        impl Iterator<Item = (Self::TimeoutType, TimeoutChanage)> + Send,
        impl Iterator<Item = (MessageDestination, Self::PeerMessage)> + Send,
        impl Iterator<Item = (ClientId, RequestId, Self::ClientRequest)> + Send,
        NxbftAtomicBroadcastInfoGen,
        Vec<Instant>,
    ) {
        let info = NxbftAtomicBroadcastInfoGen {
            round: self.round,
            wave: self.wave,
            ready,
        };

        let timeout = match self.vertex_timeout {
            VertexTimeout::Set(duration) => Either::Right(iter::once((
                NxbftTimeoutType::Vertex,
                TimeoutChanage::Set(duration, StopClass(0)),
            ))),
            VertexTimeout::Cancel => Either::Right(iter::once((
                NxbftTimeoutType::Vertex,
                TimeoutChanage::RemoveAll,
            ))),
            VertexTimeout::None => Either::Left(iter::empty()),
        };

        let messages = self
            .broadcasts
            .into_iter()
            .map(|m| (MessageDestination::Broadcast, m))
            .chain(
                self.unicasts
                    .into_iter()
                    .map(|(d, m)| (MessageDestination::Unicast(d), m)),
            );

        let delivery = self.delivery.into_iter().map(Into::into);

        (timeout, messages, delivery, info, self.decision_times)
    }

    type GenAtomicBroadcastInfo = NxbftAtomicBroadcastInfoGen;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NxbftTimeoutType {
    Vertex,
}

pub struct NxbftABCWrapper<Tx, Enc>
where
    Tx: 'static
        + Debug
        + for<'a> Deserialize<'a>
        + Serialize
        + Send
        + Unpin
        + Sync
        + Clone
        + Hashbar
        + std::hash::Hash
        + Eq,
    Enc: Enclave + 'static + Debug + Send,
    Enc::Attestation: Unpin + Send + Debug + for<'a> Deserialize<'a> + Serialize + Sync,
    Enc::Handshake: Unpin + Send + Debug + for<'a> Deserialize<'a> + Serialize + Sync,
    Enc::Signature: Unpin + Send + Debug + for<'a> Deserialize<'a> + Serialize + Sync,
{
    nxbft: NxBft<MyTransaction<Tx>, Enc>,
}

impl<Tx, Enc> AbcWrapper for NxbftABCWrapper<Tx, Enc>
where
    Tx: 'static
        + Debug
        + for<'a> Deserialize<'a>
        + Serialize
        + Send
        + Unpin
        + Sync
        + Clone
        + Hashbar
        + std::hash::Hash
        + Eq,
    Enc: Enclave + 'static + Debug + Send + Sync,
    Enc::Attestation: Unpin + Send + Debug + for<'a> Deserialize<'a> + Serialize + Sync,
    Enc::Handshake: Unpin + Send + Debug + for<'a> Deserialize<'a> + Serialize + Sync,
    Enc::Signature: Unpin + Send + Debug + for<'a> Deserialize<'a> + Serialize + Sync,
    Enc::EnclaveError: Sync,
{
    type TimeoutType = NxbftTimeoutType;

    type PeerMessage =
        PeerMessage<MyTransaction<Tx>, Enc::Attestation, Enc::Handshake, Enc::Signature>;

    type ClientRequest = Tx;

    type Output = NxbftOutputWrapper<Tx, Enc>;

    type Config = ConfigurationExtension;

    fn ignore_timeouts_for(_config: &AtomicBroadcastConfiguration<Self::Config>) -> Option<u64> {
        None
    }

    fn new(config: AtomicBroadcastConfiguration<Self::Config>) -> Self {
        let nxbft = NxBft::new(
            config.replica_id,
            config.n.get(),
            Duration::from_secs_f64(config.extension.vertex_timeout),
            config.extension.min_vertex_size,
            config
                .extension
                .max_vertex_size
                .map(NonZeroU64::get)
                .unwrap_or(u64::MAX),
        );
        Self { nxbft }
    }

    fn init(&mut self) -> Self::Output {
        NxbftOutputWrapper::new(self.nxbft.init_handshake())
    }

    fn handle_timeout(&mut self, typ: Self::TimeoutType) -> Self::Output {
        NxbftOutputWrapper::new(match typ {
            NxbftTimeoutType::Vertex => self.nxbft.process_vertex_timeout(),
        })
    }

    fn handle_peer_message(
        &mut self,
        _typ: abcperf::MessageType,
        _from: shared_ids::ReplicaId,
        msg: Self::PeerMessage,
    ) -> Self::Output {
        NxbftOutputWrapper::new(self.nxbft.process_peer_message(msg))
    }

    fn handle_client_request(
        &mut self,
        req: (ClientId, RequestId, Self::ClientRequest),
    ) -> Self::Output {
        NxbftOutputWrapper::new(self.nxbft.receive_client_request(req.into()))
    }

    type Backup = Backup<MyTransaction<Tx>, Enc>;

    fn backup(&mut self) -> Self::Backup {
        self.nxbft.backup_enclave().unwrap()
    }

    fn restore(&mut self, backup: Self::Backup) -> Option<Self::Output> {
        let (nxbft, output) = NxBft::init_recovery(backup).unwrap();
        self.nxbft = nxbft;
        Some(NxbftOutputWrapper::new(output))
    }
}
