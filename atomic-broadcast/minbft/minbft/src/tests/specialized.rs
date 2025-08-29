use super::*;
use test_log::test;
#[test]
fn vc2_test() {
    do_vc2_test();
}


fn do_vc2_test() {

    let n = 3;
    let t = 1;
    let checkpoint_period = 10;

    //TODO test mit id=0 also primary

    let ((mut minbft,_),_) = minimal_setup(n, t, ReplicaId::from_u64(2), checkpoint_period);

    let req0 = DummyPayload(0, true);
    let req1 = DummyPayload(1, true);


    let out0 = minbft.handle_client_message(ClientId::from_u64(0), req0);
    println!("out0: {:?}", out0.broadcasts);
    println!("out0: {:?}", out0.responses);
    println!("out0: {:?}", out0.timeout_requests);
    println!("out0: {:?}", out0.errors);
    println!("out0: {:?}", minbft.view_state);
    println!("=================================================");

    let out1 = minbft.handle_timeout(TimeoutType::Client);
    println!("out1: {:?}", out1.broadcasts);
    println!("out1: {:?}", out1.responses);
    println!("out1: {:?}", out1.timeout_requests);
    println!("out1: {:?}", out1.errors);
    println!("out1: {:?}", minbft.view_state);
    println!("=================================================");

    /*let out2 = minbft.handle_timeout(TimeoutType::ViewChange); 
    println!("out2: {:?}", out2.broadcasts);
    println!("out2: {:?}", out2.responses);
    println!("out2: {:?}", out2.timeout_requests);
    println!("out2: {:?}", out2.errors);
    println!("out2: {:?}", minbft.view_state);
    println!("=================================================");
    */

    let out3 = minbft.handle_client_message(ClientId::from_u64(0), req1);
    println!("out3: {:?}", out3.broadcasts);
    println!("out3: {:?}", out3.responses);
    println!("out3: {:?}", out3.timeout_requests);
    println!("out3: {:?}", out3.errors);
    println!("out3: {:?}", minbft.view_state);
    println!("=================================================");

    let out4 = minbft.handle_timeout(TimeoutType::Client);
    println!("out4: {:?}", out4.broadcasts);
    println!("out4: {:?}", out4.responses);
    println!("out4: {:?}", out4.timeout_requests);
    println!("out4: {:?}", out4.errors);
    println!("out4: {:?}", minbft.view_state);
    println!("=================================================");

}
