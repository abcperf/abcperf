use std::fmt::{self, Debug};
use std::hash::Hash;
use std::str::FromStr;

use abcperf::application::Application;
use abcperf::InterStageState;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use shared_ids::hashbar::Hashbar;
use trait_alias_macro::trait_alias_macro;

use anyhow::{anyhow, Error, Result};
use shared_ids::RequestId;

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(ValueEnum, Clone)]
enum StateMachine {
    #[cfg(feature = "state-machine-noop")]
    Noop,
    #[cfg(feature = "state-machine-maas")]
    Maas,
}

impl fmt::Display for StateMachine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_possible_value().unwrap().get_name())
    }
}

impl FromStr for StateMachine {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        <Self as ValueEnum>::from_str(s, true).map_err(|s| anyhow!("invalid state machine: {}", s))
    }
}

#[derive(ValueEnum, Clone)]
enum Algorithm {
    #[cfg(feature = "algorithm-minbft-ed25519")]
    MinbftEd25519,
    #[cfg(feature = "algorithm-minbft-noop")]
    MinbftNoop,
    #[cfg(feature = "algorithm-minbft-tee")]
    MinbftTee,
    #[cfg(feature = "algorithm-nxbft-simple-enclave")]
    NxbftSimpleEnclave,
    #[cfg(feature = "algorithm-nxbft-tee")]
    NxbftTee,
    #[cfg(feature = "algorithm-damysus-noop")]
    DamysusNoop,
    #[cfg(feature = "algorithm-damysus-tee")]
    DamysusTee,
    #[cfg(feature = "algorithm-debug-reply")]
    DebugReply,
}

impl fmt::Display for Algorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_possible_value().unwrap().get_name())
    }
}

impl FromStr for Algorithm {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        <Self as ValueEnum>::from_str(s, true).map_err(|s| anyhow!("invalid algorithm: {}", s))
    }
}

const NAME: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-", env!("VERGEN_GIT_SHA"));

fn main() -> Result<()> {
    if let Some(state) = abcperf::stage_1(NAME, VERSION)? {
        select_state_machine(
            state.state_machine_application().parse()?,
            state.abc_algorithm().parse()?,
            state,
        )?;
    }

    Ok(())
}

fn select_state_machine(
    state_machine: StateMachine,
    algorithm: Algorithm,
    state: InterStageState,
) -> Result<()> {
    match state_machine {
        #[cfg(feature = "state-machine-noop")]
        StateMachine::Noop => {
            select_algorithm::<abcperf_noop::NoopApplication>(algorithm, state)?;
        }
        #[cfg(feature = "state-machine-maas")]
        StateMachine::Maas => {
            select_algorithm::<abcperf_maas::MaasApplication>(algorithm, state)?;
        }
    }

    Ok(())
}

trait_alias_macro!(PShared = Clone + Serialize + for<'a> Deserialize<'a> + Debug + Unpin + Send + Sync + Eq + AsRef<RequestId> + 'static + Hash);

#[cfg(feature = "minbft")]
trait_alias_macro!(PTrait = minbft::RequestPayload + PShared);
#[cfg(not(feature = "minbft"))]
trait_alias_macro!(PTrait = PShared);

fn select_algorithm<APP: Application>(
    algorithm: Algorithm,
    state: InterStageState,
) -> Result<(), anyhow::Error>
where
    APP::Transaction: Unpin + Hashbar + Eq + Hash,
{
    match algorithm {
        #[cfg(feature = "algorithm-minbft-ed25519")]
        Algorithm::MinbftEd25519 => {
            state.stage_2::<abcperf_minbft::ABCperfMinbft<APP::Transaction, usig::signature::UsigEd25519>, APP>()?;
        }
        #[cfg(feature = "algorithm-minbft-noop")]
        Algorithm::MinbftNoop => {
            state.stage_2::<abcperf_minbft::ABCperfMinbft<APP::Transaction, usig::noop::UsigNoOp>, APP>(
            )?;
        }
        #[cfg(feature = "algorithm-minbft-tee")]
        Algorithm::MinbftTee => {
            state
                .stage_2::<abcperf_minbft::ABCperfMinbft<APP::Transaction, usig_tee::UsigTEE>, APP>(
                )?;
        }
        #[cfg(feature = "algorithm-damysus-noop")]
        Algorithm::DamysusNoop => {
            state.stage_2::<abcperf_damysus::ABCperfDamysus<
                APP::Transaction,
                damysus::enclave::NoopEnclave,
            >, APP>(
            )?;
        }
        #[cfg(feature = "algorithm-damysus-tee")]
        Algorithm::DamysusTee => {
            state
                .stage_2::<abcperf_damysus::ABCperfDamysus<
                APP::Transaction,
                damysus_tee::DamysusTEE,
            >, APP>()?;
        }
        #[cfg(feature = "algorithm-nxbft-simple-enclave")]
        Algorithm::NxbftSimpleEnclave => {
            state.stage_2::<abcperf_nxbft::ABCperfNxbft<
                APP::Transaction,
                nxbft::enclave::simple::SimpleEnclave,
            >, APP>()?;
        }
        #[cfg(feature = "algorithm-nxbft-tee")]
        Algorithm::NxbftTee => {
            state
                .stage_2::<abcperf_nxbft::ABCperfNxbft<APP::Transaction, nxbft_tee::NxbftTEE>, APP>(
                )?;
        }
        #[cfg(feature = "algorithm-debug-reply")]
        Algorithm::DebugReply => {
            state.stage_2::<abcperf_debug::ReplyAlgo<APP::Transaction>, APP>()?;
        }
    };

    Ok(())
}
