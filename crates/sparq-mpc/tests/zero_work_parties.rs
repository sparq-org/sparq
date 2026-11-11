use sparq_mpc::{plan_hidden_bounded_path, HiddenPathRequest};

#[test]
fn empty_planner_accepts_5000_parties_without_slots() {
    let plan = plan_hidden_bounded_path(&HiddenPathRequest::range("p", 1, 3), 0).unwrap();
    let counter = plan.comm_counter(5000, 0).unwrap();
    assert_eq!(counter.parties, 5000);
    assert_eq!(counter.multiplications, 0);
    assert_eq!(counter.field_elements_shared, 0);
    assert_eq!(counter.field_elements_opened, 0);
}
