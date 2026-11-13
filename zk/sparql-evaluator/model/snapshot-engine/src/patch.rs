/// Applies the V1 snapshot numeric bound with exact source-context matching.
/// Changed or ambiguous consumers require explicit review. Widening belongs
/// to a future dialect/version; source fragments are never matched fuzzily.
pub(crate) fn v1_snapshot_numeric_bound(mut source: String, patch: &str) -> String {
    let patch = patch
        .strip_prefix("@@ ")
        .expect("numeric patch must contain hunks");
    for hunk in patch.split("\n@@ ") {
        let (_, lines) = hunk.split_once('\n').expect("numeric patch hunk header");
        let (mut before, mut after) = (String::new(), String::new());
        for line in lines.lines() {
            let (before_line, after_line) = match line.as_bytes().first() {
                Some(b' ') => (true, true),
                Some(b'-') => (true, false),
                Some(b'+') => (false, true),
                _ => panic!("unexpected numeric patch line"),
            };
            if before_line {
                before.push_str(&line[1..]);
                before.push('\n');
            }
            if after_line {
                after.push_str(&line[1..]);
                after.push('\n');
            }
        }
        assert!(!before.is_empty(), "numeric patch needs source context");
        assert_eq!(
            source.matches(&before).count(),
            1,
            "V1 numeric consumer drift"
        );
        source = source.replacen(&before, &after, 1);
    }
    source
}

#[cfg(test)]
mod tests {
    use super::v1_snapshot_numeric_bound;

    #[test]
    fn exact_hunks_preserve_surrounding_source() {
        let patch = "@@ -2 +2 @@ example\n-old\n+new\n@@ -4 +4 @@ other\n-last\n+end\n";
        assert_eq!(
            v1_snapshot_numeric_bound("first\nold\nmiddle\nlast\n".into(), patch),
            "first\nnew\nmiddle\nend\n"
        );
    }

    #[test]
    #[should_panic(expected = "V1 numeric consumer drift")]
    fn changed_consumer_requires_review() {
        v1_snapshot_numeric_bound("changed\n".into(), "@@ -1 +1 @@\n-old\n+new\n");
    }

    #[test]
    #[should_panic(expected = "V1 numeric consumer drift")]
    fn ambiguous_consumer_requires_review() {
        v1_snapshot_numeric_bound("old\nold\n".into(), "@@ -1 +1 @@\n-old\n+new\n");
    }
}
