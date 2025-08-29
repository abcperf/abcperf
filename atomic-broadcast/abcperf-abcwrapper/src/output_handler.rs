use std::{future::Future, time::Instant};

use abcperf::{atomic_broadcast::AtomicBroadcastInfo, MessageDestination};
use shared_ids::{ClientId, RequestId};
use tokio::sync::mpsc::{self, error::TrySendError};
use tracing::{error, warn};

use crate::{AbcOutput, AbcWrapper, GenAtomicBroadcastInfo, TimeoutChanageStarted};

pub(super) struct OutputHandler<
    A: AbcWrapper,
    ReadyFunction: Send + 'static + FnOnce() + Sync,
    F: Send + Future<Output = ()>,
    SendToReplica: 'static + Sync + Send + Fn(MessageDestination, A::PeerMessage) -> F,
> {
    output_receiver: mpsc::Receiver<A::Output>,
    decisions: mpsc::Sender<(ClientId, RequestId, A::ClientRequest)>,
    ready_for_clients: Option<ReadyFunction>,
    update_info: mpsc::Sender<AtomicBroadcastInfo>,
    last_info: Option<<A::Output as AbcOutput>::GenAtomicBroadcastInfo>,
    ready: bool,
    send_to_replica: SendToReplica,
    stats: Vec<Instant>,
    timeout_sender: mpsc::Sender<(A::TimeoutType, TimeoutChanageStarted)>,
    do_backup: mpsc::Sender<()>,
}

impl<
        A: AbcWrapper,
        ReadyFunction: Send + 'static + FnOnce() + Sync,
        F: Send + Future<Output = ()>,
        SendToReplica: 'static + Sync + Send + Fn(MessageDestination, A::PeerMessage) -> F,
    > OutputHandler<A, ReadyFunction, F, SendToReplica>
{
    pub fn new(
        output_receiver: mpsc::Receiver<A::Output>,
        decisions: mpsc::Sender<(ClientId, RequestId, A::ClientRequest)>,
        ready_for_clients: ReadyFunction,
        update_info: mpsc::Sender<AtomicBroadcastInfo>,
        send_to_replica: SendToReplica,
        timeout_sender: mpsc::Sender<(A::TimeoutType, TimeoutChanageStarted)>,
        do_backup: mpsc::Sender<()>,
    ) -> Self {
        Self {
            output_receiver,
            decisions,
            ready_for_clients: Some(ready_for_clients),
            update_info,
            last_info: None,
            ready: false,
            send_to_replica,
            stats: vec![],
            timeout_sender,
            do_backup,
        }
    }

    pub async fn run(mut self) -> Vec<Instant> {
        while let Some(output) = self.output_receiver.recv().await {
            self.process_output(output).await;
        }
        self.stats
    }

    pub async fn process_output(&mut self, output: A::Output) {
        if output.do_backup() {
            match self.do_backup.try_send(()) {
                Ok(()) => (),
                Err(TrySendError::Full(_)) => {
                    error!("should do enclave backup but the previous one did not finish")
                }
                Err(TrySendError::Closed(_)) => {
                    error!("should do enclave backup but the channel ist closed")
                }
            }
        }
        if output.sets_ready() {
            if let Some(f) = self.ready_for_clients.take() {
                self.ready = true;
                f();
            }
        }
        let (timeouts, messages, outputs, mut new_info_gen, decision_times) =
            output.destructure(self.ready);

        let mut now = None;
        for (typ, timeout) in timeouts {
            let timeout = timeout.start(|| *now.get_or_insert_with(Instant::now));

            if let Err(e) = self.timeout_sender.send((typ, timeout)).await {
                error!("Error sending timeout: {:?}", e);
            }
        }

        for d in outputs {
            self.decisions.send(d).await.unwrap_or_else(|e| {
                error!("Error sending decision: {:?}", e);
            });
        }
        for (to, msg) in messages {
            (self.send_to_replica)(to, msg).await;
        }

        if let Some(new_info) = new_info_gen.generate_info(self.last_info.as_ref()) {
            match self.update_info.try_send(new_info) {
                Ok(_) | Err(TrySendError::Closed(_)) => {}
                Err(TrySendError::Full(_)) => {
                    warn!("too many updates, atomic broadcast info inaccurate");
                }
            }
        }
        self.last_info = Some(new_info_gen);

        self.stats.extend(decision_times);
    }
}
