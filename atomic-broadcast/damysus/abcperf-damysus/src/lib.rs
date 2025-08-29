use std::fmt::Debug;
use std::iter;
use std::num::NonZeroU64;
use std::time::Duration;
use std::time::Instant;

use abcperf::atomic_broadcast::AtomicBroadcastInfo;
use abcperf::config::AtomicBroadcastConfiguration;
use abcperf::MessageDestination;
use abcperf::MessageType;
use abcperf_abcwrapper::ABCperfWrapper;
use abcperf_abcwrapper::AbcOutput;
use abcperf_abcwrapper::AbcWrapper;
use abcperf_abcwrapper::StopClass;
use abcperf_abcwrapper::TimeoutChanage;
use damysus::Output;
use damysus::Timeout;
use damysus_base::enclave::Enclave;
use either::Either;
use serde::{Deserialize, Serialize};

use shared_ids::hashbar::Hashbar;
use shared_ids::ClientId;
use shared_ids::ReplicaId;
use shared_ids::RequestId;

use damysus::Damysus;
use damysus::PeerMessage;

use tracing::error;
use transaction_trait::Transaction;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConfigurationExtension {
    initial_view_timeout: f64,
    block_timeout: f64,
    minimum_block_size: u64,
    maximum_block_size: Option<NonZeroU64>,
    ignore_timeouts_for: Option<u64>,
}

pub type ABCperfDamysus<Tx, Enc> = ABCperfWrapper<DamysusABCWrapper<Tx, Enc>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamysusTimeoutType {
    Block,
    View,
}

pub struct DamysusOutputWrapper<Tx, Enc>(Output<MyTransaction<Tx>, Enc>)
where
    Enc: Enclave;

impl<Tx, Enc> AbcOutput for DamysusOutputWrapper<Tx, Enc>
where
    Enc: Enclave,
    Tx: Send,
    Enc::Signature: Send,
    Enc::PublicKey: Send,
{
    type TimeoutType = DamysusTimeoutType;
    type PeerMessage = PeerMessage<MyTransaction<Tx>, Enc::Signature, Enc::PublicKey>;
    type ClientRequest = Tx;

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn sets_ready(&self) -> bool {
        self.0.ready
    }

    #[allow(clippy::type_complexity)]
    fn destructure(
        self,
        ready: bool,
    ) -> (
        impl Iterator<Item = (Self::TimeoutType, TimeoutChanage)> + Send,
        impl Iterator<Item = (MessageDestination, Self::PeerMessage)> + Send,
        impl Iterator<Item = (ClientId, RequestId, Self::ClientRequest)> + Send,
        AtomicBroadcastInfo,
        Vec<Instant>,
    ) {
        let Output {
            broadcasts,
            unicasts,
            view_timeout,
            block_timeout,
            delivery,
            ready: _,
            view,
            decision_times,
        } = self.0;

        let info = AtomicBroadcastInfo {
            ready,
            state: format!("View {}", view),
            leader: None,
        };

        let iter_view_timeout = match view_timeout {
            Timeout::Set(duration) => Either::Right(iter::once((
                DamysusTimeoutType::View,
                TimeoutChanage::Set(duration, StopClass(0)),
            ))),
            Timeout::Cancel => Either::Right(iter::once((
                DamysusTimeoutType::View,
                TimeoutChanage::RemoveAll,
            ))),
            Timeout::None => Either::Left(iter::empty()),
        };

        let iter_block_timeout = match block_timeout {
            Timeout::Set(duration) => Either::Right(iter::once((
                DamysusTimeoutType::Block,
                TimeoutChanage::Set(duration, StopClass(0)),
            ))),
            Timeout::Cancel => Either::Right(iter::once((
                DamysusTimeoutType::Block,
                TimeoutChanage::RemoveAll,
            ))),
            Timeout::None => Either::Left(iter::empty()),
        };

        let timeout = iter_view_timeout.chain(iter_block_timeout);

        let messages = broadcasts
            .into_iter()
            .map(|m| (MessageDestination::Broadcast, m))
            .chain(
                unicasts
                    .into_iter()
                    .map(|(d, m)| (MessageDestination::Unicast(d), m)),
            );

        let delivery = delivery.into_iter().map(Into::into);

        (timeout, messages, delivery, info, decision_times)
    }

    fn do_backup(&self) -> bool {
        false // Damysus does not support backup
    }

    type GenAtomicBroadcastInfo = AtomicBroadcastInfo;
}

pub struct DamysusABCWrapper<Tx: Clone + Debug, Enc: Enclave> {
    damysus: Damysus<MyTransaction<Tx>, Enc>,
}

impl<
        Tx: 'static
            + Debug
            + for<'a> Deserialize<'a>
            + Serialize
            + Send
            + Sync
            + Clone
            + Hashbar
            + Unpin,
        Enc: Enclave,
    > AbcWrapper for DamysusABCWrapper<Tx, Enc>
where
    Enc: Enclave + 'static + Debug + Send + Sync,
    Enc::Signature: Unpin + Send + Debug + for<'a> Deserialize<'a> + Serialize + Sync,
    Enc::PublicKey: Unpin + Send + Debug + for<'a> Deserialize<'a> + Serialize + Sync,
{
    type TimeoutType = DamysusTimeoutType;

    type PeerMessage = PeerMessage<MyTransaction<Tx>, Enc::Signature, Enc::PublicKey>;

    type ClientRequest = Tx;

    type Output = DamysusOutputWrapper<Tx, Enc>;

    type Config = ConfigurationExtension;

    fn ignore_timeouts_for(config: &AtomicBroadcastConfiguration<Self::Config>) -> Option<u64> {
        config.extension.ignore_timeouts_for
    }

    fn new(config: AtomicBroadcastConfiguration<Self::Config>) -> Self {
        let damysus = Damysus::new(
            config.replica_id,
            config.n.get(),
            config.extension.minimum_block_size,
            config
                .extension
                .maximum_block_size
                .map(NonZeroU64::get)
                .unwrap_or(u64::MAX),
            Duration::from_secs_f64(config.extension.block_timeout),
            Duration::from_secs_f64(config.extension.initial_view_timeout),
        );
        Self { damysus }
    }

    fn init(&mut self) -> Self::Output {
        DamysusOutputWrapper(self.damysus.init_handshake())
    }

    fn handle_timeout(&mut self, typ: Self::TimeoutType) -> Self::Output {
        DamysusOutputWrapper(match typ {
            DamysusTimeoutType::Block => self.damysus.handle_block_timeout(),
            DamysusTimeoutType::View => self.damysus.handle_view_timeout(),
        })
    }

    fn handle_peer_message(
        &mut self,
        _typ: MessageType,
        _from: ReplicaId,
        msg: Self::PeerMessage,
    ) -> Self::Output {
        DamysusOutputWrapper(self.damysus.process_peer_message(msg))
    }

    fn handle_client_request(
        &mut self,
        req: (ClientId, RequestId, Self::ClientRequest),
    ) -> Self::Output {
        DamysusOutputWrapper(self.damysus.receive_client_request(req.into()))
    }

    type Backup = ();

    fn backup(&mut self) -> Self::Backup {
        error!("damysus does not support backup");
    }

    fn restore(&mut self, (): Self::Backup) -> Option<Self::Output> {
        error!("damysus does not support restore");
        None
    }
}
