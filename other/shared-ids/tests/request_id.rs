use shared_ids::RequestId;

#[test]
fn ord() {
    let ord = [
        None,
        Some(RequestId::from_u64(0)),
        Some(RequestId::from_u64(1)),
        Some(RequestId::from_u64(u64::MAX)),
    ];

    for i in 0..ord.len() {
        for j in 0..ord.len() {
            assert_eq!(i.cmp(&j), ord[i].cmp(&ord[j]))
        }
    }
}
