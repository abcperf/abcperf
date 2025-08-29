use std::ops::Deref;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use hashbar::{Hashbar, Hasher};
use id_set::ReplicaSet;
use nxbft::{
    enclave::{simple::SimpleEnclave, HandshakeResult, InitHandshakeResult},
    Enclave,
};
use nxbft_base::vertex::Vertex;
use nxbft_tee::NxbftTEE;
use rayon::prelude::*;
use remote_attestation_shared::types::RawSignature;
use sgx_types::types::Ec256Signature;
use shared_ids::ReplicaId;

struct DummyHashable;

impl Hashbar for DummyHashable {
    fn hash<H: Hasher>(&self, _hasher: &mut H) {}
}

fn make_vertex(counter: u64, replica: u64) -> Vertex<DummyHashable, RawSignature> {
    Vertex::new(
        counter.into(),
        ReplicaId::from_u64(replica),
        ReplicaSet::default(),
        Vec::new(),
        |_| Ok::<_, ()>((Ec256Signature::default().into(), 0)),
    )
    .unwrap()
}

fn instances<E: Enclave + Send>(n: u64) -> Vec<E>
where
    <E as Enclave>::Attestation: Send,
{
    let mut instances = Vec::with_capacity(n as usize);
    for _ in 0..n {
        instances.push(E::new(n));
    }

    let certs: Vec<_> = instances
        .par_iter_mut()
        .map(|instance| {
            instance.init().unwrap();
            instance.get_attestation_certificate().unwrap()
        })
        .collect();

    let mut shares = Vec::new();

    for (i, cert) in certs.into_iter().enumerate() {
        for j in 0u64..n {
            if let InitHandshakeResult::Ok(share) = instances[j as usize]
                .init_peer_handshake(ReplicaId::from_u64(i as u64), cert.clone())
            {
                shares.push((i, j, share));
            } else {
                panic!("Expected valid handshake result");
            }
        }
    }

    for (i, j, share) in shares {
        assert!(matches!(
            instances[i].complete_peer_handshake(ReplicaId::from_u64(j), &share),
            HandshakeResult::Ok
        ));
    }

    instances
}

fn criterion_benchmark(c: &mut Criterion) {
    let mut noop = instances::<SimpleEnclave>(10);
    let mut tee = instances::<NxbftTEE>(10);

    let mut group = c.benchmark_group("sign");

    group.bench_function("noop", |b| {
        b.iter(|| {
            let vt = make_vertex(0, 0);
            noop[0].sign(black_box(vt.deref().deref().clone()))
        })
    });
    group.bench_function("tee", |b| {
        b.iter(|| {
            let vt = make_vertex(0, 0);
            tee[0].sign(black_box(vt.deref().deref().clone()))
        })
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
