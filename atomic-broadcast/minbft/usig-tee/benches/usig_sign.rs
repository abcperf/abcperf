use criterion::{black_box, criterion_group, criterion_main, Criterion};
use usig::Usig;
use usig_tee::UsigTEE;

fn criterion_benchmark(c: &mut Criterion) {
    let mut noop = usig::noop::UsigNoOp::default();
    let mut ed25519 = usig::signature::new_ed25519();
    let mut tee = UsigTEE::init_panic();

    let mut group = c.benchmark_group("usig");

    group.bench_function("noop", |b| b.iter(|| noop.sign(black_box(&[0; 512]))));
    group.bench_function("ed25519", |b| b.iter(|| ed25519.sign(black_box(&[0; 512]))));
    group.bench_function("tee", |b| b.iter(|| tee.sign(black_box(&[0; 512]))));

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
