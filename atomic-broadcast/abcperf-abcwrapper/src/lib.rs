use std::{
    future::Future,
    marker::PhantomData,
    thread,
    time::{Duration, Instant},
};

pub use crate::timeout::StopClass;
use abcperf::{
    atomic_broadcast::{ABCConfig, ABCReplicaMessage, ABCTransaction, AtomicBroadcastInfo},
    config::AtomicBroadcastConfiguration,
    ABCChannels, AtomicBroadcast, MessageDestination, MessageType,
};
use shared_ids::{ClientId, ReplicaId, RequestId};
use tokio::{
    select,
    sync::mpsc::{self, error::TrySendError, Receiver, Sender},
    task::JoinHandle,
};
use tracing::{error, info, warn, Instrument};

use crate::{
    output_handler::OutputHandler,
    timeout::{TimeoutList, TimeoutType},
};

mod output_handler;
mod timeout;

pub struct ABCperfWrapper<T: AbcWrapper> {
    phantom_data: PhantomData<T>,
}

impl<T: AbcWrapper> Default for ABCperfWrapper<T> {
    fn default() -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<T: AbcWrapper> AtomicBroadcast for ABCperfWrapper<T> {
    type Config = T::Config;
    type ReplicaMessage = T::PeerMessage;
    type Transaction = T::ClientRequest;

    fn start<F: Send + Future<Output = ()> + 'static>(
        config: AtomicBroadcastConfiguration<Self::Config>,
        channels: ABCChannels<Self::ReplicaMessage, Self::Transaction>,
        ready_for_clients: impl Send + 'static + FnOnce() + Sync,
        send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> JoinHandle<Vec<Instant>> {
        let ignore_timeouts_for = T::ignore_timeouts_for(&config);
        let damysus = T::new(config);
        let ABCChannels {
            incoming_replica_messages,
            transaction_input: requests,
            transaction_output: responses,
            update_info,
            do_recover,
        } = channels;

        let (output_sender, output_receiver) = mpsc::channel(1000);
        let (command_sender, command_receiver) = mpsc::channel(1000);
        let (timeout_sender, timeout_receiver) = mpsc::channel(1000);
        let (do_backup_sender, do_backup_receiver) = mpsc::channel(1);

        thread::spawn(|| abc_worker(damysus, command_receiver, output_sender));

        let output_handler = OutputHandler::<T, _, _, _>::new(
            output_receiver,
            responses,
            ready_for_clients,
            update_info,
            send_to_replica,
            timeout_sender,
            do_backup_sender,
        );

        let output_handler_join = tokio::spawn(output_handler.run().in_current_span());

        let ignore_timeouts_until =
            Instant::now() + Duration::from_secs(ignore_timeouts_for.unwrap_or(0));

        tokio::spawn(
            async_to_sync_worker(
                incoming_replica_messages,
                requests,
                timeout_receiver,
                ignore_timeouts_until,
                command_sender,
                do_recover,
                do_backup_receiver,
            )
            .in_current_span(),
        );
        output_handler_join
    }
}

enum AbcCommand<A: AbcWrapper> {
    Timeout(A::TimeoutType),
    PeerMessage(MessageType, ReplicaId, A::PeerMessage),
    ClientRequest((ClientId, RequestId, A::ClientRequest)),
    Backup,
    Recover,
}

pub trait AbcWrapper: 'static + Send + Sync {
    type TimeoutType: TimeoutType;
    type PeerMessage: ABCReplicaMessage;
    type ClientRequest: ABCTransaction;
    type Output: AbcOutput<
            TimeoutType = Self::TimeoutType,
            PeerMessage = Self::PeerMessage,
            ClientRequest = Self::ClientRequest,
        > + Send
        + 'static;
    type Config: ABCConfig;
    type Backup: Clone;
    fn ignore_timeouts_for(config: &AtomicBroadcastConfiguration<Self::Config>) -> Option<u64>;
    fn new(config: AtomicBroadcastConfiguration<Self::Config>) -> Self;
    fn init(&mut self) -> Self::Output;
    fn handle_timeout(&mut self, typ: Self::TimeoutType) -> Self::Output;
    fn handle_peer_message(
        &mut self,
        typ: MessageType,
        from: ReplicaId,
        msg: Self::PeerMessage,
    ) -> Self::Output;
    fn handle_client_request(
        &mut self,
        req: (ClientId, RequestId, Self::ClientRequest),
    ) -> Self::Output;
    fn backup(&mut self) -> Self::Backup;
    fn restore(&mut self, backup: Self::Backup) -> Option<Self::Output>;
}

pub trait GenAtomicBroadcastInfo: Send {
    fn generate_info(&mut self, prev: Option<&Self>) -> Option<AtomicBroadcastInfo>;
}

impl GenAtomicBroadcastInfo for AtomicBroadcastInfo {
    fn generate_info(&mut self, prev: Option<&Self>) -> Option<AtomicBroadcastInfo> {
        if Some(&*self) != prev {
            Some(self.clone())
        } else {
            None
        }
    }
}

pub trait AbcOutput {
    type TimeoutType: TimeoutType;
    type PeerMessage;
    type ClientRequest;
    type GenAtomicBroadcastInfo: GenAtomicBroadcastInfo;
    fn is_empty(&self) -> bool;
    fn sets_ready(&self) -> bool;
    fn do_backup(&self) -> bool;

    #[allow(clippy::type_complexity)]
    fn destructure(
        self,
        ready: bool,
    ) -> (
        impl Iterator<Item = (Self::TimeoutType, TimeoutChanage)> + Send,
        impl Iterator<Item = (MessageDestination, Self::PeerMessage)> + Send,
        impl Iterator<Item = (ClientId, RequestId, Self::ClientRequest)> + Send,
        Self::GenAtomicBroadcastInfo,
        Vec<Instant>,
    );
}

fn abc_worker<A: AbcWrapper>(
    mut abc: A,
    mut command_receiver: Receiver<AbcCommand<A>>,
    output_sender: Sender<A::Output>,
) {
    info!("worker OS thread started");

    let output = abc.init();
    if let Err(e) = output_sender.blocking_send(output) {
        error!("Cannot send output to output handler: {:?}", e);
        return;
    }

    let mut backup = None;

    while let Some(cmd) = command_receiver.blocking_recv() {
        let output = match cmd {
            AbcCommand::ClientRequest(transaction) => abc.handle_client_request(transaction),
            AbcCommand::PeerMessage(typ, from, peer_message) => {
                abc.handle_peer_message(typ, from, peer_message)
            }
            AbcCommand::Timeout(timeout) => abc.handle_timeout(timeout),
            AbcCommand::Backup => {
                backup = Some(abc.backup());
                continue;
            }
            AbcCommand::Recover => {
                if let Some(backup) = backup.clone() {
                    if let Some(output) = abc.restore(backup) {
                        output
                    } else {
                        continue;
                    }
                } else {
                    error!("tried to recover but enclave snapshot is missing");
                    continue;
                }
            }
        };

        if output.is_empty() {
            continue;
        }

        match output_sender.try_send(output) {
            Err(TrySendError::Closed(_)) => {
                error!("Cannot send output to output handler");
                return;
            }
            Err(TrySendError::Full(output)) => {
                warn!("Output is processed too slowly!");
                if output_sender.blocking_send(output).is_err() {
                    error!("Cannot send output to output handler");
                    return;
                }
            }
            Ok(()) => {}
        }
    }
    info!("worker OS thread stopped");
}

pub enum TimeoutChanage {
    Set(Duration, StopClass),
    Remove(StopClass),
    RemoveAll,
}

impl TimeoutChanage {
    fn start(self, now: impl FnOnce() -> Instant) -> TimeoutChanageStarted {
        match self {
            TimeoutChanage::Set(time, stop_class) => {
                TimeoutChanageStarted::Set(now() + time, stop_class)
            }
            TimeoutChanage::Remove(stop_class) => TimeoutChanageStarted::Remove(stop_class),
            TimeoutChanage::RemoveAll => TimeoutChanageStarted::RemoveAll,
        }
    }
}

enum TimeoutChanageStarted {
    Set(Instant, StopClass),
    Remove(StopClass),
    RemoveAll,
}

async fn async_to_sync_worker<A: AbcWrapper>(
    mut from_peers: mpsc::UnboundedReceiver<(MessageType, ReplicaId, A::PeerMessage)>,
    mut transaction_channel: Receiver<(ClientId, RequestId, A::ClientRequest)>,
    mut timeout_receiver: Receiver<(A::TimeoutType, TimeoutChanageStarted)>,
    ignore_timeouts_until: Instant,
    command_sender: Sender<AbcCommand<A>>,
    mut do_recover: Receiver<()>,
    mut do_backup: Receiver<()>,
) {
    let mut timeout_list = TimeoutList::<A::TimeoutType>::new();
    let mut replica_open = true;
    let mut client_open = true;

    loop {
        let command = select! {
            biased;
            Some(()) = do_backup.recv(), if replica_open || client_open => {
                AbcCommand::Backup
            }
            Some(()) = do_recover.recv(), if replica_open || client_open => {
                AbcCommand::Recover
            }
            Some((typ, state)) = timeout_receiver.recv(), if replica_open || client_open => {
                match state{
                    TimeoutChanageStarted::Set(time, stop_class) => timeout_list.insert(typ, time, stop_class),
                    TimeoutChanageStarted::Remove(stop_class) => timeout_list.remove(&typ, stop_class),
                    TimeoutChanageStarted::RemoveAll => timeout_list.remove_all(&typ),
                }
                continue;
            }
            msg = from_peers.recv(), if replica_open => {
                if let Some((typ, from, message)) = msg {
                    AbcCommand::PeerMessage(typ, from, message)
                } else {
                    replica_open = false;
                    continue
                }
            }
            msg = transaction_channel.recv(), if client_open => {
                if let Some(tx) = msg {
                    AbcCommand::ClientRequest(tx)
                } else {
                    client_open = false;
                    continue
                }
            }
            (time, typ) = timeout_list.wait_for_timeout(), if replica_open || client_open => {
                if time < ignore_timeouts_until {
                    continue
                }
                AbcCommand::Timeout(typ)
            }
            else => return
        };
        if let Err(e) = command_sender.send(command).await {
            error!("Cannot send command to nxbft worker: {:?}", e);
            return;
        }
    }
}
