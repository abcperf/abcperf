mod common;
use common::{
    fuzzing::{fuzzing_no_drop, fuzzing_with_drop},
    happy::happy,
    setup::instances,
};
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
fn test_fuzzing_no_drop() {
    fuzzing_no_drop();
}

#[test]
fn test_fuzzing_drop() {
    fuzzing_with_drop();
}
