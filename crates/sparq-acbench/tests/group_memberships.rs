//! Exported membership evidence is deterministic and reflects the generated groups.

use std::collections::BTreeSet;

use sparq_acbench::{consortium, personal, project_mgmt, GenParams};

#[test]
fn group_closures_are_deterministic_for_fixed_seeds() {
    for seed in [0, 42, 1234] {
        let mut params = GenParams::smoke();
        params.seed = seed;
        params.group_nesting_depth = 4;
        assert_eq!(
            personal::generate(&params).group_memberships,
            personal::generate(&params).group_memberships
        );
        assert_eq!(
            project_mgmt::generate(&params).group_memberships,
            project_mgmt::generate(&params).group_memberships
        );
        assert_eq!(
            consortium::generate(&params).group_memberships,
            consortium::generate(&params).group_memberships
        );
    }
}

#[test]
fn personal_closure_includes_nested_agents_without_sibling_or_parent_leakage() {
    let mut params = GenParams::smoke();
    params.group_nesting_depth = 3;
    params.members_per_group = 2;
    let ds = personal::generate(&params);
    for (level, first) in [(0, 0), (1, 2), (2, 4)] {
        let expected: BTreeSet<String> = (first..6)
            .map(|i| format!("https://pod.example/agent{i}/profile/card#me"))
            .collect();
        assert_eq!(
            ds.group_memberships[&format!("https://pod.example/user42/groups/level{level}/")],
            expected
        );
    }
    assert_eq!(ds.group_memberships.len(), 3);
}

#[test]
fn project_groups_export_owner_roles_and_cross_org_members() {
    let ds = project_mgmt::generate(&GenParams::smoke());
    let base = "https://bench.sparq.dev/pm";
    let owners = &ds.group_memberships[&format!("{base}/groups/org/0/owner")];
    assert!(owners.contains(&format!("{base}/agents/org/0/owner")));
    assert!(!owners.contains(&format!("{base}/agents/org/0/agent/0")));
    assert_eq!(owners.len(), 4);
    let expected: BTreeSet<String> = [0, 1]
        .into_iter()
        .flat_map(|org| {
            [0, 1]
                .into_iter()
                .map(move |member| format!("{base}/agents/org/{org}/agent/{member}"))
        })
        .collect();
    assert_eq!(
        ds.group_memberships[&format!("{base}/groups/subcontractor/0-1")],
        expected
    );
    assert_eq!(ds.group_memberships.len(), 7);
}

#[test]
fn consortium_closure_exports_exact_members_without_prefix_inference() {
    let ds = consortium::generate(&GenParams::smoke());
    let members = &ds.group_memberships["https://consortium.example/inst0/members"];
    let expected: BTreeSet<String> = (0..4)
        .map(|i| format!("https://consortium.example/agents/inst0-m{i}"))
        .collect();
    assert_eq!(members, &expected);
    assert!(!members.contains("https://consortium.example/agents/inst0-m999"));
    assert!(!members.contains("https://consortium.example/agents/inst1-m0"));
    assert_eq!(ds.group_memberships.len(), 3);
}
