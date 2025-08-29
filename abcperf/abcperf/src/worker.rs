use std::{net::SocketAddr, ops::Deref, pin::Pin};

use anyhow::{anyhow, Result};
use clap::Args;
use futures::Stream;
use ip_family::{IpFamily, IpFamilyExt};
use quinn::Connection;
use tracing::{error, error_span};

use crate::{
    application::Application,
    client::{self, ClientOpt},
    config::Config,
    connection::{self, bi_message_stream, Responder},
    message::{
        HelloMsg, HelloResponseError, InitMsg, InstanceType, OrchRepMessage, StartWaitForErrorMsg,
    },
    quic_new_client_connect,
    replica::{self, ReplicaOpt},
    AtomicBroadcast, InnerInterStageState, SecondStageMode, ValidatedConfig, VersionInfo,
};

pub(crate) enum OrchestratedOpt {
    Replica(ReplicaOpt),
    Client(ClientOpt),
}

impl From<ReplicaOpt> for OrchestratedOpt {
    fn from(value: ReplicaOpt) -> Self {
        Self::Replica(value)
    }
}

impl From<ClientOpt> for OrchestratedOpt {
    fn from(value: ClientOpt) -> Self {
        Self::Client(value)
    }
}

impl Deref for OrchestratedOpt {
    type Target = CommonOrchestratedOpt;

    fn deref(&self) -> &Self::Target {
        match self {
            OrchestratedOpt::Replica(r) => r,
            OrchestratedOpt::Client(c) => c,
        }
    }
}

impl OrchestratedOpt {
    fn instance_type(&self) -> InstanceType {
        match self {
            OrchestratedOpt::Replica(_) => InstanceType::Replica,
            OrchestratedOpt::Client(_) => InstanceType::Client,
        }
    }
}

/// Models the options with which a replica can be configured.
#[derive(Args)]
pub(super) struct CommonOrchestratedOpt {
    /// The address of the orchestrator to connect to.
    orchestrator_address: SocketAddr,
}

#[cfg(test)]
pub(super) async fn main<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>(
    opt: impl Into<OrchestratedOpt>,
    version_info: crate::VersionInfo,
    config: Config<ABC::Config, APP::Config>,
) -> Result<()> {
    let state = crate::worker::stage_1(opt, version_info).await?;

    let orchestrated_inter_stage_state = match state.mode {
        crate::SecondStageMode::Orchestrator(_) => unreachable!("we used orchestrated::stage_1"),
        crate::SecondStageMode::Orchestrated(s) => s,
    };

    stage_2::<ABC, APP>(*orchestrated_inter_stage_state, config).await?;

    Ok(())
}

impl CommonOrchestratedOpt {
    #[cfg(test)]
    pub(crate) fn new(orchestrator_address: SocketAddr) -> Self {
        Self {
            orchestrator_address,
        }
    }

    pub(crate) fn ip_family(&self) -> IpFamily {
        self.orchestrator_address.family()
    }
}

pub(super) async fn stage_1(
    opt: impl Into<OrchestratedOpt>,
    version_info: VersionInfo,
) -> Result<InnerInterStageState> {
    #[cfg(feature = "tokio-console")]
    console_subscriber::init();

    let opt = opt.into();
    // Create a new client connection to connect to the orchestrator.
    let replica_init_span = error_span!("worker-init").entered();
    let orchestrator_con = quic_new_client_connect(opt.orchestrator_address, "localhost").await?;

    // Clone Orchestrator connection to send and accept
    // unidirectional connections.
    let orchestrator_con_uni = orchestrator_con.clone();

    // Create a stream which repeatedly accepts bidirectional connections.
    let bi_streams = futures_util::stream::unfold(orchestrator_con, |c| async move {
        let con = c.accept_bi().await;
        Some((con, c))
    });

    // Create a stream with the items being the receiving data and
    // the responder of a bidirectional connection retrieved from the
    // created stream above.
    let mut orchestrator_con = Box::pin(bi_message_stream::<OrchRepMessage, _>(bi_streams));

    // Receive and handle the initial message sent by the Orchestrator.
    let (message, responder) =
        connection::recv_message_of_type::<HelloMsg, _>(&mut orchestrator_con).await?;
    replica_init_span.exit();

    // Check if the algorithm sent in the initial message corresponds
    // to the algorithm which was passed in this main function.
    if version_info != message.version_info {
        responder
            .reply(Err(HelloResponseError::AlgoMissmatch))
            .await?;
        return Err(anyhow!(
            "running {:?} and tried to connect to {:?}",
            version_info,
            &message.version_info
        ));
    }

    responder.reply(Ok(opt.instance_type())).await?;

    let state = OrchestratedInterStageState {
        opt,
        orchestrator_con,
        orchestrator_con_uni,
    };

    let config: Config = message.config_string.parse::<ValidatedConfig>()?.into();

    Ok(InnerInterStageState {
        abc_algorithm: config.abc.algorithm,
        state_machine_application: config.application.name,
        mode: SecondStageMode::Orchestrated(Box::new(state)),
        version_info,
        config_string: message.config_string,
    })
}

pub(super) async fn stage_2<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
>(
    OrchestratedInterStageState {
        opt,
        mut orchestrator_con,
        orchestrator_con_uni,
    }: OrchestratedInterStageState,
    config: Config<ABC::Config, APP::Config>,
) -> Result<()> {
    let (message, responder) =
        connection::recv_message_of_type::<InitMsg, _>(&mut orchestrator_con).await?;
    responder.reply(()).await?;

    let (StartWaitForErrorMsg, error_responder) =
        connection::recv_message_of_type(&mut orchestrator_con).await?;

    let result = match opt {
        OrchestratedOpt::Replica(opt) => {
            replica::stage_2::<ABC, APP>(
                opt,
                orchestrator_con,
                orchestrator_con_uni,
                config,
                message,
            )
            .await
        }
        OrchestratedOpt::Client(opt) => {
            client::stage_2::<ABC, APP>(
                opt,
                orchestrator_con,
                orchestrator_con_uni,
                config,
                message,
            )
            .await
        }
    };

    if let Err(e) = &result {
        if let Err(e) = error_responder.reply(anyhow!("{e:?}").into()).await {
            error!("Failed to report error to orchestrator: {e}");
        }
    }

    result
}

pub(crate) type OrchestratorCon = Pin<Box<dyn Stream<Item = Result<(OrchRepMessage, Responder)>>>>;
pub(crate) struct OrchestratedInterStageState {
    pub(crate) opt: OrchestratedOpt,
    pub(crate) orchestrator_con: OrchestratorCon,
    pub(crate) orchestrator_con_uni: Connection,
}
