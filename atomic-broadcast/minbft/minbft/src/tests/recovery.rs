use super::*;
use test_log::test;
#[test]
fn test_recovery_request() {
    do_test_recovery_request();
}

fn do_test_recovery_request() {
    let n = 3;
    let t = n/2;
    let checkpoint_period = NonZeroU64::new(10).unwrap();


    let (mut minbfts, _) = setup_set(n, t, checkpoint_period.into());

    let id_req = ReplicaId::from_u64(2);

    let usig = UsigNoOp::default();
    let config = Config {
        n: n.try_into().unwrap(),
        t,
        id : id_req,
        max_batch_size: Some(1.try_into().expect("> 0")),
        batch_timeout: Duration::from_secs(1),
        initial_timeout_duration: Duration::from_secs(1),
        checkpoint_period,
        backoff_multiplier: 2,
    };
    let (minbft_req, out0) = MinBft::<DummyPayload, DummyReplicaStorage, UsigNoOp>::new(usig, config, DummyReplicaStorage::new(), true).unwrap();
    minbfts.insert(id_req, minbft_req);

    for broadcast in out0.broadcasts.iter() {
        println!("broadcast: {:?}", broadcast);
        for minbft in minbfts.values_mut() {
            if minbft.config.id != id_req {
                let out = minbft.handle_peer_message(id_req, broadcast.clone());

                for b in out.broadcasts.iter() {
                    println!("broadcast2: {:?}", b);
                }
            }
        }
    }



}

