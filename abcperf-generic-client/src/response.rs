use std::{cmp::Ordering, collections::HashMap, mem};

use shared_ids::{AnyId, ClientId, RequestId};
use tokio::sync::oneshot;

#[derive(Debug)]
pub struct ResponseHandler<P> {
    clients: HashMap<ClientId, ClientHandlerState<P>>,
}

impl<P> Default for ResponseHandler<P> {
    fn default() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }
}

impl<P> ResponseHandler<P> {
    pub fn request(
        &mut self,
        client_id: ClientId,
        request_id: RequestId,
        channel: oneshot::Sender<P>,
    ) {
        self.clients
            .entry(client_id)
            .or_default()
            .request(request_id, channel)
    }

    pub fn response(&mut self, client_id: ClientId, request_id: RequestId, payload: P) {
        self.clients
            .entry(client_id)
            .or_default()
            .response(request_id, payload)
    }
}

#[derive(Debug)]
struct ClientHandlerState<P> {
    id: RequestId,
    inner: InnerClientHandlerState<P>,
}

impl<P> Default for ClientHandlerState<P> {
    fn default() -> Self {
        Self {
            id: RequestId::from_u64(0),
            inner: InnerClientHandlerState::Neither,
        }
    }
}

#[derive(Debug)]
enum InnerClientHandlerState<P> {
    Neither,
    Channel(oneshot::Sender<P>),
    Response(P),
}

impl<P> InnerClientHandlerState<P> {
    fn take(&mut self) -> Self {
        mem::replace(self, Self::Neither)
    }
}

impl<P> ClientHandlerState<P> {
    fn request(&mut self, id: RequestId, channel: oneshot::Sender<P>) {
        match id.cmp(&self.id) {
            Ordering::Less => {} // ignore
            Ordering::Equal => match self.inner.take() {
                InnerClientHandlerState::Neither | InnerClientHandlerState::Channel(_) => {
                    self.inner = InnerClientHandlerState::Channel(channel)
                }
                InnerClientHandlerState::Response(payload) => {
                    let _ = channel.send(payload);
                    self.next_id();
                }
            },
            Ordering::Greater => {
                *self = Self {
                    id,
                    inner: InnerClientHandlerState::Channel(channel),
                }
            }
        }
    }

    fn response(&mut self, id: RequestId, payload: P) {
        match id.cmp(&self.id) {
            Ordering::Less => {} // ignore
            Ordering::Equal => match self.inner.take() {
                InnerClientHandlerState::Neither => {
                    self.inner = InnerClientHandlerState::Response(payload)
                }
                InnerClientHandlerState::Channel(channel) => {
                    let _ = channel.send(payload);
                    self.next_id();
                }
                InnerClientHandlerState::Response(_) => {} // ignore
            },
            Ordering::Greater => {
                *self = Self {
                    id,
                    inner: InnerClientHandlerState::Response(payload),
                }
            }
        }
    }

    fn next_id(&mut self) {
        let id = self.id.as_mut_u64();
        *id = id.checked_add(1).unwrap();
    }
}
