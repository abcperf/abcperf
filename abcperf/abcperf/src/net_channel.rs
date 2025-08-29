use std::ops::Deref;

use anyhow::{anyhow, ensure, Result};
use bytes::Bytes;
use futures::SinkExt;
use quinn::{Connection, RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tracing::error;

type FramedReadQuinn = FramedRead<RecvStream, LengthDelimitedCodec>;
pub(crate) type FramedWriteQuinn = FramedWrite<SendStream, LengthDelimitedCodec>;

pub(crate) async fn init_sending_side(
    connection: &Connection,
    id: &'static str,
) -> Result<FramedWriteQuinn> {
    let stream = connection
        .open_uni()
        .await
        .map_err(|e| anyhow!("failed to open stream: {}", e))?;

    let mut framed = FramedWrite::new(stream, LengthDelimitedCodec::new());

    framed.send(Bytes::from(id)).await?;

    Ok(framed)
}

pub(crate) async fn init_receiving_side(
    connection: &Connection,
    id: &'static str,
) -> Result<FramedReadQuinn> {
    let stream = connection
        .clone()
        .accept_uni()
        .await
        .map_err(|e| anyhow!("failed to open stream: {}", e))?;
    let mut framed = FramedRead::new(stream, LengthDelimitedCodec::new());

    ensure!(
        framed
            .next()
            .await
            .ok_or_else(|| anyhow!("missing initial message"))??
            .as_ref()
            == id.as_bytes()
    );

    Ok(framed)
}

pub(crate) fn start_watch_channel_to_net<T: Send + Sync + 'static + Serialize>(
    channel: watch::Receiver<T>,
    net: FramedWriteQuinn,
) {
    async fn run<T: Serialize>(mut channel: watch::Receiver<T>, mut net: FramedWriteQuinn) {
        loop {
            let bytes = match bincode::serialize(channel.borrow_and_update().deref()) {
                Ok(bytes) => bytes,
                Err(e) => {
                    error!("{e}");
                    break;
                }
            };

            let bytes = Bytes::from(bytes);

            match net.send(bytes).await {
                Ok(()) => {}
                Err(e) => {
                    error!("{e}");
                    break;
                }
            }

            match channel.changed().await {
                Ok(()) => {}
                Err(_) => {
                    break;
                }
            }
        }
    }

    tokio::spawn(run(channel, net));
}

pub(crate) fn start_net_to_channel<T: Send + 'static + for<'a> Deserialize<'a>>(
    net: FramedReadQuinn,
    channel: mpsc::Sender<T>,
) {
    async fn run<T: for<'a> Deserialize<'a>>(mut net: FramedReadQuinn, channel: mpsc::Sender<T>) {
        while let Some(result) = net.next().await {
            let bytes = match result {
                Ok(bytes) => bytes,
                Err(e) => {
                    error!("{e}");
                    break;
                }
            };
            let data = match bincode::deserialize(&bytes) {
                Ok(data) => data,
                Err(e) => {
                    error!("{e}");
                    break;
                }
            };
            match channel.send(data).await {
                Ok(()) => {}
                Err(_) => break,
            }
        }
    }

    tokio::spawn(run(net, channel));
}
