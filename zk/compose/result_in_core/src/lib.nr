// Research grade and not externally audited.
// Public variable/list metadata selects membership over already authenticated private values.
// Cost grows with rows times list capacity; credential authentication stays in result_m5.
pub fn membership_check<let P: u32, let V: u32, let R: u32>(
    pattern_count: u32,
    pattern_vars: [[u32; 3]; P],
    projection: [bool; V],
    result_count: u32,
    values: [[Field; V]; R],
    variable: u32,
    count: u32,
    constants: [Field; 16],
) {
    assert((variable > 0) & (variable <= V), "IN variable capacity");
    assert(!projection[variable - 1], "IN member requires hidden operand");
    assert((count > 0) & (count <= 16), "IN list capacity");
    let mut bound = false;
    for p in 0..P {
        for s in 0..3 {
            bound = bound | ((p < pattern_count) & (pattern_vars[p][s] == variable));
        }
    }
    assert(bound, "unbound IN variable");
    for i in 0..16 {
        assert((i < count) | (constants[i] == 0), "noncanonical IN padding");
    }
    for r in 0..R {
        let mut matched = false;
        for i in 0..16 {
            matched = matched | ((i < count) & (values[r][variable - 1] == constants[i]));
        }
        assert((r >= result_count) | matched, "IN membership is false");
    }
}

fn check(value: Field, count: u32, constants: [Field; 16]) {
    let mut values = [[0; 6]; 4];
    values[0][1] = value;
    membership_check::<3, 6, 4>(1, [[1, 0, 2], [0; 3], [0; 3]], [true, false, false, false, false, false], 1, values, 2, count, constants);
}

#[test]
fn both_members_match_without_projecting_the_operand() {
    let mut constants = [0; 16];
    constants[0] = 11;
    constants[1] = 22;
    check(11, 2, constants);
    check(22, 2, constants);
}

#[test(should_fail_with = "IN membership is false")]
fn nonmember_refuses() {
    let mut constants = [0; 16];
    constants[0] = 11;
    check(22, 1, constants);
}

#[test(should_fail_with = "noncanonical IN padding")]
fn inactive_list_entries_are_padding() {
    let mut constants = [0; 16];
    constants[0] = 11;
    constants[1] = 22;
    check(11, 1, constants);
}

#[test(should_fail_with = "IN list capacity")]
fn empty_list_is_refused() {
    check(11, 0, [0; 16]);
}

#[test(should_fail_with = "unbound IN variable")]
fn operand_must_occur_in_an_active_pattern() {
    let mut constants = [0; 16];
    constants[0] = 11;
    membership_check::<3, 6, 4>(1, [[1, 0, 0], [0; 3], [0; 3]], [false; 6], 1, [[11; 6]; 4], 2, 1, constants);
}
