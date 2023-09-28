use std::net::Ipv4Addr;

use abcperf_generic_client::cs::{
    http_warp::HttpWarp, quic_quinn::QuicQuinn, quic_s2n::QuicS2N, typed::TypedCSTrait, CSTrait,
};
use criterion::*;
use tokio::sync::oneshot;

async fn run<CS: CSTrait>(cs: CS, clients: usize) {
    let cs = TypedCSTrait::<CS, (), ()>::new(cs);

    let (send, recv) = oneshot::channel();

    let cs = cs.configure_debug();

    let server = cs.server((Ipv4Addr::LOCALHOST, 0));

    let (addr, join_handle) = server.start((), |(), ()| async { Some(()) }, recv);

    let mut handles = Vec::new();

    for _ in 0..clients {
        let client = cs.client();
        handles.push(tokio::spawn(async move {
            for _ in 0..100 {
                let mut connection = client.connect(addr, "localhost").await.unwrap();
                connection.request(()).await.unwrap();
            }
        }));
    }

    for fut in handles {
        fut.await.unwrap();
    }

    // shutdown
    send.send(()).unwrap();

    join_handle.await;
}

fn criterion_benchmark(c: &mut Criterion) {
    for clients in [1, 10, 20, 30] {
        c.bench_with_input(
            BenchmarkId::new("quic_s2n", clients),
            &clients,
            move |b, i| {
                b.to_async(tokio::runtime::Runtime::new().unwrap())
                    .iter_with_large_drop(|| async { run(QuicS2N, *i).await })
            },
        );
        c.bench_with_input(
            BenchmarkId::new("quic_quinn", clients),
            &clients,
            move |b, i| {
                b.to_async(tokio::runtime::Runtime::new().unwrap())
                    .iter_with_large_drop(|| async { run(QuicQuinn, *i).await })
            },
        );
        c.bench_with_input(
            BenchmarkId::new("http_warp", clients),
            &clients,
            move |b, i| {
                b.to_async(tokio::runtime::Runtime::new().unwrap())
                    .iter_with_large_drop(|| async { run(HttpWarp, *i).await })
            },
        );
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
