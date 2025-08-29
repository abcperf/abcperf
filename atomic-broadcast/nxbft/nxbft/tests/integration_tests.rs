mod common;
use common::fuzzing::fuzzing;
use common::happy::happy;
use common::recovery::recovery;
use common::recovery::recovery_fuzzing;
use common::setup::instances;
use rayon::prelude::*;
use test_log::test;

#[test]
fn test_setup() {
    (3..=50).into_par_iter().for_each(|i| {
        let _ = instances(i);
    });
}

#[test]
fn test_happy() {
    happy();
}

#[test]
fn test_recovery_non_fuzzing() {
    recovery();
}

#[test]
#[should_panic]
fn test_recovery_fuzzing() {
    recovery_fuzzing();
}

#[test]
fn test_fuzzing() {
    fuzzing();
}
