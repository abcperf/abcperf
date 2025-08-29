use std::{
    env,
    net::{Ipv6Addr, UdpSocket},
    path::PathBuf,
};

use anyhow::Result;
use clap::Args;
use tokio::process::Command;

use crate::config::{Config, ValidatedConfig};

#[derive(Args)]
pub(super) struct LocalOpt {
    /// The config file to use.
    config: PathBuf,
}

pub(crate) async fn main(opt: LocalOpt) -> Result<()> {
    #[cfg(feature = "tokio-console")]
    console_subscriber::init();

    let config: Config = ValidatedConfig::load(&opt.config)?.into();

    let socket = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0))?;
    let port = socket.local_addr()?.port();
    drop(socket);
    let socket: String = format!("[::1]:{port}");

    let mut orchestrator = command()?
        .arg("orchestrator")
        .arg(&socket)
        .arg(opt.config)
        .spawn()?;

    let mut replicas = Vec::new();
    for _ in 0..config.n.get() {
        replicas.push(command()?.arg("replica").arg(&socket).spawn()?);
    }

    orchestrator.wait().await?;
    for mut replica in replicas {
        replica.wait().await?;
    }
    Ok(())
}

fn command() -> Result<Command> {
    let executable = env::current_exe()?;
    let mut command = Command::new(executable);
    for arg in env::args().skip(1) {
        if arg.eq_ignore_ascii_case("local") {
            break;
        }
        command.arg(arg);
    }
    Ok(command)
}
