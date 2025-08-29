use hashbar::Hashbar;
use id_set::ReplicaSet;
use nxbft_base::vertex::Vertex;
use shared_ids::ReplicaId;

use nxbft::{consensus_dag::Dag, DagAddress, Round};

#[test]
#[should_panic]
fn invalid_replica_id() {
    let dag = Dag::<(), ()>::new(5);
    let _ = dag.get(DagAddress::new(4.into(), ReplicaId::from_u64(7)));
}

#[test]
fn empty_dag() {
    let dag = Dag::<(), ()>::new(5);
    assert!(dag
        .get(DagAddress::new(4.into(), ReplicaId::from_u64(3)))
        .is_none());
}

#[test]
#[should_panic]
fn unknown_deliver() {
    let mut dag = Dag::<(), ()>::new(5);
    let _ = dag.deliver_from(
        DagAddress::new(4.into(), ReplicaId::from_u64(2)),
        Round::from(0),
    );
}

#[derive(Debug, PartialEq, Eq)]
struct U32Hash(i32, i32);

impl Hashbar for U32Hash {
    fn hash<H: sha2::digest::Update>(&self, hasher: &mut H) {
        hasher.update(&self.0.to_le_bytes());
        hasher.update(&self.1.to_le_bytes());
    }
}

fn make_vertex(
    round: impl Into<Round>,
    by: u64,
    edges: Vec<u64>,
    decision: i32,
    deliver_count: i32,
) -> Vertex<U32Hash, ()> {
    let mut set = ReplicaSet::default();
    for e in edges.into_iter() {
        set.insert(ReplicaId::from_u64(e));
    }
    Vertex::new(
        round.into(),
        ReplicaId::from_u64(by),
        set,
        vec![U32Hash(decision, deliver_count)],
        |_| Ok::<_, ()>(((), 0)),
    )
    .unwrap()
}

#[test]
#[should_panic]
fn insert_twice() {
    let mut dag = Dag::<U32Hash, ()>::new(5);
    dag.new_round();
    dag.insert(make_vertex(1, 0, vec![], 1, 4));
    dag.insert(make_vertex(1, 0, vec![1], 1, 8));
}

fn build_example_dag() -> Dag<U32Hash, ()> {
    let mut dag = Dag::<U32Hash, ()>::new(5);
    assert!(!dag.knows(DagAddress::new(1.into(), ReplicaId::from_u64(0))));
    dag.new_round();
    dag.insert(make_vertex(1, 0, vec![], 1, 4));
    assert!(dag.knows(DagAddress::new(1.into(), ReplicaId::from_u64(0))));
    dag.insert(make_vertex(1, 1, vec![], 1, 7));
    dag.insert(make_vertex(1, 2, vec![], 1, 11));
    dag.insert(make_vertex(1, 3, vec![], 1, 15));
    dag.insert(make_vertex(1, 4, vec![], 2, 22));
    dag.new_round();
    dag.insert(make_vertex(2, 0, vec![0, 1, 3], 1, 3));
    dag.insert(make_vertex(2, 1, vec![0, 1, 3], 1, 6));
    dag.insert(make_vertex(2, 2, vec![0, 2, 3], 1, 10));
    dag.insert(make_vertex(2, 3, vec![1, 2, 3], 1, 14));
    dag.insert(make_vertex(2, 4, vec![1, 3, 4], 2, 21));
    dag.new_round();
    dag.insert(make_vertex(3, 0, vec![0, 1, 2], 1, 2));
    dag.insert(make_vertex(3, 1, vec![0, 1, 2], 1, 5));
    dag.insert(make_vertex(3, 2, vec![0, 1, 2], 1, 9));
    dag.insert(make_vertex(3, 3, vec![0, 2, 3], 1, 13));
    dag.insert(make_vertex(3, 4, vec![2, 3, 4], 2, 20));
    dag.new_round();
    dag.insert(make_vertex(4, 0, vec![0, 1, 2], 2, 5));
    dag.insert(make_vertex(4, 1, vec![0, 1, 2], 1, 1));
    dag.insert(make_vertex(4, 2, vec![0, 2, 3], 1, 8));
    dag.insert(make_vertex(4, 3, vec![1, 2, 3], 1, 12));
    dag.insert(make_vertex(4, 4, vec![2, 3, 4], 2, 19));
    dag.new_round();
    dag.insert(make_vertex(5, 0, vec![0, 1, 2], 2, 4));
    dag.insert(make_vertex(5, 1, vec![0, 1, 2], 2, 9));
    dag.insert(make_vertex(5, 2, vec![0, 1, 2], 2, 12));
    dag.insert(make_vertex(5, 3, vec![1, 2, 3], 1, 0));
    dag.insert(make_vertex(5, 4, vec![2, 3, 4], 2, 18));
    dag.new_round();
    dag.insert(make_vertex(6, 0, vec![0, 1, 2], 2, 3));
    dag.insert(make_vertex(6, 1, vec![0, 1, 2], 2, 8));
    dag.insert(make_vertex(6, 2, vec![0, 1, 2], 2, 11));
    dag.insert(make_vertex(6, 3, vec![1, 2, 3], 2, 15));
    dag.insert(make_vertex(6, 4, vec![2, 3, 4], 2, 17));
    dag.new_round();
    dag.insert(make_vertex(7, 0, vec![0, 1, 2], 2, 2));
    dag.insert(make_vertex(7, 1, vec![0, 1, 2], 2, 7));
    dag.insert(make_vertex(7, 2, vec![0, 1, 2], 2, 10));
    dag.insert(make_vertex(7, 3, vec![1, 2, 3], 2, 14));
    dag.insert(make_vertex(7, 4, vec![2, 3, 4], 2, 16));
    dag.new_round();
    dag.insert(make_vertex(8, 0, vec![0, 1, 2], 2, 2));
    dag.insert(make_vertex(8, 1, vec![0, 1, 2], 2, 7));
    dag.insert(make_vertex(8, 2, vec![1, 2, 3], -1, -1));
    dag.insert(make_vertex(8, 3, vec![1, 2, 3], -1, -1));
    dag.insert(make_vertex(8, 4, vec![2, 3, 4], 2, 13));
    dag.new_round();
    dag.insert(make_vertex(9, 0, vec![0, 1, 4], 2, 0));
    dag.insert(make_vertex(9, 1, vec![0, 1, 2], -1, -1));
    dag.insert(make_vertex(9, 4, vec![2, 3, 4], -1, -1));
    dag
}

#[test]
fn dag_structure() {
    let mut dag = build_example_dag();
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(8.into(), ReplicaId::from_u64(2))
    ));
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(8.into(), ReplicaId::from_u64(3))
    ));
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(8.into(), ReplicaId::from_u64(4))
    ));
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(7.into(), ReplicaId::from_u64(1))
    ));
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(6.into(), ReplicaId::from_u64(0))
    ));
    assert!(!dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(7.into(), ReplicaId::from_u64(0))
    ));
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(0)),
        DagAddress::new(1.into(), ReplicaId::from_u64(0))
    ));
    assert!(!dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(1)),
        DagAddress::new(1.into(), ReplicaId::from_u64(4))
    ));
    assert_eq!(dag.get_round_valid_count(9.into()), 3);
    assert_eq!(dag.get_round_valid_count(8.into()), 5);
    assert_eq!(dag.get_round_valid_count(1.into()), 5);
    assert_eq!(
        dag.get(dag.get_round(9.into()).next().unwrap())
            .unwrap()
            .transactions()
            .first()
            .unwrap(),
        &U32Hash(2, 0)
    );
    assert_eq!(
        dag.get(dag.get_round(6.into()).nth(2).unwrap())
            .unwrap()
            .transactions()
            .first()
            .unwrap(),
        &U32Hash(2, 11)
    );
    let mut dag = build_example_dag();
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(8.into(), ReplicaId::from_u64(2))
    ));
    let mut dag = build_example_dag();
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(8.into(), ReplicaId::from_u64(3))
    ));
    let mut dag = build_example_dag();
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(8.into(), ReplicaId::from_u64(4))
    ));
    let mut dag = build_example_dag();
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(7.into(), ReplicaId::from_u64(1))
    ));
    let mut dag = build_example_dag();
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(6.into(), ReplicaId::from_u64(0))
    ));
    let mut dag = build_example_dag();
    assert!(!dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(4)),
        DagAddress::new(7.into(), ReplicaId::from_u64(0))
    ));
    let mut dag = build_example_dag();
    assert!(dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(0)),
        DagAddress::new(1.into(), ReplicaId::from_u64(0))
    ));
    let mut dag = build_example_dag();
    assert!(!dag.has_path(
        DagAddress::new(9.into(), ReplicaId::from_u64(1)),
        DagAddress::new(1.into(), ReplicaId::from_u64(4))
    ));
    // TODO: Check reaches of vertices
}

#[test]
fn deliver_and_prune() {
    let mut dag = build_example_dag();
    let delivered = dag.deliver_from(DagAddress::new(5.into(), ReplicaId::from_u64(3)), 0.into());
    assert_eq!(delivered.len(), 16);
    assert_eq!(
        delivered.first().unwrap(),
        &DagAddress::new(5.into(), ReplicaId::from_u64(3))
    );
    assert_eq!(
        delivered.get(8).unwrap(),
        &DagAddress::new(1.into(), ReplicaId::from_u64(0))
    );
    assert_eq!(
        delivered.last().unwrap(),
        &DagAddress::new(4.into(), ReplicaId::from_u64(1))
    );
    let undelivered = dag.prune(5.into());
    assert_eq!(undelivered.0.as_u64(), 1);
    assert_eq!(undelivered.1.as_u64(), 5);
    assert_eq!(undelivered.2.len(), 5);
    assert!(undelivered
        .2
        .iter()
        .any(|v| v.address() == DagAddress::new(4.into(), ReplicaId::from_u64(0))));
}

#[test]
fn deliver_twice_and_prune() {
    let mut dag = build_example_dag();
    let _ = dag.deliver_from(DagAddress::new(5.into(), ReplicaId::from_u64(3)), 0.into());
    let delivered = dag.deliver_from(DagAddress::new(9.into(), ReplicaId::from_u64(0)), 0.into());
    assert_eq!(delivered.len(), 23);
    assert_eq!(
        delivered.last().unwrap(),
        &DagAddress::new(8.into(), ReplicaId::from_u64(0))
    );
    let undelivered = dag.prune(10.into());
    assert_eq!(undelivered.0.as_u64(), 1);
    assert_eq!(undelivered.1.as_u64(), 10);
    assert_eq!(undelivered.2.len(), 4);
    assert!(!undelivered
        .2
        .iter()
        .any(|v| v.address() == DagAddress::new(4.into(), ReplicaId::from_u64(0))));
    assert!(undelivered
        .2
        .iter()
        .any(|v| v.address() == DagAddress::new(8.into(), ReplicaId::from_u64(2))));
    assert_eq!(dag.get_round_valid_count(1.into()), 0);
    assert_eq!(dag.get_round_valid_count(9.into()), 0);
    assert_eq!(dag.get_round(0.into()).count(), 0);
    assert_eq!(dag.get_round(9.into()).count(), 0);
}

#[test]
fn prune_between_delivers() {
    let mut dag = build_example_dag();
    let _ = dag.deliver_from(DagAddress::new(5.into(), ReplicaId::from_u64(3)), 0.into());
    let _ = dag.prune(5.into());
    let delivered = dag.deliver_from(DagAddress::new(9.into(), ReplicaId::from_u64(0)), 5.into());
    assert_eq!(delivered.len(), 18);
    assert_eq!(
        delivered.last().unwrap(),
        &DagAddress::new(8.into(), ReplicaId::from_u64(0))
    );
    let undelivered = dag.prune(10.into());
    assert_eq!(undelivered.0.as_u64(), 5);
    assert_eq!(undelivered.1.as_u64(), 10);
    assert_eq!(undelivered.2.len(), 4);
    assert!(!undelivered
        .2
        .iter()
        .any(|v| v.address() == DagAddress::new(4.into(), ReplicaId::from_u64(0))));
    assert!(undelivered
        .2
        .iter()
        .any(|v| v.address() == DagAddress::new(8.into(), ReplicaId::from_u64(2))));
}

#[test]
fn test_dfs() {
    let mut dag = build_example_dag();
    let delivered = dag.deliver_from(DagAddress::new(1.into(), ReplicaId::from_u64(1)), 0.into());
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        *delivered.first().unwrap(),
        DagAddress::new(1.into(), ReplicaId::from_u64(1))
    );
    let delivered = dag.deliver_from(DagAddress::new(5.into(), ReplicaId::from_u64(3)), 0.into());
    assert_eq!(delivered.len(), 15);
    assert_eq!(
        *delivered.first().unwrap(),
        DagAddress::new(5.into(), ReplicaId::from_u64(3))
    );
    assert_eq!(
        *delivered.last().unwrap(),
        DagAddress::new(4.into(), ReplicaId::from_u64(1))
    );
    let delivered = dag.deliver_from(DagAddress::new(9.into(), ReplicaId::from_u64(0)), 0.into());
    assert_eq!(delivered.len(), 23);
    assert_eq!(
        *delivered.first().unwrap(),
        DagAddress::new(9.into(), ReplicaId::from_u64(0))
    );
    assert_eq!(
        *delivered.last().unwrap(),
        DagAddress::new(8.into(), ReplicaId::from_u64(0))
    );
}
