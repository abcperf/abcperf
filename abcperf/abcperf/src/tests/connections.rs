use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
    time::{Duration, Instant},
};

use crate::{
    application::{
        client::{ApplicationClientEmulator, SMRClientFactory},
        server::{ApplicationServer, ApplicationServerChannels, ApplicationServerStarted},
        smr_client::SmrClient,
        Application,
    },
    stats::ClientStats,
};
use crate::{
    atomic_broadcast::{ABCChannels, AtomicBroadcast},
    client::ClientOpt,
    config::{
        update::{Invocateable, LoadConfigUpdate},
        ABCConfigWrapper, ApplicationClientEmulatorConfig, ApplicationConfigWrapper,
        ApplicationServerConfig, AtomicBroadcastConfiguration, BasicConfig, Config, NTConfig,
        NetworkConfig, OrchestratorRequestGenerationConfig,
    },
    orchestrator::{self, save::SaveOpt, OrchestratorInnerOpt},
    worker, MessageDestination, MessageType,
};
use anyhow::Result;
use futures::Future;
use rstest::rstest;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, oneshot},
    task::{JoinHandle, LocalSet},
    time,
};
use tracing::info;
use tracing_test::traced_test;

use crate::{replica::ReplicaOpt, VersionInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestConfig {}

struct TestAlgo;

impl AtomicBroadcast for TestAlgo {
    type Config = TestConfig;
    type ReplicaMessage = ();
    type Transaction = ();

    fn start<F: Send + Future<Output = ()> + 'static>(
        _config: AtomicBroadcastConfiguration<Self::Config>,
        _channels: ABCChannels<Self::ReplicaMessage, Self::Transaction>,
        ready_for_clients: impl Send + 'static + FnOnce(),
        _send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> JoinHandle<Vec<Instant>> {
        ready_for_clients();
        tokio::spawn(async { vec![] })
    }
}

struct TestApplicationServer;

impl ApplicationServer for TestApplicationServer {
    type Request = ();

    type Response = ();

    type Transaction = ();

    type Config = TestConfig;

    type ReplicaMessage = ();

    fn start<F: Send + Future<Output = ()>>(
        _config: ApplicationServerConfig<Self::Config>,
        channels: ApplicationServerChannels<Self>,
        _send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> ApplicationServerStarted<Self> {
        let _ = channels.ready.send(());
        ApplicationServerStarted {
            abc_transaction_input_channel: mpsc::channel(1).1,
            abc_transaction_output_channel: mpsc::channel(1).0,
            application_server_join_handle: tokio::spawn(async { Ok(()) }),
        }
    }
}

struct TestApplicationClientEmulator;

impl<CF: SMRClientFactory<(), ()>> ApplicationClientEmulator<CF> for TestApplicationClientEmulator {
    type Request = ();

    type Response = ();

    type Config = TestConfig;

    fn start(
        _client_factory: CF,
        _config: ApplicationClientEmulatorConfig<Self::Config>,
        args: crate::application::client::ApplicationClientEmulatorArguments,
    ) -> JoinHandle<Result<Vec<ClientStats>>> {
        tokio::spawn(async move {
            args.stop.cancelled().await;

            Ok(vec![])
        })
    }
}

struct TestApplication;

impl Application for TestApplication {
    type Request = ();

    type Response = ();

    type Transaction = ();

    type ReplicaMessage = ();

    type Config = TestConfig;

    type ClientEmulator<CF: SMRClientFactory<Self::Request, Self::Response>> =
        TestApplicationClientEmulator;
    type Server = TestApplicationServer;
}

static ALGO_INFO: VersionInfo = VersionInfo {
    abcperf_version: Cow::Borrowed("abcperf version for testing"),
    bin_name: Cow::Borrowed("bin name for testing"),
    bin_version: Cow::Borrowed("bin version for testing"),
};

async fn run_test<A: AtomicBroadcast<Config = TestConfig, Transaction = ()>>(n: u64) {
    let (addr_send, addr_recv) = oneshot::channel();

    let config = Config {
        basic: BasicConfig {
            nt: NTConfig {
                n: n.try_into().unwrap(),
                t: 0,
            },
            experiment_label: String::new(),
            run_tags: HashMap::new(),
            client_workers: 1.try_into().unwrap(),
        },

        experiment_duration: Some(0f64),
        abc: ABCConfigWrapper {
            algorithm: "".to_owned(),
            config: TestConfig {},
        },
        fault_emulation: vec![],
        application: ApplicationConfigWrapper {
            name: "".to_owned(),
            config: TestConfig {},
        },
        orchestrator_request_generation: OrchestratorRequestGenerationConfig {
            load: vec![Invocateable {
                invocation_time: 0.0,
                config: LoadConfigUpdate {
                    ticks_per_second: 1.try_into().unwrap(),
                    requests_per_tick: 1.try_into().unwrap(),
                },
            }],
            smr_client: SmrClient::Fallback,
            fallback_timeout: 0.0,
        },
        network: NetworkConfig {
            replica_to_replica: vec![],
            client_to_replica: vec![],
        },
    };

    let client = tokio::spawn(orchestrator::run::<A, TestApplication>(
        OrchestratorInnerOpt::new(SaveOpt::discard(), None, None, (Ipv4Addr::LOCALHOST, 0)),
        config.clone(),
        ALGO_INFO.clone(),
        Some(addr_send),
    ));

    let addr = addr_recv.await.unwrap();

    let local = LocalSet::new();

    local
        .run_until(async move {
            let mut joins = Vec::new();

            for _ in 0..n {
                joins.push(tokio::task::spawn_local(
                    worker::main::<A, TestApplication>(
                        ReplicaOpt::new(addr),
                        ALGO_INFO.clone(),
                        config.clone(),
                    ),
                ));
            }

            joins.push(tokio::task::spawn_local(
                worker::main::<A, TestApplication>(
                    ClientOpt::new(addr),
                    ALGO_INFO.clone(),
                    config.clone(),
                ),
            ));

            for join in joins {
                join.await.unwrap().unwrap();
            }
        })
        .await;

    client.await.unwrap().unwrap();
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn test_control_connections(#[values(1, 2, 3, 4, 5, 10, 15, 20)] n: u64) {
    run_test::<TestAlgo>(n).await
}

#[derive(Debug, Serialize, Deserialize)]
enum TestAlgoConnectionsMsg {
    Broadcast,
    Unicast,
}

struct TestAlgoConnections;

impl AtomicBroadcast for TestAlgoConnections {
    type Config = TestConfig;
    type ReplicaMessage = TestAlgoConnectionsMsg;
    type Transaction = ();

    fn start<F: Send + Future<Output = ()> + 'static>(
        config: AtomicBroadcastConfiguration<Self::Config>,
        channels: ABCChannels<Self::ReplicaMessage, Self::Transaction>,
        ready_for_clients: impl Send + 'static + FnOnce(),
        send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> JoinHandle<Vec<Instant>> {
        tokio::spawn(async move {
            let ABCChannels {
                mut incoming_replica_messages,
                transaction_input: _,
                transaction_output: _,
                update_info: _,
                ..
            } = channels;

            send_to_replica(
                MessageDestination::Broadcast,
                TestAlgoConnectionsMsg::Broadcast,
            )
            .await;
            for replica in config.replicas() {
                send_to_replica(
                    MessageDestination::Unicast(replica),
                    TestAlgoConnectionsMsg::Unicast,
                )
                .await;
            }

            time::sleep(Duration::from_secs(1)).await;

            let n = config.n.get();
            let mut broadcasts = HashSet::new();
            let mut unicasts = HashSet::new();
            for _ in 0..((n * 2) - 1) {
                let (msg_type, from, msg) = incoming_replica_messages.recv().await.unwrap();
                assert!(from.as_u64() < n);
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
            assert_eq!(broadcasts.len() as u64, n - 1);
            assert_eq!(unicasts.len() as u64, n);
            ready_for_clients();
            assert!(matches!(
                incoming_replica_messages.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
            vec![]
        })
    }
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn test_algo_connections(#[values(1, 2, 3, 4, 5, 10, 15, 20)] n: u64) {
    info!("Test {n}");
    run_test::<TestAlgoConnections>(n).await
}
