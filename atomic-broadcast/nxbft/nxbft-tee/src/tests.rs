use std::ops::Deref;

use hashbar::{Hashbar, Hasher};
use id_set::ReplicaSet;
use nxbft_base::enclave::Enclave;
use nxbft_base::enclave::{HandshakeResult, InitHandshakeResult};
use nxbft_base::vertex::Vertex;
use rayon::prelude::*;
use remote_attestation_shared::types::RawSignature;
use rstest::rstest;
use sgx_types::types::Ec256Signature;
use shared_ids::ReplicaId;

use crate::types::Signature;

use super::NxbftTEE;

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

fn make_signed_vertex(
    counter: u64,
    replica: u64,
    instance: &mut NxbftTEE,
) -> Vertex<DummyHashable, Signature> {
    Vertex::new(
        counter.into(),
        ReplicaId::from_u64(replica),
        ReplicaSet::default(),
        Vec::new(),
        |v| instance.sign(v.clone()),
    )
    .unwrap()
}

#[test]
fn incomplete_setup() {
    let mut enclave = NxbftTEE::new(3);
    assert!(!enclave.is_ready());
    enclave.init().unwrap();
    assert!(!enclave.is_ready());
    enclave.get_attestation_certificate().unwrap();
    assert!(!enclave.is_ready());
}

#[test]
#[should_panic]
fn invalid_peer_num() {
    let _ = NxbftTEE::new(2);
}

fn instances(n: u64) -> Vec<NxbftTEE> {
    let mut instances = Vec::with_capacity(n as usize);
    for _ in 0..n {
        instances.push(NxbftTEE::new(n));
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

#[rstest]
fn setup(#[values(3, 4, 5, 10)] n: u64) {
    for instance in instances(n) {
        assert!(instance.is_ready())
    }
}

#[test]
fn sign_attempt() {
    let mut instances = instances(3);

    let vt = make_vertex(0, 0);

    instances[0].sign(vt.deref().deref().clone()).unwrap();
}

#[test]
fn integrate_three() {
    let mut instances = instances(3);
    let _v0_1 = make_signed_vertex(1, 0, &mut instances[0]);
    let _v0_2 = make_signed_vertex(2, 0, &mut instances[0]);
    let _v0_3 = make_signed_vertex(3, 0, &mut instances[0]);
    let v0_4 = make_signed_vertex(4, 0, &mut instances[0]);

    let _v1_1 = make_signed_vertex(1, 1, &mut instances[1]);
    let _v1_2 = make_signed_vertex(2, 1, &mut instances[1]);
    let _v1_3 = make_signed_vertex(3, 1, &mut instances[1]);
    let v1_4 = make_signed_vertex(4, 1, &mut instances[1]);

    let _v2_1 = make_signed_vertex(1, 2, &mut instances[2]);
    let _v2_2 = make_signed_vertex(2, 2, &mut instances[2]);
    let _v2_3 = make_signed_vertex(3, 2, &mut instances[2]);
    let _v2_4 = make_signed_vertex(4, 2, &mut instances[2]);

    let toss = instances[0].toss(vec![v0_4.clone(), v1_4.clone()]).unwrap();

    assert!(toss.as_u64() < 3);
}
