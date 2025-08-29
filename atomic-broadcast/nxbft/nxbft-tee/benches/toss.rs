use std::ops::Deref;

use criterion::{criterion_group, criterion_main, Criterion};
use hashbar::{Hashbar, Hasher};
use id_set::ReplicaSet;
use nxbft::{
    enclave::{simple::SimpleEnclave, HandshakeResult, InitHandshakeResult},
    Enclave,
};
use nxbft_base::vertex::Vertex;
use nxbft_tee::NxbftTEE;
use rayon::prelude::*;
use shared_ids::ReplicaId;

struct DummyHashable;

impl Hashbar for DummyHashable {
    fn hash<H: Hasher>(&self, _hasher: &mut H) {}
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

fn make_signed_vertex<E: Enclave>(
    round: u64,
    replica: u64,
    instance: &mut E,
) -> Vertex<DummyHashable, E::Signature> {
    Vertex::new(
        round.into(),
        ReplicaId::from_u64(replica),
        ReplicaSet::default(),
        Vec::new(),
        |v| instance.sign(v.clone()),
    )
    .unwrap()
}

fn criterion_benchmark(c: &mut Criterion) {
    let mut noop = instances::<SimpleEnclave>(3);
    let mut tee = instances::<NxbftTEE>(3);

    let mut group = c.benchmark_group("toss");
    let mut c_0 = 1;
    let mut c_1 = 1;
    fn add(c: &mut u64) -> u64 {
        let cr = *c;
        *c += 1;
        cr
    }
    group.bench_function("noop", |b| {
        b.iter(|| {
            let _v0_1 = make_signed_vertex(add(&mut c_0), 0, &mut noop[0]);
            let _v0_2 = make_signed_vertex(add(&mut c_0), 0, &mut noop[0]);
            let _v0_3 = make_signed_vertex(add(&mut c_0), 0, &mut noop[0]);
            let v0_4 = make_signed_vertex(add(&mut c_0), 0, &mut noop[0]);

            let _v1_1 = make_signed_vertex(add(&mut c_1), 1, &mut noop[1]);
            let _v1_2 = make_signed_vertex(add(&mut c_1), 1, &mut noop[1]);
            let _v1_3 = make_signed_vertex(add(&mut c_1), 1, &mut noop[1]);
            let v1_4 = make_signed_vertex(add(&mut c_1), 1, &mut noop[1]);

            noop[2]
                .toss(vec![v0_4.deref().clone(), v1_4.deref().clone()])
                .unwrap();
        })
    });
    let mut c_0 = 1;
    let mut c_1 = 1;
    group.bench_function("tee", |b| {
        b.iter(|| {
            let _v0_1 = make_signed_vertex(add(&mut c_0), 0, &mut tee[0]);
            let _v0_2 = make_signed_vertex(add(&mut c_0), 0, &mut tee[0]);
            let _v0_3 = make_signed_vertex(add(&mut c_0), 0, &mut tee[0]);
            let v0_4 = make_signed_vertex(add(&mut c_0), 0, &mut tee[0]);

            let _v1_1 = make_signed_vertex(add(&mut c_1), 1, &mut tee[1]);
            let _v1_2 = make_signed_vertex(add(&mut c_1), 1, &mut tee[1]);
            let _v1_3 = make_signed_vertex(add(&mut c_1), 1, &mut tee[1]);
            let v1_4 = make_signed_vertex(add(&mut c_1), 1, &mut tee[1]);

            tee[2]
                .toss(vec![v0_4.deref().clone(), v1_4.deref().clone()])
                .unwrap();
        })
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
