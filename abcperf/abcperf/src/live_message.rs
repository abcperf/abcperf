use std::{net::IpAddr, num::NonZeroUsize, sync::Arc};

use netem::config::Config;
use serde::{Deserialize, Serialize};

/// Enumerates the possible messages sent live from the Orchestrator to
/// the client emulator.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum OrchToClientEmuLiveMsg {
    /// Models the adjustment of the number of parallel client requests.
    ParallelReqsUpdate(NonZeroUsize),
    /// Models the message that contains network config adjustments.
    NetConfigAdj(Config),
}

/// Models the network configuration adjustment message.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct NetConfigAdjMsg {
    /// The destination addresses for which the egress traffic of
    /// the network should be configured.
    pub destinations: Arc<[IpAddr]>,
    /// The NetEm configuration to be applied for the destination addresses.
    pub config: Config,
}
