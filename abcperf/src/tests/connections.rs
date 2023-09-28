use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    net::{Ipv4Addr, SocketAddr},
    time::Instant,
};

use crate::stats::ClientStats;
use crate::{
    atomic_broadcast::{AtomicBroadcast, AtomicBroadcastChannels, AtomicBroadcastConfiguration},
    MessageDestination, MessageType,
};
use anyhow::{Error, Result};
use async_trait::async_trait;
use futures::{future, Future};
use rstest::rstest;
use serde::{Deserialize, Serialize};
use shared_ids::{AnyId, ClientId, ReplicaId};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    client::{self, save::SaveOpt, ClientInnerOpt},
    config::{ClientConfig, Config, ReplicasConfig},
    replica::{self, ReplicaOpt},
    ClientEmulator, SeedRng, Server, VersionInfo,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestConfig;

impl From<TestConfig> for HashMap<String, String> {
    fn from(_: TestConfig) -> Self {
        HashMap::new()
    }
}

struct TestAlgo;

#[async_trait]
impl AtomicBroadcast for TestAlgo {
    type Config = TestConfig;
    type ReplicaMessage = ();
    type Transaction = ();
    type Decision = ();

    fn start(
        self,
        _config: AtomicBroadcastConfiguration<Self::Config>,
        _channels: AtomicBroadcastChannels<Self::ReplicaMessage, Self::Transaction, Self::Decision>,
        ready_for_clients: impl Send + 'static + FnOnce(),
    ) -> JoinHandle<Result<(), Error>> {
        ready_for_clients();
        tokio::spawn(async { Ok(()) })
    }
}

struct TestClientHandler;

#[async_trait]
impl Server for TestClientHandler {
    type AlgoRequest = ();

    /// responses for clients
    type AlgoResponse = ();

    /// messages send directly to other replicas
    type ReplicaMessage = ();

    type Config = TestConfig;

    #[allow(clippy::too_many_arguments)]
    async fn run<F: Send + Future<Output = ()>>(
        self,
        _config: ClientConfig<Self::Config>,
        _requests: mpsc::Sender<(ClientId, Self::AlgoRequest)>,
        _responses: mpsc::Receiver<Self::AlgoResponse>,
        exit: oneshot::Receiver<()>,
        ready: oneshot::Sender<SocketAddr>,
        _local_socket: SocketAddr,
        _replica_send: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
        _replica_recv: mpsc::Receiver<(MessageType, ReplicaId, Self::ReplicaMessage)>,
    ) {
        ready.send("[::]:0".parse().unwrap()).unwrap();

        exit.await.unwrap();
    }
}

struct TestClientEmulator {}

impl ClientEmulator for TestClientEmulator {
    type Config = TestConfig;

    fn start(
        self,
        _config: ClientConfig<Self::Config>,
        _replicas: Vec<(ReplicaId, SocketAddr)>,
        _seed: &mut SeedRng,
        _start_time: Instant,
    ) -> (oneshot::Sender<()>, JoinHandle<Result<Vec<ClientStats>>>) {
        let (exit_send, exit_recv) = oneshot::channel();
        let handle = tokio::spawn(async move {
            exit_recv.await.unwrap();

            Ok(vec![])
        });
        (exit_send, handle)
    }
}

static ALGO_INFO: VersionInfo = VersionInfo {
    abcperf_version: Cow::Borrowed("abcperf version for testing"),
    name: Cow::Borrowed("algo name for testing"),
    version: Cow::Borrowed("algo version for testing"),
};

async fn run_test<A: AtomicBroadcast<Config = TestConfig, Transaction = (), Decision = ()>>(
    n: u64,
    new_algo: impl Fn() -> A,
) {
    let (addr_send, addr_recv) = oneshot::channel();

    let client = tokio::spawn(client::run::<_, TestClientEmulator>(
        ClientInnerOpt::new(
            SaveOpt::discard(),
            "".into(),
            None,
            (Ipv4Addr::LOCALHOST, 0),
        ),
        Config {
            algo: TestConfig,
            n: n.try_into().unwrap(),
            t: 0,
            client: TestConfig,
            experiment_duration: 0f64,
            replicas: ReplicasConfig {
                //network: None,
                sample_delay: 1000000f64,
            },
            f: 0,
            omission_chance: 0.0,
        },
        ALGO_INFO.clone(),
        TestClientEmulator {},
        Some(addr_send),
    ));

    let addr = addr_recv.await.unwrap();

    for result in future::join_all((0..n).map(|_| {
        replica::main(ReplicaOpt::new(addr), ALGO_INFO.clone(), &new_algo, || {
            TestClientHandler
        })
    }))
    .await
    {
        result.unwrap();
    }

    client.await.unwrap().unwrap();
}

#[rstest]
#[tokio::test]
async fn test_control_connections(#[values(1, 2, 3, 4, 5, 10, 15, 20, 25)] n: u64) {
    run_test(n, || TestAlgo).await
}

#[derive(Debug, Serialize, Deserialize)]
enum TestAlgoConnectionsMsg {
    Broadcast,
    Unicast,
}

struct TestAlgoConnections;

#[async_trait]
impl AtomicBroadcast for TestAlgoConnections {
    type Config = TestConfig;
    type ReplicaMessage = TestAlgoConnectionsMsg;
    type Transaction = ();
    type Decision = ();

    fn start(
        self,
        config: AtomicBroadcastConfiguration<Self::Config>,
        channels: AtomicBroadcastChannels<Self::ReplicaMessage, Self::Transaction, Self::Decision>,
        ready_for_clients: impl Send + 'static + FnOnce(),
    ) -> JoinHandle<Result<(), Error>> {
        tokio::spawn(async move {
            let AtomicBroadcastChannels {
                mut incoming_replica_messages,
                outgoing_replica_messages,
                requests: _,
                responses: _,
            } = channels;

            outgoing_replica_messages
                .send((
                    MessageDestination::Broadcast,
                    TestAlgoConnectionsMsg::Broadcast,
                ))
                .await
                .unwrap();
            for replica in config.replicas().filter(|r| *r != config.replica_id) {
                outgoing_replica_messages
                    .send((
                        MessageDestination::Unicast(replica),
                        TestAlgoConnectionsMsg::Unicast,
                    ))
                    .await
                    .unwrap();
            }
            let others = config.n.get() - 1;
            let mut broadcasts = HashSet::new();
            let mut unicasts = HashSet::new();
            for _ in 0..(others * 2) {
                let (msg_type, from, msg) = incoming_replica_messages.recv().await.unwrap();
                assert_ne!(from, config.replica_id);
                assert!(from.as_u64() < config.n.get());
                match msg {
                    TestAlgoConnectionsMsg::Broadcast => {
                        assert!(broadcasts.insert(from));
                        assert_eq!(msg_type, MessageType::Broadcast);
                    }
                    TestAlgoConnectionsMsg::Unicast => {
                        assert!(unicasts.insert(from));
                        assert_eq!(msg_type, MessageType::Unicast);
                    }
                }
            }
            assert_eq!(broadcasts.len() as u64, others);
            assert_eq!(unicasts.len() as u64, others);
            ready_for_clients();
            assert!(matches!(
                incoming_replica_messages.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
            Ok(())
        })
    }
}

#[rstest]
#[tokio::test]
async fn test_algo_connections(#[values(1, 2, 3, 4, 5, 10, 15, 20, 25)] n: u64) {
    run_test(n, || TestAlgoConnections).await
}
