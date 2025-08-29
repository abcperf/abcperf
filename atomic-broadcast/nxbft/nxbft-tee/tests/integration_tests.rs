mod common;
use common::happy::happy;
use common::recovery::recovery;
use common::setup::instances;
use rstest::rstest;

#[rstest]
fn test_setup(#[values(3, 4, 5, 10)] n: u64) {
    let _ = instances(n);
}

#[rstest]
fn test_happy(#[values(3, 4, 5, 10)] n: u64) {
    happy(n);
}

#[rstest]
fn test_recovery(#[values(3, 4, 5, 10)] n: u64) {
    recovery(n);
}
