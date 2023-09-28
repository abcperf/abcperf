use std::{
    marker::PhantomData,
    net::IpAddr,
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use futures::{future, ready, Stream, StreamExt, TryStream};
use pin_project::pin_project;
use quinn::{Connection, SendStream};
use serde::Deserialize;
use shared_ids::ReplicaId;

use crate::message::{CSMsg, ClientReplicaMessage};

pub(super) fn bi_message_stream<
    T: for<'a> Deserialize<'a>,
    S: Stream<Item = Result<(quinn::SendStream, quinn::RecvStream), quinn::ConnectionError>>,
>(
    streams: S,
) -> impl Stream<Item = Result<(T, Responder)>> {
    let stream = streams.then(|r| async move {
        let (send, mut recv) = r?;
        let data = recv.read_to_end(crate::READ_TO_END_LIMIT).await?;
        Ok((BytesMut::from(data.as_slice()), Responder(send)))
    });
    BiStream::new(stream)
}

pub(super) struct Responder(SendStream);

impl Responder {
    async fn reply<M: CSMsg>(mut self, data: M::Response) -> Result<()> {
        let data = bincode::serialize(&data)?;
        self.0.write_all(&data).await?;
        self.0.finish().await?;
        Ok(())
    }
}

pub(super) struct TypedResponder<M: CSMsg> {
    inner: Responder,
    phantom_data: PhantomData<M>,
}

impl<M: CSMsg> TypedResponder<M> {
    pub(super) async fn reply(self, data: M::Response) -> Result<()> {
        self.inner.reply::<M>(data).await
    }
}

pub(super) async fn recv_message_of_type<
    M: CSMsg,
    C: Stream<Item = Result<(ClientReplicaMessage, Responder)>> + Unpin,
>(
    client: &mut C,
) -> Result<(M, TypedResponder<M>)> {
    let (msg, responder) = client
        .next()
        .await
        .ok_or_else(|| anyhow!("message missing"))??;
    let msg = M::try_from(msg)
        .map_err(|_| anyhow!("not a message of type {}", std::any::type_name::<M>()))?;
    Ok((msg, responder.into()))
}

impl<M: CSMsg> From<Responder> for TypedResponder<M> {
    fn from(inner: Responder) -> Self {
        Self {
            inner,
            phantom_data: PhantomData::default(),
        }
    }
}

#[pin_project]
#[derive(Debug)]
struct BiStream<
    S: TryStream<Ok = (BytesMut, Responder), Error = anyhow::Error>,
    T: for<'a> Deserialize<'a>,
> {
    #[pin]
    stream: S,
    phantom_data: PhantomData<T>,
}

impl<
        S: TryStream<Ok = (BytesMut, Responder), Error = anyhow::Error>,
        T: for<'a> Deserialize<'a>,
    > BiStream<S, T>
{
    fn new(stream: S) -> Self {
        Self {
            stream,
            phantom_data: PhantomData::default(),
        }
    }
}

impl<
        S: TryStream<Ok = (BytesMut, Responder), Error = anyhow::Error>,
        T: for<'a> Deserialize<'a>,
    > Stream for BiStream<S, T>
{
    type Item = Result<(T, Responder), S::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(
            match ready!(self.as_mut().project().stream.try_poll_next(cx)) {
                Some(data) => {
                    let (bytes, responder) = data?;
                    Some(Ok((bincode::deserialize(&bytes)?, responder)))
                }
                None => None,
            },
        )
    }
}

pub(super) struct ReplicaConnections {
    connections: Box<[ReplicaConnection]>,
}

impl ReplicaConnections {
    pub(super) fn new(connections: impl Into<Box<[ReplicaConnection]>>) -> Self {
        Self {
            connections: connections.into(),
        }
    }

    pub(super) fn remote_addresses(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.connections
            .iter()
            .map(|c| c.connection.remote_address().ip())
    }

    pub(super) fn ids(&self) -> impl Iterator<Item = ReplicaId> + '_ {
        self.connections.iter().map(|c| c.id)
    }

    pub(super) async fn send_all<M: CSMsg>(&mut self, msg: M) -> Result<Vec<M::Response>> {
        let msg = msg.into();
        let msg = &msg;
        let vec = future::join_all(
            self.connections
                .iter_mut()
                .map(|con| async move { con.send_internal::<M>(msg).await }),
        )
        .await;
        vec.into_iter().collect()
    }
}

pub(super) struct ReplicaConnection {
    id: ReplicaId,
    connection: Connection,
}

impl ReplicaConnection {
    pub(super) fn new(id: ReplicaId, connection: Connection) -> Self {
        Self { id, connection }
    }

    pub(super) async fn send<M: CSMsg>(&mut self, msg: M) -> Result<M::Response> {
        let msg = msg.into();
        self.send_internal::<M>(&msg).await
    }

    async fn send_internal<M: CSMsg>(&mut self, msg: &ClientReplicaMessage) -> Result<M::Response> {
        let msg = bincode::serialize(msg)?;
        let (mut send, mut recv) = self.connection.open_bi().await?;
        send.write_all(&msg).await?;
        send.finish().await?;
        let data = recv.read_to_end(crate::READ_TO_END_LIMIT).await?;
        Ok(bincode::deserialize(&data)?)
    }
}
