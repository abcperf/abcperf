use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use abcperf_generic_client::cs::{
    http_warp::HttpWarp,
    quic_quinn::QuicQuinn,
    quic_s2n::QuicS2N,
    typed::{TypedCSTrait, TypedCSTraitClient},
    CSConfig, CSTrait,
};
use serde::{Deserialize, Serialize};
use std::future::Future;
use tokio::sync::oneshot;

async fn single<
    CS: CSTrait,
    T: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
    F: Fn(TypedCSTrait<CS, T, T>, SocketAddr) -> Fut,
    Fut: Future<Output = ()>,
>(
    cs: CS,
    expected_count: usize,
    run_client: F,
) {
    let cs = TypedCSTrait::<CS, T, T>::new(cs);

    let counter = Arc::new(AtomicUsize::new(0));

    let (send, recv) = oneshot::channel();

    let server = cs.configure_debug().server((Ipv4Addr::LOCALHOST, 0));

    let (addr, join_handle) = server.start(
        counter.clone(),
        |counter, r| {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Some(r) }
        },
        recv,
    );

    run_client(cs, addr).await;

    assert_eq!(counter.load(Ordering::SeqCst), expected_count);

    // shutdown
    send.send(()).unwrap();

    join_handle.await;
}

fn client<
    CS: CSTrait,
    Request: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
    Response: Serialize + for<'a> Deserialize<'a> + Send + 'static,
>(
    cs: TypedCSTrait<CS, Request, Response>,
) -> TypedCSTraitClient<<CS::Config as CSConfig>::Client, Request, Response> {
    let cs = cs.configure_debug();
    cs.client()
}

async fn run<CS: CSTrait>(cs: CS) {
    single::<_, (), _, _>(cs, 0, |_, _| async {}).await;

    single::<_, (), _, _>(cs, 0, |cs, addr| async move {
        let client = client(cs);
        client.connect(addr, "localhost").await.unwrap();
    })
    .await;

    single::<_, (), _, _>(cs, 1, |cs, addr| async move {
        let client = client(cs);
        let mut connection = client.connect(addr, "localhost").await.unwrap();
        connection.request(()).await.unwrap();
    })
    .await;

    single::<_, (), _, _>(cs, 2, |cs, addr| async move {
        let client = client(cs);
        let mut connection = client.connect(addr, "localhost").await.unwrap();
        connection.request(()).await.unwrap();
        connection.request(()).await.unwrap();
    })
    .await;

    single::<_, (), _, _>(cs, 2, |cs, addr| async move {
        let client = client(cs);
        {
            let mut connection = client.connect(addr, "localhost").await.unwrap();
            connection.request(()).await.unwrap();
        }
        {
            let mut connection = client.connect(addr, "localhost").await.unwrap();
            connection.request(()).await.unwrap();
        }
    })
    .await;

    single::<_, (), _, _>(cs, 2, |cs, addr| async move {
        {
            let client = client(cs);
            let mut connection = client.connect(addr, "localhost").await.unwrap();
            connection.request(()).await.unwrap();
        }
        {
            let client = client(cs);
            let mut connection = client.connect(addr, "localhost").await.unwrap();
            connection.request(()).await.unwrap();
        }
    })
    .await;

    single::<_, (), _, _>(cs, 2, |cs, addr| async move {
        let client = client(cs);
        let mut connection_1 = client.connect(addr, "localhost").await.unwrap();
        let mut connection_2 = client.connect(addr, "localhost").await.unwrap();
        connection_1.request(()).await.unwrap();
        connection_2.request(()).await.unwrap();
    })
    .await;

    single::<_, (), _, _>(cs, 2, |cs, addr| async move {
        let client_1 = client(cs);
        let client_2 = client(cs);
        let mut connection_1 = client_1.connect(addr, "localhost").await.unwrap();
        let mut connection_2 = client_2.connect(addr, "localhost").await.unwrap();
        connection_1.request(()).await.unwrap();
        connection_2.request(()).await.unwrap();
    })
    .await;

    single::<_, (), _, _>(cs, 100, |cs, addr| async move {
        let client = client(cs);
        let mut connection = client.connect(addr, "localhost").await.unwrap();
        for _ in 0..100 {
            connection.request(()).await.unwrap();
        }
    })
    .await;

    single::<_, (), _, _>(cs, 100, |cs, addr| async move {
        for _ in 0..100 {
            let client = client(cs);
            let mut connection = client.connect(addr, "localhost").await.unwrap();
            connection.request(()).await.unwrap();
        }
    })
    .await;

    single::<_, String, _, _>(cs, 100, |cs, addr| async move {
        let client = client(cs);
        let mut connection = client.connect(addr, "localhost").await.unwrap();
        for i in 0..100 {
            let input = i.to_string();

            let output = connection.request(input.clone()).await.unwrap();

            assert_eq!(input, output);
        }
    })
    .await;

    single::<_, String, _, _>(cs, 100, |cs, addr| async move {
        for i in 0..100 {
            let client = client(cs);
            let mut connection = client.connect(addr, "localhost").await.unwrap();

            let input = i.to_string();

            let output = connection.request(input.clone()).await.unwrap();

            assert_eq!(input, output);
        }
    })
    .await;
}

#[tokio::test]
async fn quic_s2n() {
    run(QuicS2N).await
}

#[tokio::test]
async fn quic_quinn() {
    run(QuicQuinn).await
}

#[tokio::test]
async fn http_warp() {
    run(HttpWarp).await
}
