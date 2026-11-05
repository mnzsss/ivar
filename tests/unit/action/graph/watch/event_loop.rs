use super::*;

#[test]
fn the_leader_reindexes_on_its_own_five_second_clock() {
    let start = Instant::now();
    let mut last = None;
    assert!(reindex_due(&mut last, start));
    assert!(!reindex_due(&mut last, start + Duration::from_secs(4)));
    assert!(reindex_due(&mut last, start + Duration::from_secs(5)));
}
