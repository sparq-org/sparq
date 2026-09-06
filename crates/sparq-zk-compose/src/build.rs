// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! Builds circuit inputs + witnesses from sparq-zk commitments.
//!
//! The (k, n, r) circuit-family id is DERIVED from the data: `k` = number of
//! committed graphs, `n` = smallest compiled slot-bucket >= the largest
//! graph, `r` = smallest compiled row-bucket >= the disclosed match count.
//! Prover and verifier both call [`derive_scan_id`] so a proof can only fit
//! its member (brief: "derive the circuit-family id from the proof manifest").

use crate::manifest::{CircuitId, FieldHex, FilterOp, ProofInputs};
use sparq_zk::commit::GraphCommitment;
use sparq_zk::encode::encode_term;
use sparq_zk::field::{field_to_be_bytes_32, field_to_hex, Fr};
use sparq_zk::sig::join_value_commitment;
use oxrdf::{NamedNode, Term};

/// Compiled slot buckets (`n`) of the scan family, ascending.
pub const SCAN_N_BUCKETS: &[u32] = &[16, 64];
/// Compiled row buckets (`r`) of the scan family, ascending.
pub const SCAN_R_BUCKETS: &[u32] = &[4, 8];
/// Compiled `k` values of the scan family.
pub const SCAN_K_VALUES: &[u32] = &[1, 2];
/// Compiled digit counts (`d`) of the filter_int family. The `filter_int_d{D}`
/// circuit's `digits: [u8; D]` witness requires the operand's decimal digit
/// count to EXACTLY equal `D` (see `compose_core::filter_int::filter_int_check`),
/// so a member is provable for an operand iff its digit count == `D`. v1 compiles
/// the contiguous range 1..=4 plus the historical gap-fill at 3 (sq-wto).
// [OPUS-4.8] sq-wto: 3 added so the {1,2,4} family is contiguous 1..=4 and no
// in-range digit count derives an unprovable wrong-D member.
pub const FILTER_INT_D_VALUES: &[u32] = &[1, 2, 3, 4];
/// Compiled digit counts (`d`) of the MANIFEST-COMPOSABLE filter_f64 family
/// ([OPUS-4.8] sq-q7e / sq-tat). Same EXACT-match discipline as `filter_int`:
/// `filter_f64_d{D}`'s `digits: [u8; D]` witness pins the operand's digit count
/// to `D`, so a member is provable iff the integer-valued double's decimal digit
/// count equals `D`. v1 compiles the contiguous 1..=4; `D <= 15` is the soundness
/// ceiling (value `< 2^53`, exact `f64::from`), so larger counts have no member
/// and `derive_filter_f64_id` returns `None` (clean error, never a wrong-D).
pub const FILTER_F64_D_VALUES: &[u32] = &[1, 2, 3, 4];
/// Compiled MAGNITUDE-digit counts (`md`) of the MANIFEST-COMPOSABLE
/// `filter_signed_int` family ([OPUS-4.8] sq-7lrq, the sq-1q9h members). Same
/// EXACT-match discipline as `filter_int` / `filter_f64`: the
/// `filter_signed_int_d{MD}` circuit's `mag_digits: [u8; MD]` witness pins the
/// operand's magnitude-digit count to `MD` (`filter_signed_int_check::<MD>` rebuilds
/// the canonical `"[-]?<digits>"^^<…#integer>` token from exactly `MD` digits), so a
/// member is provable for an operand iff its magnitude-digit count == `MD`. v1
/// compiles `{2, 4}` (the two members sq-1q9h landed: 2-digit negative coordinates
/// and 4-digit signed amounts). An out-of-family magnitude-digit count returns
/// `None` from [`derive_filter_signed_int_id`] (clean error, never a wrong-MD
/// silently-unprovable member — sq-wto). `MD <= 19` is the `u64`-magnitude ceiling.
pub const FILTER_SIGNED_INT_MD_VALUES: &[u32] = &[2, 4];
/// Compiled `(id, fd)` integer-/fraction-digit counts of the MANIFEST-COMPOSABLE
/// `filter_decimal` family ([OPUS-4.8] sq-7lrq, the sq-1q9h member). The
/// `filter_decimal_i{ID}_f{FD}` circuit pins the operand's integer-digit count to
/// `ID` AND its fraction-digit count to `FD` (the `int_digits: [u8; ID]` /
/// `frac_digits: [u8; FD]` witnesses), so a `(id, fd)`-shaped operand is provable
/// ONLY by the `(ID, FD) == (id, fd)` member. v1 compiles `(3, 2)` (the
/// `"123.45"`-shape member sq-1q9h landed); an out-of-family `(id, fd)` returns
/// `None` from [`derive_filter_decimal_id`] (clean error, never a wrong-shape member).
pub const FILTER_DECIMAL_ID_FD_VALUES: &[(u32, u32)] = &[(3, 2)];
/// Compiled graph-size buckets (`n_a`/`n_b`) of the hidden-join `join_eq` family,
/// ascending (sq-bwwl / sq-fi03). The `join_eq_na{N_A}_nb{N_B}` member re-commits
/// each witnessed graph with `[[Field; 3]; N]` slots (the `join_eq_check<N_A,
/// N_B>` const generics), so a graph holding `<= N` triples is provable by the
/// smallest compiled bucket `>= N` — exactly the scan `n`-bucket discipline. The
/// [`SCAN_N_BUCKETS`]-aligned `{16, 64}` set is compiled, in all four `(n_a, n_b)`
/// combinations (`na16_nb16`, `na16_nb64`, `na64_nb16`, `na64_nb64`), so a join
/// can compose with a scan over either scan-`n` bucket; an out-of-family size
/// (`> 64`) derives `None` (a clean error, never a wrong-N member). The verifier
/// gate that consumes the derived id is `bind_joins` (step 4, sq-sfsi).
// [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): compiled join_eq graph-size buckets.
// [OPUS-4.8] sq-pzet (sq-8dx2): add the `64` bucket — the remaining (na, nb)
// members so a hidden join composes with the n=64 scan bucket. NOT a soundness
// change (the verifier remains NOT-yet-sound, sq-qhy4 / sq-9hrn / sq-1s2): the
// in-circuit relation is `join_eq_check` verbatim, only the bucket grows.
pub const JOIN_EQ_N_BUCKETS: &[u32] = &[16, 64];

fn smallest_bucket(buckets: &[u32], need: u32) -> Option<u32> {
    buckets.iter().copied().find(|&b| b >= need)
}

/// Derive the scan circuit id for `k` graphs whose largest holds `max_graph`
/// triples, disclosing `match_rows` rows. `None` if no compiled member fits.
pub fn derive_scan_id(k: u32, max_graph: u32, match_rows: u32) -> Option<CircuitId> {
    if !SCAN_K_VALUES.contains(&k) {
        return None;
    }
    let n = smallest_bucket(SCAN_N_BUCKETS, max_graph.max(1))?;
    let r = smallest_bucket(SCAN_R_BUCKETS, match_rows.max(1))?;
    Some(CircuitId::Scan { k, n, r })
}

/// Derive the filter_int circuit id for a `digits`-digit value.
///
/// # sq-wto correctness (EXACT match, not smallest-bucket-≥)
/// The `filter_int_d{D}` circuit pins the operand's digit count to `D` exactly:
/// its witness is `digits: [u8; D]` and `filter_int_check::<D>` rebuilds the
/// canonical token from exactly `D` digit bytes, so a `d`-digit operand is
/// provable ONLY by the `D == d` member. The old `smallest_bucket(.., digits)`
/// (smallest compiled `D >= digits`) was a COMPLETENESS BUG (sq-61g differential
/// fuzzer / sq-wto): a 3-digit operand derived `d = 4`, but no 3-digit value can
/// fill a `[u8; 4]` witness, so `nargo execute` produced no witness — an honest
/// FILTER that SILENTLY yields an unprovable statement. We now require an EXACT
/// compiled member and return `None` for any digit count with no `D == digits`
/// member (out-of-family => a CLEAN error at the call site, never a
/// silently-unprovable d). With the d3 member the provable range is the
/// contiguous 1..=4; 0 maps to the 1-digit canonical form ("0"); 5..=19 (and
/// >19, which overflows u64) have no member and return `None`.
// [OPUS-4.8] sq-wto: exact digit-count match; out-of-bucket => None (clean
// error), never a wrong-D silently-unprovable member.
pub fn derive_filter_int_id(digits: u32) -> Option<CircuitId> {
    let d = digits.max(1);
    if FILTER_INT_D_VALUES.contains(&d) {
        Some(CircuitId::FilterInt { d })
    } else {
        None
    }
}

/// Derive the MANIFEST-COMPOSABLE filter_f64 circuit id for a `digits`-digit
/// integer-valued double ([OPUS-4.8] sq-q7e / sq-tat). EXACT-match discipline,
/// identical to [`derive_filter_int_id`]: the `filter_f64_d{D}` circuit pins the
/// operand's digit count to `D` (`digits: [u8; D]`), so only the `D == digits`
/// member is provable. Out-of-family counts (no compiled `D == digits`, incl. the
/// `>15` soundness ceiling) return `None` — a clean error, never a wrong-D
/// silently-unprovable member.
// [OPUS-4.8] sq-q7e + sq-tat: exact digit-count match for the composable f64 family.
pub fn derive_filter_f64_id(digits: u32) -> Option<CircuitId> {
    let d = digits.max(1);
    if FILTER_F64_D_VALUES.contains(&d) {
        Some(CircuitId::FilterF64 { d })
    } else {
        None
    }
}

/// Derive the MANIFEST-COMPOSABLE `filter_signed_int` circuit id for a
/// `mag_digits`-magnitude-digit SIGNED xsd:integer operand ([OPUS-4.8] sq-7lrq).
/// EXACT-match discipline, identical to [`derive_filter_int_id`]: the
/// `filter_signed_int_d{MD}` circuit pins the operand's MAGNITUDE-digit count to
/// `MD` (`mag_digits: [u8; MD]`; the optional leading `-` is bound into the token
/// but not counted in `MD`), so only the `MD == mag_digits` member is provable.
/// Out-of-family counts (no compiled `MD == mag_digits`) return `None` — a clean
/// error, never a wrong-MD silently-unprovable member (sq-wto).
// [OPUS-4.8] sq-7lrq: exact magnitude-digit match for the composable signed-int family.
pub fn derive_filter_signed_int_id(mag_digits: u32) -> Option<CircuitId> {
    let md = mag_digits.max(1);
    if FILTER_SIGNED_INT_MD_VALUES.contains(&md) {
        Some(CircuitId::FilterSignedInt { md })
    } else {
        None
    }
}

/// Derive the MANIFEST-COMPOSABLE `filter_decimal` circuit id for an operand with
/// `int_digits` integer-part digits and `frac_digits` fraction digits ([OPUS-4.8]
/// sq-7lrq). EXACT-match discipline over BOTH counts: the `filter_decimal_i{ID}_f{FD}`
/// circuit pins the operand's integer-digit count to `ID` AND its fraction-digit
/// count to `FD` (`int_digits: [u8; ID]` / `frac_digits: [u8; FD]`), so only the
/// `(ID, FD) == (int_digits, frac_digits)` member is provable. The integer part is
/// clamped to at least one digit (canonical `0.xx` has the single integer digit
/// `"0"`); `frac_digits` is taken verbatim (a decimal's fraction count is fixed by
/// its lexical form, including trailing zeros, which `oxrdf` preserves). An
/// out-of-family `(id, fd)` returns `None` — a clean error, never a wrong-shape
/// member.
// [OPUS-4.8] sq-7lrq: exact (id, fd) digit-shape match for the composable decimal family.
pub fn derive_filter_decimal_id(int_digits: u32, frac_digits: u32) -> Option<CircuitId> {
    let id = int_digits.max(1);
    let fd = frac_digits;
    if FILTER_DECIMAL_ID_FD_VALUES.contains(&(id, fd)) {
        Some(CircuitId::FilterDecimal { id, fd })
    } else {
        None
    }
}

/// Derive the hidden-join `join_eq` circuit id for two graphs holding `n_a` /
/// `n_b` triples (sq-bwwl / sq-fi03). Mirrors [`derive_scan_id`]'s `n`-bucket
/// discipline: each side maps to the smallest compiled [`JOIN_EQ_N_BUCKETS`]
/// bucket `>= size` (the member re-commits `[[Field; 3]; N]` slots per graph, so
/// any graph with `<= N` triples fits the `N`-bucket member). `None` if either
/// side exceeds every compiled bucket — a clean error, never a wrong-N member.
/// Prover and verifier both derive the id this way so a `join_eq` proof can only
/// verify against the member its witnesses fit (audit-#2 canonical-vk discipline).
// [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): derive the join_eq member id.
pub fn derive_join_eq_id(n_a: u32, n_b: u32) -> Option<CircuitId> {
    let n_a = smallest_bucket(JOIN_EQ_N_BUCKETS, n_a.max(1))?;
    let n_b = smallest_bucket(JOIN_EQ_N_BUCKETS, n_b.max(1))?;
    Some(CircuitId::JoinEq { n_a, n_b })
}

/// The compiled `(d, k, n)` members of the bounded-depth `path_reach_d{d}_k{k}_n{n}`
/// family (sq-3kd2g.6 / the sq-3kd2g.2 circuits): depth bound `d`, committed-graph
/// count `k`, per-graph slot bucket `n`. This is the SINGLE source of the family
/// list — [`derive_path_reach_id`] validates an `(d, k, n)` triple against it
/// EXACTLY (no wrong-bucket fallback), mirroring the filter families' EXACT-match
/// discipline (sq-wto). The four members `zk/compose/` compiles today:
/// `(2,1,16)`, `(4,1,16)`, `(4,2,16)`, `(8,1,16)`.
// [OPUS-4.8] sq-3kd2g.6: compiled path_reach members. Opt-in (`extended-fragment`).
#[cfg(feature = "extended-fragment")]
pub const PATH_REACH_MEMBERS: &[(u32, u32, u32)] =
    &[(2, 1, 16), (4, 1, 16), (4, 2, 16), (8, 1, 16)];

/// Compiled slot buckets (`n`) of the path family, ascending (only `16` today).
// [OPUS-4.8] sq-3kd2g.6.
#[cfg(feature = "extended-fragment")]
pub const PATH_REACH_N_BUCKETS: &[u32] = &[16];

/// Derive the `path_reach_d{d}_k{k}_n{n}` member id for depth bound `d`, `k`
/// committed graphs, `n` slots/graph. EXACT membership against
/// [`PATH_REACH_MEMBERS`]: `(d, k, n)` must be a compiled member, else `None`
/// (fail-closed — a claim outside the family derives no id, never a wrong bucket).
///
/// `d` is the design record's normative "path depth `k`" (§4 req 1), constant in
/// the member's VK and re-stated as the public `depth_bound`. `k` = commitments
/// arity, `n` = slot bucket. Prover and verifier both call this so a proof only
/// fits the member its public inputs name.
// [OPUS-4.8] sq-3kd2g.6: derive the bounded-depth path member id (EXACT match).
#[cfg(feature = "extended-fragment")]
pub fn derive_path_reach_id(d: u32, k: u32, n: u32) -> Option<CircuitId> {
    if PATH_REACH_MEMBERS.contains(&(d, k, n)) {
        Some(CircuitId::PathReach { d, k, n })
    } else {
        None
    }
}

/// Pick the compiled path member for `k` graphs (largest holding `max_graph`
/// triples) that admits a chain of at least `min_depth` steps: the SMALLEST
/// compiled `d >= min_depth` with a matching `(k, n)`, where `n` is the smallest
/// slot bucket `>= max_graph`. `None` if no compiled member covers it (e.g. a
/// chain longer than every member's depth, or a graph larger than every bucket).
// [OPUS-4.8] sq-3kd2g.6.
#[cfg(feature = "extended-fragment")]
pub fn smallest_path_reach_id(k: u32, max_graph: u32, min_depth: u32) -> Option<CircuitId> {
    let n = smallest_bucket(PATH_REACH_N_BUCKETS, max_graph.max(1))?;
    PATH_REACH_MEMBERS
        .iter()
        .copied()
        .filter(|&(_, mk, mn)| mk == k && mn == n)
        .map(|(d, _, _)| d)
        .filter(|&d| d >= min_depth)
        .min()
        .map(|d| CircuitId::PathReach { d, k, n })
}

/// [OPUS-5] sq-kndw: the compiled `revoke_hidden_ref_d{depth}_a{set_depth}`
/// FULLY-HIDDEN revocation members, as `(status-list depth, accepted-set depth)`.
/// This is the SINGLE SOURCE of the family list —
/// [`derive_revoke_hidden_ref_id`] validates a pair against it EXACTLY (no
/// wrong-bucket fallback), mirroring the path/filter families' EXACT-match
/// discipline (sq-wto). One member `zk/compose/` compiles today: `(10, 4)`.
// [OPUS-5] sq-kndw: compiled fully-hidden revocation members.
pub const REVOKE_HIDDEN_REF_MEMBERS: &[(u32, u32)] = &[(10, 4)];

/// Derive the `revoke_hidden_ref_d{depth}_a{set_depth}` member id (sq-kndw).
/// EXACT membership against [`REVOKE_HIDDEN_REF_MEMBERS`]: the pair must be a
/// COMPILED member, else `None` — fail-closed, so a relying party that configures
/// a `(hidden_index_depth, accepted_set_depth)` combination with no compiled
/// circuit gets a clean refusal rather than a proof attempt against a member that
/// does not exist. Prover and verifier both call this, so a proof only ever fits
/// the member its public inputs name.
// [OPUS-5] sq-kndw: derive the fully-hidden revocation member id (EXACT match).
pub fn derive_revoke_hidden_ref_id(depth: u32, set_depth: u32) -> Option<CircuitId> {
    if REVOKE_HIDDEN_REF_MEMBERS.contains(&(depth, set_depth)) {
        Some(CircuitId::RevokeHiddenRef { depth, set_depth })
    } else {
        None
    }
}

/// A constant or variable slot of a BGP triple pattern.
#[derive(Debug, Clone)]
pub enum Slot {
    Const(Term),
    Var,
}

/// A single BGP triple pattern (subject, predicate, object slots).
#[derive(Debug, Clone)]
pub struct Pattern {
    pub s: Slot,
    pub p: Slot,
    pub o: Slot,
}

/// The witnesses a scan proof needs but the manifest never carries.
#[derive(Debug, Clone)]
pub struct ScanWitness {
    /// Per-graph active triple counts.
    pub counts: Vec<u32>,
    /// Per-graph per-slot term encodings (active leaves; build pads to N).
    pub enc: Vec<Vec<[FieldHex; 3]>>,
}

/// Built scan proof: the public inputs + the private witness.
#[derive(Debug, Clone)]
pub struct BuiltScan {
    pub inputs: ProofInputs,
    pub witness: ScanWitness,
}

fn hexf(f: &Fr) -> FieldHex {
    FieldHex(field_to_hex(f))
}

/// Encode a BGP slot to the (is_const, const_enc) pair the circuit takes, and
/// also return the encoded constant (for row matching).
fn encode_slot(slot: &Slot, salt: &Fr) -> (bool, FieldHex, Option<Fr>) {
    match slot {
        Slot::Const(t) => {
            let enc = encode_term(t, salt).expect("committable constant term");
            (true, hexf(&enc), Some(enc))
        }
        Slot::Var => (false, FieldHex("0x0".to_string()), None),
    }
}

/// Build a scan proof's inputs + witnesses from per-graph commitments.
///
/// All graphs must share the salt convention used to commit them; the encoded
/// leaves come straight from each [`GraphCommitment`]'s recorded leaves' source
/// terms. We re-encode each canonical triple's terms (the circuit checks
/// `h3(enc) == leaf`), so this re-derivation is sound by construction: a wrong
/// encoding would fail the in-circuit commitment recompute.
///
/// # Strict-ordering canonicalisation (plan S2.5, [OPUS-4.8])
/// `scan_check` enforces `commitments[0] < commitments[1] < ...` (strict, on the
/// canonical field representative) to force the committed graphs pairwise
/// distinct and close the duplicate-inclusion / COUNT-forgery class. So this
/// builder emits the commitments -- and, in lock-step, every per-graph witness
/// (`counts`, `enc`) and the public `attribution` vector -- sorted ASCENDING by
/// the canonical big-endian commitment bytes (the exact order the in-circuit
/// `Field::lt` / the bb public-input word use). Sorting here (rather than
/// trusting caller order) means an honest k>=2 scan always satisfies the gate
/// regardless of the order the caller supplied its graphs in. If the caller
/// supplies two graphs with the SAME commitment (a genuine duplicate), the sort
/// places them adjacent and the in-circuit `<` then rejects -- the gate's intent.
pub fn build_scan(
    commitments: &[GraphCommitment],
    pattern: &Pattern,
) -> Option<BuiltScan> {
    let k = commitments.len() as u32;
    // S2.5: canonicalise graph order ascending by the commitment's field
    // representative so the strict-ordering gate (`scan_check` step 1b) is
    // satisfied for any honest input order. Sort the SAME canonical big-endian
    // bytes the circuit compares (`field_to_be_bytes_32`), so the host order
    // and the in-circuit `Field::lt` order agree exactly.
    let mut order: Vec<usize> = (0..commitments.len()).collect();
    order.sort_by(|&a, &b| {
        field_to_be_bytes_32(&commitments[a].commitment)
            .cmp(&field_to_be_bytes_32(&commitments[b].commitment))
    });
    let commitments: Vec<&GraphCommitment> = order.iter().map(|&i| &commitments[i]).collect();
    // Per-graph term encodings of every canonical triple.
    let mut enc: Vec<Vec<[FieldHex; 3]>> = Vec::with_capacity(commitments.len());
    let mut enc_fr: Vec<Vec<[Fr; 3]>> = Vec::with_capacity(commitments.len());
    let mut counts = Vec::with_capacity(commitments.len());
    for c in &commitments {
        let salt = c.salt;
        let mut graph_hex = Vec::new();
        let mut graph_fr = Vec::new();
        for t in &c.canonical.triples {
            let s = encode_term(&subj_term(t), &salt)?;
            let p = encode_term(&Term::NamedNode(t.predicate.clone()), &salt)?;
            let o = encode_term(&t.object, &salt)?;
            graph_hex.push([hexf(&s), hexf(&p), hexf(&o)]);
            graph_fr.push([s, p, o]);
        }
        counts.push(graph_hex.len() as u32);
        enc.push(graph_hex);
        enc_fr.push(graph_fr);
    }

    // Pattern encoding.
    // Each graph shares no single salt for IRIs/literals (those are
    // salt-independent), so constant encodings are stable across graphs; use
    // graph 0's salt as the reference (bnodes are never query constants).
    let ref_salt = commitments.first().map(|c| c.salt).unwrap_or(Fr::from(0u64));
    let (sc, se, sf) = encode_slot(&pattern.s, &ref_salt);
    let (pc, pe, pf) = encode_slot(&pattern.p, &ref_salt);
    let (oc, oe, of) = encode_slot(&pattern.o, &ref_salt);
    let pattern_is_const = [sc, pc, oc];
    let pattern_const_enc = [se, pe, oe];
    let const_fr = [sf, pf, of];

    // Disclosed rows: every active slot matching the pattern's constants. While
    // sweeping, record per-graph source attribution (audit #8): `attribution[g]`
    // is true iff graph `g` contributes at least one matched triple — the exact
    // bit `scan.nr` step 4 constrains. This is the proof-bound provenance the
    // verifier cross-checks against `manifest.attributions`.
    // [OPUS-4.8] audit #8.
    let mut rows: Vec<[FieldHex; 3]> = Vec::new();
    let mut attribution: Vec<bool> = Vec::with_capacity(enc_fr.len());
    for graph in &enc_fr {
        let mut graph_matches = false;
        for triple in graph {
            let matches = const_fr.iter().enumerate().all(|(i, c)| match c {
                Some(cf) => triple[i] == *cf,
                None => true,
            });
            if matches {
                graph_matches = true;
                rows.push([hexf(&triple[0]), hexf(&triple[1]), hexf(&triple[2])]);
            }
        }
        attribution.push(graph_matches);
    }
    let row_count = rows.len() as u32;
    let max_graph = counts.iter().copied().max().unwrap_or(0);
    let id = derive_scan_id(k, max_graph, row_count)?;

    let commitments_hex: Vec<FieldHex> =
        commitments.iter().map(|c| hexf(&c.commitment)).collect();

    Some(BuiltScan {
        inputs: ProofInputs::Scan {
            id,
            commitments: commitments_hex,
            pattern_is_const,
            pattern_const_enc,
            rows,
            row_count,
            attribution,
        },
        witness: ScanWitness { counts, enc },
    })
}

/// Build a filter_int proof for a hidden xsd:integer operand `value` under
/// operator `op` against `bound`, with the disclosed `expected` verdict.
/// `operand_enc` must be the same term encoding the scan proof disclosed for
/// the operand column (binding consistency).
pub fn build_filter_int(
    operand_enc: FieldHex,
    value: u64,
    op: FilterOp,
    bound: u64,
    expected: bool,
) -> Option<(ProofInputs, Vec<u8>)> {
    let digits = value.to_string();
    let id = derive_filter_int_id(digits.len() as u32)?;
    let digit_bytes: Vec<u8> = digits.bytes().collect();
    Some((
        ProofInputs::FilterInt { id, operand_enc, op, bound, expected },
        digit_bytes,
    ))
}

/// The BOOLEAN-lane verdict for `value <op> bound` over the XSD boolean order
/// `false < true` (sq-5xdlk) — the host mirror of the `filter_value_dl_int`
/// member's `integer_verdict` applied to the boolean value hooks
/// `{0 = false, 1 = true}` (`zk/compose/compose_core/src/filter_value.nr`).
///
/// `EQ`/`NE` are the ordinary boolean equality; `LT`/`LE`/`GT`/`GE` are the
/// DEGENERATE orderings XPath's `op:boolean-less-than` / `op:boolean-greater-than`
/// define, i.e. exactly `false < true` — they are meaningful, not errors, which is
/// why the integer member serves this lane unchanged.
// [OPUS-5] sq-5xdlk: boolean value-lane verdict oracle. Opt-in (`dual-leaf`).
#[cfg(feature = "dual-leaf")]
pub fn boolean_verdict(value: bool, op: FilterOp, bound: bool) -> bool {
    match op {
        FilterOp::Lt => !value && bound,
        FilterOp::Le => !value || bound,
        FilterOp::Gt => value && !bound,
        FilterOp::Ge => value || !bound,
        FilterOp::Eq => value == bound,
        FilterOp::Ne => value != bound,
    }
}

/// A built DUAL-LEAF `xsd:boolean` value-lane FILTER (sq-5xdlk): the public
/// [`ProofInputs`] plus the member's two PRIVATE field witnesses.
///
/// The witnesses are the dual-leaf components `sparq_zk::dual_leaf_boolean::
/// encode_boolean` produced for the committed literal, so `inputs.operand_enc`
/// is by construction the leaf those witnesses rebind to in-circuit.
// [OPUS-5] sq-5xdlk: boolean value-lane host wiring. Opt-in, NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltFilterValueDlBoolean {
    /// Public inputs for the shared [`CircuitId::FilterValueDl`] member, carrying
    /// the BOOLEAN `datatype_const` and `bound ∈ {0, 1}`.
    pub inputs: ProofInputs,
    /// PRIVATE: the boolean `VALUE_HOOK` (`0` = false, `1` = true) as a field.
    pub value_hook: FieldHex,
    /// PRIVATE: the OFF-circuit blake3 lexical hash, carried as a free witness.
    pub lexical_component: FieldHex,
}

/// Build a DUAL-LEAF `xsd:boolean` value-lane FILTER over the committed literal
/// `literal` under operator `op` against the constant `bound` (sq-5xdlk — the
/// circuit half of the boolean lane whose host encoder is sq-hh7a4).
///
/// NO new Noir member is involved: this targets the EXISTING
/// [`CircuitId::FilterValueDl`] (`filter_value_dl_int`) with
/// `datatype_const = `[`crate::manifest::boolean_datatype_const`]`()` and the
/// boolean hooks `{0, 1}` inside its `u64` domain (see that function's docs for
/// why the lanes cannot cross).
///
/// The disclosed verdict is COMPUTED here ([`boolean_verdict`]) rather than taken
/// from the caller, so an honest host cannot accidentally disclose a verdict the
/// member will refuse to prove; a test constructing the LYING case builds the
/// [`ProofInputs::FilterValueDl`] variant directly with a flipped `expected`.
///
/// Returns the encoder's `Err` unchanged when `literal` is not a canonical
/// `xsd:boolean` (`"true"`/`"false"`): the §6 fail-closed co-binding rejects the
/// non-canonical XSD-legal spellings `"1"`/`"0"` and every non-boolean datatype,
/// so a desynced leaf is never built here.
///
/// DOCUMENTED RISK: inherits the value lane's INV-VL downgrade (#769 accepted,
/// CR-G8 / sq-qhy4). NOT externally audited; no soundness / privacy claim.
// [OPUS-5] sq-5xdlk: boolean value-lane host wiring. Opt-in, NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
pub fn build_filter_value_dl_boolean(
    literal: &oxrdf::Literal,
    op: FilterOp,
    bound: bool,
) -> Result<BuiltFilterValueDlBoolean, sparq_zk::dual_leaf::DualLeafError> {
    let components = sparq_zk::dual_leaf_boolean::encode_boolean(literal)?;
    // The encoder maps ONLY the canonical lexicals, so the hook is exactly {0, 1}.
    let value = components.value_hook != Fr::from(0u64);
    Ok(BuiltFilterValueDlBoolean {
        inputs: ProofInputs::FilterValueDl {
            id: CircuitId::FilterValueDl,
            operand_enc: hexf(&components.leaf()),
            op,
            bound: u64::from(bound),
            datatype_const: crate::manifest::boolean_datatype_const(),
            expected: boolean_verdict(value, op, bound),
        },
        value_hook: hexf(&components.value_hook),
        lexical_component: hexf(&components.lexical_component),
    })
}

/// The SIGN-AWARE scaled comparison verdict for `value <op> bound` over two
/// `(neg, magnitude)` operands on ONE scaled timeline (sq-wz99x) — the host
/// mirror of the Noir member's UNCHANGED `signed_scaled_verdict`
/// (`zk/compose/compose_core/src/filter_value.nr`). `(true, 0)` must be
/// normalised to `(false, 0)` by the caller (there is no `-0`).
///
/// On the `Z`-only hookable domain this IS the XSD `timeOnTimeline` order, so
/// all six operators are meaningful for `xsd:dateTime` / `xsd:date`.
// [OPUS-5] sq-wz99x: dateTime/date value-lane verdict oracle. Opt-in (`dual-leaf`).
#[cfg(feature = "dual-leaf")]
pub fn signed_epoch_verdict(v_neg: bool, v_mag: u64, b_neg: bool, b_mag: u64, op: FilterOp) -> bool {
    let eq = v_neg == b_neg && v_mag == b_mag;
    let lt = match (v_neg, b_neg) {
        (true, false) => true,
        (false, true) => false,
        // Both pre-epoch: the LARGER magnitude is the EARLIER instant.
        (true, true) => v_mag > b_mag,
        (false, false) => v_mag < b_mag,
    };
    match op {
        FilterOp::Lt => lt,
        FilterOp::Le => lt || eq,
        FilterOp::Gt => !lt && !eq,
        FilterOp::Ge => !lt,
        FilterOp::Eq => eq,
        FilterOp::Ne => !eq,
    }
}

/// Split an encoder-produced signed `VALUE_HOOK` field back into the
/// `(neg, magnitude)` pair the Noir member takes as `(value_neg,
/// value_hook_scaled)` (sq-wz99x).
///
/// The host encoder folds the sign by FIELD NEGATION (`if neg { -mag } else
/// { mag }`), which is exactly what the member recomputes, so this inverts that:
/// a hook that is already a `u64` is non-negative; otherwise its field negation
/// must be. `None` if NEITHER fits `u64` — impossible for encoder output (the
/// §13.4 predicate rejects a magnitude overflowing `u64`), so callers surface it
/// fail-closed rather than assuming.
#[cfg(feature = "dual-leaf")]
fn split_signed_hook(hook: &Fr) -> Option<(bool, u64)> {
    fn as_u64(f: &Fr) -> Option<u64> {
        let b = field_to_be_bytes_32(f);
        // A u64 occupies the low 8 big-endian bytes; anything above means the
        // field element is not a small non-negative integer.
        if b[..24].iter().any(|x| *x != 0) {
            return None;
        }
        Some(u64::from_be_bytes(b[24..32].try_into().expect("8 bytes")))
    }
    // Zero matches the first branch, so `-0` is never produced here.
    as_u64(hook)
        .map(|m| (false, m))
        .or_else(|| as_u64(&(Fr::from(0u64) - hook)).map(|m| (true, m)))
}

/// A built DUAL-LEAF `xsd:dateTime` / `xsd:date` value-lane FILTER (sq-wz99x):
/// the public [`ProofInputs`] plus the member's three PRIVATE witnesses.
///
/// The witnesses are the dual-leaf components
/// `sparq_zk::dual_leaf_datetime::{encode_datetime, encode_date}` produced for the
/// committed literal, with the signed `VALUE_HOOK` split back into the member's
/// `(value_neg, value_hook_scaled)` shape — so `inputs.operand_enc` is by
/// construction the leaf those witnesses rebind to in-circuit.
// [OPUS-5] sq-wz99x: dateTime/date value-lane host wiring. Opt-in, NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltFilterValueDlDateTime {
    /// Public inputs for the [`CircuitId::FilterValueDlDateTime`] member, carrying
    /// the LANE constant that selects `xsd:dateTime` or `xsd:date`.
    pub inputs: ProofInputs,
    /// PRIVATE: sign of the operand's epoch (`true` = pre-1970).
    pub value_neg: bool,
    /// PRIVATE: `|T|` in milliseconds — the value handle's MAGNITUDE, as a field.
    pub value_hook_scaled: FieldHex,
    /// PRIVATE: the OFF-circuit blake3 lexical hash, carried as a free witness.
    pub lexical_component: FieldHex,
}

/// Build a DUAL-LEAF `xsd:dateTime` value-lane FILTER over the committed literal
/// `literal` under operator `op` against the constant instant `bound` (sq-wz99x —
/// the circuit half of the §13 lane whose host encoder is sq-we9vs).
///
/// BOTH operands go through `sparq_zk::dual_leaf_datetime::encode_datetime`, so
/// the disclosed `bound_scaled_epoch` is derived from the SAME §13.4 fail-closed
/// canonical predicate and the SAME scaled-epoch mapping as the hidden value —
/// the host cross-vector that makes the comparison meaningful. A `bound` outside
/// the hookable domain (bare / non-`Z` offset / `24:00:00` / leap second /
/// non-canonical year / over-`FS` fraction / `u64`-overflowing) returns the
/// encoder's `Err` unchanged, so a desynced or indeterminate comparison is never
/// built here.
///
/// Requiring both operands on the SAME lane is what makes a cross-lane
/// (`xsd:date` vs `xsd:dateTime`) comparison structurally inexpressible through
/// this API — matching the in-circuit lane separation, which is the public
/// `datatype_const` and only that (see
/// [`crate::manifest::date_datatype_const`]).
///
/// The disclosed verdict is COMPUTED here ([`signed_epoch_verdict`]) rather than
/// taken from the caller, so an honest host cannot accidentally disclose a verdict
/// the member will refuse to prove.
///
/// DOCUMENTED RISK: inherits the value lane's INV-VL downgrade (#769 accepted),
/// and the §13 rule set is itself an OPEN external-audit obligation (CR-G8 /
/// sq-qhy4). NOT externally audited; no soundness / privacy claim.
// [OPUS-5] sq-wz99x: dateTime lane host wiring. Opt-in, NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
pub fn build_filter_value_dl_datetime(
    literal: &oxrdf::Literal,
    op: FilterOp,
    bound: &oxrdf::Literal,
) -> Result<BuiltFilterValueDlDateTime, sparq_zk::dual_leaf::DualLeafError> {
    let value = sparq_zk::dual_leaf_datetime::encode_datetime(literal)?;
    let bound_components = sparq_zk::dual_leaf_datetime::encode_datetime(bound)?;
    build_dl_datetime_common(
        literal,
        bound,
        op,
        value,
        bound_components,
        crate::manifest::datetime_datatype_const(),
    )
}

/// Build a DUAL-LEAF `xsd:date` value-lane FILTER — the same member, the same
/// scaled-epoch timeline, the DATE lane constant (sq-wz99x, §13.3).
///
/// A date's value handle is the scaled epoch of its STARTING instant (midnight
/// UTC), so it is numerically equal to the dateTime hook of that same instant;
/// [`crate::manifest::date_datatype_const`] is what keeps the two lanes apart. As
/// with [`build_filter_value_dl_datetime`], both operands go through
/// `sparq_zk::dual_leaf_datetime::encode_date`, so a bare date bound (order-
/// INDETERMINATE per §13.2) is rejected fail-closed rather than compared.
///
/// DOCUMENTED RISK: as [`build_filter_value_dl_datetime`] (CR-G8 / sq-qhy4).
// [OPUS-5] sq-wz99x: date lane host wiring. Opt-in, NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
pub fn build_filter_value_dl_date(
    literal: &oxrdf::Literal,
    op: FilterOp,
    bound: &oxrdf::Literal,
) -> Result<BuiltFilterValueDlDateTime, sparq_zk::dual_leaf::DualLeafError> {
    let value = sparq_zk::dual_leaf_datetime::encode_date(literal)?;
    let bound_components = sparq_zk::dual_leaf_datetime::encode_date(bound)?;
    build_dl_datetime_common(
        literal,
        bound,
        op,
        value,
        bound_components,
        crate::manifest::date_datatype_const(),
    )
}

/// The lane-agnostic remainder shared by the dateTime and date builders — ONE
/// member, ONE assembly; only the `datatype_const` differs (sq-wz99x, §13.5).
#[cfg(feature = "dual-leaf")]
fn build_dl_datetime_common(
    literal: &oxrdf::Literal,
    bound: &oxrdf::Literal,
    op: FilterOp,
    value: sparq_zk::dual_leaf::DualLeafComponents,
    bound_components: sparq_zk::dual_leaf::DualLeafComponents,
    datatype_const: FieldHex,
) -> Result<BuiltFilterValueDlDateTime, sparq_zk::dual_leaf::DualLeafError> {
    // The encoder's §13.4 predicate already rejects a magnitude overflowing u64,
    // so this split cannot fail on encoder output — surface it fail-closed anyway
    // rather than unwrapping an invariant the type system does not carry.
    let (value_neg, value_mag) = split_signed_hook(&value.value_hook).ok_or_else(|| {
        sparq_zk::dual_leaf::DualLeafError::NonCanonicalValue(literal.to_string())
    })?;
    let (bound_neg, bound_mag) = split_signed_hook(&bound_components.value_hook)
        .ok_or_else(|| sparq_zk::dual_leaf::DualLeafError::NonCanonicalValue(bound.to_string()))?;
    Ok(BuiltFilterValueDlDateTime {
        inputs: ProofInputs::FilterValueDlDateTime {
            id: CircuitId::FilterValueDlDateTime,
            operand_enc: hexf(&value.leaf()),
            op,
            bound_neg,
            bound_scaled_epoch: bound_mag,
            datatype_const,
            expected: signed_epoch_verdict(value_neg, value_mag, bound_neg, bound_mag, op),
        },
        value_neg,
        // The member takes the MAGNITUDE privately and re-folds the sign itself.
        value_hook_scaled: hexf(&Fr::from(value_mag)),
        lexical_component: hexf(&value.lexical_component),
    })
}

/// Encode an xsd:integer literal `value` to its term encoding under `salt`
/// (salt-independent for literals; convenience for callers wiring a filter to
/// a known constant).
pub fn encode_int_literal(value: u64) -> FieldHex {
    let lit = oxrdf::Literal::new_typed_literal(
        value.to_string(),
        NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
    );
    let enc = encode_term(&Term::Literal(lit), &Fr::from(0u64))
        .expect("integer literal is committable");
    hexf(&enc)
}

/// XSD double datatype IRI (the composable filter_f64 fragment's literal type).
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";

/// Build a MANIFEST-COMPOSABLE filter_f64 proof for a hidden INTEGER-VALUED
/// xsd:double operand `value` (written in plain canonical decimal-integer lexical
/// form, e.g. `42` => `"42"^^xsd:double`) under operator `op` against the constant
/// double `bound`, with the disclosed `expected` verdict ([OPUS-4.8] sq-q7e /
/// sq-tat). `operand_enc` must be the SAME term encoding the scan proof disclosed
/// for the operand column (binding consistency, exactly like `build_filter_int`).
///
/// Returns `(inputs, digit_bytes)` or `None` if `value`'s digit count has no
/// compiled `filter_f64_d{D}` member (out-of-family => clean error, never a
/// silently-unprovable wrong-D — sq-wto discipline). `bound` is the FILTER's
/// constant double; its IEEE-754 bit pattern is carried as `b_bits` and the
/// verdict follows IEEE semantics (NaN-correct via `sparq_ieee754`).
///
/// # Fragment (honest scope)
/// The hidden operand must be an integer-valued double in plain decimal form: the
/// circuit derives the IEEE bits as `f64::from(value)` (exact for the <= 2^53
/// fragment) and binds `operand_enc` to the `"<digits>"^^xsd:double` token. A
/// fractional/scientific operand is outside this member (no in-circuit
/// decimal→IEEE parser — deferred). `value` itself is a `u64` here, so the host
/// caller wires an integer-valued operand by construction.
pub fn build_filter_f64(
    operand_enc: FieldHex,
    value: u64,
    op: FilterOp,
    bound: f64,
    expected: bool,
) -> Option<(ProofInputs, Vec<u8>)> {
    let digits = value.to_string();
    let id = derive_filter_f64_id(digits.len() as u32)?;
    let digit_bytes: Vec<u8> = digits.bytes().collect();
    Some((
        ProofInputs::FilterF64 { id, operand_enc, op, b_bits: bound.to_bits(), expected },
        digit_bytes,
    ))
}

/// Encode an INTEGER-VALUED xsd:double literal `value` (plain decimal-integer
/// lexical form `"<value>"^^xsd:double`) to its term encoding — the convenience
/// wiring a composable float filter to a known constant operand. The lexical form
/// is `value.to_string()` (no `.0`/exponent), matching the
/// `filter_f64_composable_check` token rebuild. (Salt-independent for literals.)
// [OPUS-4.8] sq-q7e + sq-tat.
pub fn encode_double_literal(value: u64) -> FieldHex {
    let lit = oxrdf::Literal::new_typed_literal(
        value.to_string(),
        NamedNode::new(XSD_DOUBLE).unwrap(),
    );
    let enc = encode_term(&Term::Literal(lit), &Fr::from(0u64))
        .expect("double literal is committable");
    hexf(&enc)
}

/// XSD integer / decimal datatype IRIs (the signed-int / decimal filter
/// fragments' literal types). [OPUS-4.8] sq-1q9h.
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";

/// Build a MANIFEST-COMPOSABLE `filter_signed_int` proof for a hidden SIGNED
/// xsd:integer operand `value` under operator `op` against the constant `bound`
/// (also signed `i64`), with the disclosed `expected` verdict ([OPUS-4.8] sq-7lrq).
/// `operand_enc` must be the SAME term encoding the scan proof disclosed for the
/// operand column (binding consistency, exactly like [`build_filter_int`]).
///
/// Returns `(inputs, witness)` — the [`FilterSignedWitness`] carries the operand's
/// PRIVATE sign flag + canonical MAGNITUDE digits the manifest never holds — or
/// `None` if `value`'s MAGNITUDE digit count has no compiled
/// `filter_signed_int_d{MD}` member ([`derive_filter_signed_int_id`] => `None`, a
/// clean error, never a silently-unprovable wrong-MD — sq-wto). The witnessed
/// `mag_digits` are the canonical magnitude (`|value|.to_string()`, no sign); the
/// circuit re-derives the sign from the witnessed `neg` flag and the token. The
/// FILTER's constant `bound` is carried sign-split as `(bound_neg, |bound|)` in the
/// PUBLIC inputs. Thread the witness through [`crate::toml::prover_toml_for`]'s
/// `filter_signed_witness` arg (mirrors `build_join` -> the `join_witness` arg).
// [OPUS-4.8] sq-7lrq: composable signed xsd:integer FILTER builder.
pub fn build_filter_signed_int(
    operand_enc: FieldHex,
    value: i64,
    op: FilterOp,
    bound: i64,
    expected: bool,
) -> Option<(ProofInputs, FilterSignedWitness)> {
    let mag = value.unsigned_abs();
    let digits = mag.to_string();
    let id = derive_filter_signed_int_id(digits.len() as u32)?;
    let mag_digit_bytes: Vec<u8> = digits.bytes().collect();
    Some((
        ProofInputs::FilterSignedInt {
            id,
            operand_enc,
            op,
            bound_neg: bound < 0,
            bound: bound.unsigned_abs(),
            expected,
        },
        FilterSignedWitness {
            neg: value < 0,
            int_digits: mag_digit_bytes,
            frac_digits: Vec::new(),
        },
    ))
}

/// Build a MANIFEST-COMPOSABLE `filter_decimal` proof for a hidden xsd:decimal
/// operand given its `neg` sign, canonical integer-part digit string `int_part`, and
/// EXACTLY `frac` fraction-digit string (lexical form `[-]?<int>.<frac>`, e.g.
/// `(false, "123", "45")` => `"123.45"`), under operator `op` against a HOST-PRESCALED
/// constant `(bound_neg, bound_scaled)` where `bound_scaled = round(|bound| * 10^fd)`
/// and `fd == frac.len()` ([OPUS-4.8] sq-7lrq), with the disclosed `expected` verdict.
/// `operand_enc` must be the SAME term encoding the scan proof disclosed for the
/// operand column (binding consistency, exactly like [`build_filter_int`]).
///
/// The caller supplies the operand digits as STRINGS (not a parsed number) because a
/// decimal's lexical form fixes BOTH digit counts — including a leading integer `0`
/// (`"0.50"`) or trailing fraction zeros (`"1.50"`) — which the committed token, and
/// hence the member shape, must match byte-for-byte. The caller is responsible for
/// prescaling `bound` to `bound_scaled` at `fd = frac.len()` decimal places.
///
/// Returns `(inputs, witness)` — the [`FilterSignedWitness`] carries the operand's
/// PRIVATE sign flag + canonical integer-part and fraction digits — or `None` if the
/// operand's `(int_part.len(), frac.len())` shape has no compiled
/// `filter_decimal_i{ID}_f{FD}` member ([`derive_filter_decimal_id`] => `None`, a
/// clean error, never a silently-unprovable wrong shape — sq-wto), or if either
/// string contains a non-ASCII-digit byte. Thread the witness through
/// [`crate::toml::prover_toml_for`]'s `filter_signed_witness` arg.
// [OPUS-4.8] sq-7lrq: composable xsd:decimal FILTER builder.
#[allow(clippy::too_many_arguments)]
pub fn build_filter_decimal(
    operand_enc: FieldHex,
    neg: bool,
    int_part: &str,
    frac: &str,
    op: FilterOp,
    bound_neg: bool,
    bound_scaled: u64,
    expected: bool,
) -> Option<(ProofInputs, FilterSignedWitness)> {
    if !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let id = derive_filter_decimal_id(int_part.len() as u32, frac.len() as u32)?;
    let int_digit_bytes: Vec<u8> = int_part.bytes().collect();
    let frac_digit_bytes: Vec<u8> = frac.bytes().collect();
    Some((
        ProofInputs::FilterDecimal {
            id,
            operand_enc,
            op,
            bound_neg,
            bound_scaled,
            expected,
        },
        FilterSignedWitness {
            neg,
            int_digits: int_digit_bytes,
            frac_digits: frac_digit_bytes,
        },
    ))
}

/// The PRIVATE witnesses a composable signed-int / decimal FILTER proof needs but
/// the manifest never carries — the operand's sign flag + canonical digits. For
/// [`ProofInputs::FilterSignedInt`] the digits live in [`Self::int_digits`] (the
/// magnitude digits) and [`Self::frac_digits`] is empty; for
/// [`ProofInputs::FilterDecimal`] both arrays carry the integer-part and fraction
/// digits. Mirrors [`JoinWitness`]: a non-manifest witness threaded through
/// [`crate::toml::prover_toml_for`]'s `filter_signed_witness` arg. [OPUS-4.8] sq-7lrq.
#[derive(Debug, Clone)]
pub struct FilterSignedWitness {
    /// The hidden operand's sign flag (`true` = negative). For signed-int this is
    /// bound into the canonical token via the leading `-`; for decimal likewise.
    pub neg: bool,
    /// Canonical digits of the operand's integer part (signed-int: the magnitude
    /// digits; decimal: the integer-part digits). Length is the member's `MD` / `ID`.
    pub int_digits: Vec<u8>,
    /// Canonical fraction digits (decimal only; EMPTY for signed-int). Length is the
    /// member's `FD`.
    pub frac_digits: Vec<u8>,
}

/// Encode a SIGNED `xsd:integer` literal `value` (canonical lexical form
/// `value.to_string()`, e.g. `-42` => `"-42"^^xsd:integer`) to its term
/// encoding — the host convenience wiring a `filter_signed_int` proof to a known
/// constant operand. The lexical form (with an optional leading `-`) matches the
/// `filter_signed_int_check` token rebuild byte-for-byte (oxrdf passes the
/// lexical form through verbatim). (Salt-independent for literals.)
/// [OPUS-4.8] sq-1q9h.
pub fn encode_signed_int_literal(value: i64) -> FieldHex {
    let lit = oxrdf::Literal::new_typed_literal(
        value.to_string(),
        NamedNode::new(XSD_INTEGER).unwrap(),
    );
    let enc = encode_term(&Term::Literal(lit), &Fr::from(0u64))
        .expect("integer literal is committable");
    hexf(&enc)
}

/// Encode an `xsd:decimal` literal from its sign, integer-part digits, and
/// EXACTLY `fd` fraction digits (lexical form `[-]?<int>.<frac>`, e.g.
/// `(false, 123, "45")` => `"123.45"^^xsd:decimal`) to its term encoding — the
/// host convenience wiring a `filter_decimal` proof to a known constant operand.
/// The lexical form matches the `filter_decimal_check` token rebuild
/// byte-for-byte. `frac` must be exactly the member's `FD` digits (zero-padded
/// by the caller, e.g. `"50"` for `0.50`). (Salt-independent for literals.)
/// [OPUS-4.8] sq-1q9h.
pub fn encode_decimal_literal(neg: bool, int_part: u64, frac: &str) -> FieldHex {
    let sign = if neg { "-" } else { "" };
    let lexical = format!("{sign}{int_part}.{frac}");
    let lit = oxrdf::Literal::new_typed_literal(lexical, NamedNode::new(XSD_DECIMAL).unwrap());
    let enc = encode_term(&Term::Literal(lit), &Fr::from(0u64))
        .expect("decimal literal is committable");
    hexf(&enc)
}

fn subj_term(t: &oxrdf::Triple) -> Term {
    match &t.subject {
        oxrdf::NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
        oxrdf::NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
    }
}

// === Hidden cross-credential JOIN (sq-bwwl / sq-r2s8, step 4 proving path) ====
// [OPUS-4.8] sq-r2s8: assemble the `join_eq` witness for proving — the host
// analogue of `build_scan`. Mirrors the scan build_* pattern: re-encode each
// witnessed graph's canonical triples (same salt convention the commitment used,
// so the in-circuit `commit_fold` recompute reproduces `C(G)`), locate the two
// rows whose chosen slots carry the SAME join value, and bind that value under a
// per-presentation blinder into the public hiding `join_commitment`. The join
// VALUE and both rows are PRIVATE witnesses; only the two commitments, the hiding
// commitment, and the two query-bound slots are public.

/// The private witnesses a `join_eq` proof needs but the manifest never carries —
/// exactly the `main` PRIVATE parameters of `join_eq_na{n_a}_nb{n_b}` (in
/// declaration order): the two graphs' per-slot encodings + active counts, the two
/// joined rows, and the per-presentation blinder. `build` does NOT pad to the
/// circuit's `N` slots; [`crate::toml::join_prover_toml`] pads `enc_a`/`enc_b` to
/// the member's bucket exactly as `prover_toml_for`'s scan arm pads `enc`.
// [OPUS-4.8] sq-r2s8: hidden cross-credential JOIN witness (mirrors ScanWitness).
#[derive(Debug, Clone)]
pub struct JoinWitness {
    /// Graph-A per-slot term encodings of every canonical triple (active leaves).
    pub enc_a: Vec<[FieldHex; 3]>,
    /// Active triple count of graph A (`<= n_a`).
    pub counts_a: u32,
    /// Graph-B per-slot term encodings.
    pub enc_b: Vec<[FieldHex; 3]>,
    /// Active triple count of graph B (`<= n_b`).
    pub counts_b: u32,
    /// The joined row of graph A (the row whose `slot_a` carries the join value).
    pub row_a: [FieldHex; 3],
    /// The joined row of graph B (the row whose `slot_b` carries the join value).
    pub row_b: [FieldHex; 3],
    /// The per-presentation blinder folded into the public `join_commitment`.
    pub blinding: FieldHex,
}

/// A built hidden-join proof: the public inputs ([`ProofInputs::JoinEq`]) + the
/// private witness ([`JoinWitness`]). Mirrors [`BuiltScan`].
// [OPUS-4.8] sq-r2s8.
#[derive(Debug, Clone)]
pub struct BuiltJoin {
    pub inputs: ProofInputs,
    pub witness: JoinWitness,
}

/// One graph's canonical triples re-encoded both as field elements (to locate the
/// joined rows) and as hex (the witness the toml emits). [OPUS-4.8] sq-r2s8.
type EncodedGraph = (Vec<[Fr; 3]>, Vec<[FieldHex; 3]>);

/// Re-encode every canonical triple of `c` under its recorded salt — the same
/// re-derivation [`build_scan`] performs. `None` if a term is not committable.
// [OPUS-4.8] sq-r2s8.
fn encode_join_graph(c: &GraphCommitment) -> Option<EncodedGraph> {
    let salt = c.salt;
    let mut fr = Vec::with_capacity(c.canonical.triples.len());
    let mut hex = Vec::with_capacity(c.canonical.triples.len());
    for t in &c.canonical.triples {
        let s = encode_term(&subj_term(t), &salt)?;
        let p = encode_term(&Term::NamedNode(t.predicate.clone()), &salt)?;
        let o = encode_term(&t.object, &salt)?;
        fr.push([s, p, o]);
        hex.push([hexf(&s), hexf(&p), hexf(&o)]);
    }
    Some((fr, hex))
}

/// Build a hidden cross-credential JOIN proof's inputs + witness from the two
/// graph commitments and the two query-bound join slots, blinding the joined
/// value with `blinding` (sq-bwwl / sq-r2s8, design §2.2/§3.2).
///
/// `slot_a`/`slot_b` (in `{0,1,2}` for s/p/o) are the query-derived positions the
/// shared variable occupies in each pattern — the SAME slots the verifier's
/// `bind_joins` gate pins (design §4.4). This builder finds the first `(row_a,
/// row_b)` whose `row_a[slot_a] == row_b[slot_b]` (the join value), so an honest
/// caller need only supply the two committed graphs and the slots. Both graphs
/// must use the salt convention recorded in their [`GraphCommitment`] (the
/// in-circuit `commit_fold` recomputes `C(G)` from the re-encoded leaves — a wrong
/// encoding would fail that recompute, so this re-derivation is sound by
/// construction, exactly as in [`build_scan`]).
///
/// Returns `None` if: either slot is out of `{0,1,2}`; no row pair shares a value
/// at the chosen slots (no honest join — the prover has no satisfying witness); or
/// either graph's size has no compiled [`JOIN_EQ_N_BUCKETS`] member
/// ([`derive_join_eq_id`] => `None`, a clean error, never a wrong-N member).
///
/// `blinding` is a per-presentation random field element; two presentations of the
/// same join value under different blinders produce UNLINKABLE `join_commitment`s
/// (design §2.4 R4). The caller draws it (e.g. from its CSPRNG mapped into the
/// field), exactly as the ingest salt is drawn.
// [OPUS-4.8] sq-r2s8: hidden cross-credential JOIN builder.
pub fn build_join(
    commit_a: &GraphCommitment,
    slot_a: u32,
    commit_b: &GraphCommitment,
    slot_b: u32,
    blinding: Fr,
) -> Option<BuiltJoin> {
    if slot_a > 2 || slot_b > 2 {
        return None;
    }

    // Re-encode each graph's canonical triples under its recorded salt — the same
    // re-derivation `build_scan` does (the leaf is `h3(enc)`, so a wrong encoding
    // fails the in-circuit commitment recompute). The field form locates the joined
    // rows; the hex form is the witness the toml emits.
    let (fr_a, enc_a) = encode_join_graph(commit_a)?;
    let (fr_b, enc_b) = encode_join_graph(commit_b)?;
    let counts_a = fr_a.len() as u32;
    let counts_b = fr_b.len() as u32;

    let id = derive_join_eq_id(counts_a, counts_b)?;

    // Locate the join: the first row pair whose chosen slots carry the same value.
    // `slot_a`/`slot_b` are bounds-checked above, so the indexing is in range.
    let (sa, sb) = (slot_a as usize, slot_b as usize);
    let mut joined: Option<(usize, usize, Fr)> = None;
    'outer: for (ia, ra) in fr_a.iter().enumerate() {
        for (ib, rb) in fr_b.iter().enumerate() {
            if ra[sa] == rb[sb] {
                joined = Some((ia, ib, ra[sa]));
                break 'outer;
            }
        }
    }
    let (ia, ib, value) = joined?;

    let row_a = enc_a[ia].clone();
    let row_b = enc_b[ib].clone();

    // Bind the join value under the per-presentation blinder — the in-circuit step
    // 5 recomputes this identically (single source of truth: the Rust + Noir
    // `join_value_commitment` agree byte-for-byte), so the public `join_commitment`
    // matches what the proof binds.
    let join_commitment = hexf(&join_value_commitment(&value, &blinding));

    Some(BuiltJoin {
        inputs: ProofInputs::JoinEq {
            id,
            commit_a: hexf(&commit_a.commitment),
            commit_b: hexf(&commit_b.commitment),
            join_commitment,
            slot_a,
            slot_b,
        },
        witness: JoinWitness {
            enc_a,
            counts_a,
            enc_b,
            counts_b,
            row_a,
            row_b,
            blinding: hexf(&blinding),
        },
    })
}

/// The witnesses a bounded-depth `path_reach` proof needs but the manifest never
/// carries (sq-3kd2g.6): the PRIVATE chain length, the per-step node array, and
/// the per-graph triple encodings. Mirrors [`ScanWitness`] / [`JoinWitness`]. The
/// build pads `nodes` to `d` and each graph's `enc` to `n` (the member buckets).
// [OPUS-4.8] sq-3kd2g.6. Opt-in (`extended-fragment`), NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone)]
pub struct PathReachWitness {
    /// The actual (hidden) chain length `l` (`<= d`; `0` for a zero-length `p*`/`p?`).
    pub path_len: u32,
    /// The chain node after each step, length `d`: `nodes[s] = n_{s+1}` for the
    /// active steps `s < l`, and the inert pass-through endpoint for `s >= l`.
    pub nodes: Vec<FieldHex>,
    /// Per-graph active triple counts (length `k`, in commitment-sorted order).
    pub counts: Vec<u32>,
    /// Per-graph per-slot term encodings (active leaves; build pads to `n`).
    pub enc: Vec<Vec<[FieldHex; 3]>>,
}

/// A built bounded-depth path proof: the public inputs
/// ([`ProofInputs::PathReach`]) + the private witness ([`PathReachWitness`]).
/// Mirrors [`BuiltScan`] / [`BuiltJoin`].
// [OPUS-4.8] sq-3kd2g.6.
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone)]
pub struct BuiltPathReach {
    pub inputs: ProofInputs,
    pub witness: PathReachWitness,
}

/// Shortest directed chain `src ->(pred) ... ->(pred) dst` with `1..=max_depth`
/// edges over the union `edges` (each `(from, to)` a committed `(from, pred, to)`
/// triple). Visited-set BFS, so the first time a node is reached is via a
/// shortest path; cycles are permitted by the statement but never needed for a
/// SHORTEST witness. Returns the node list `[src, n_1, ..., dst]`, or `None` if
/// `dst` is unreachable within the bound (or `src == dst`, which is the
/// zero-length case handled by the caller — a `+` cycle back to `src` is not
/// searched here). [OPUS-4.8] sq-3kd2g.6.
#[cfg(feature = "extended-fragment")]
fn bfs_path_chain(edges: &[(Fr, Fr)], src: Fr, dst: Fr, max_depth: usize) -> Option<Vec<Fr>> {
    if src == dst {
        return None;
    }
    let mut visited: Vec<Fr> = vec![src];
    // (node, parent) records for path reconstruction.
    let mut parent: Vec<(Fr, Fr)> = Vec::new();
    // (node, depth) BFS queue as an index-walked Vec (no external deps).
    let mut queue: Vec<(Fr, usize)> = vec![(src, 0)];
    let mut qi = 0;
    while qi < queue.len() {
        let (cur, depth) = queue[qi];
        qi += 1;
        if depth >= max_depth {
            continue;
        }
        for &(f, t) in edges {
            if f == cur && !visited.contains(&t) {
                visited.push(t);
                parent.push((t, cur));
                if t == dst {
                    let mut path = vec![dst];
                    let mut node = dst;
                    while node != src {
                        let p = parent.iter().find(|(n, _)| *n == node)?.1;
                        path.push(p);
                        node = p;
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push((t, depth + 1));
            }
        }
    }
    None
}

/// Build a bounded-depth property-path (`p+` / `p*` / `p?`) reachability proof's
/// inputs + witness from the committed graphs, the path predicate, and the
/// disclosed source/destination endpoints (sq-3kd2g.6, the compose side of the
/// sq-3kd2g.2 circuits; design record §4).
///
/// `allow_zero` selects the operator's zero-length admissibility (`true` for
/// `p*`/`p?`, `false` for `p+`). The builder re-encodes each graph's canonical
/// triples under its recorded salt (the SAME re-derivation [`build_scan`] does —
/// the in-circuit `commit_fold` recomputes `C(G)`, so a wrong encoding fails that
/// recompute), then searches for the SHORTEST chain of `<= d` committed
/// `predicate` edges from `source` to `dest` (or the zero-length case when
/// `allow_zero`, `source == dest`, and the endpoint occurs as a node of the
/// committed union — req 5). It picks the SMALLEST compiled depth member that
/// covers the chain ([`smallest_path_reach_id`]) and fills the inert
/// pass-through padding (`nodes[l..d] = ` the endpoint) so the witness satisfies
/// the circuit's padding-soundness rule (req 4).
///
/// Returns `None` if: `commitments` is empty; a term is not committable; no chain
/// `<= max compiled d` connects the endpoints (no honest witness — the prover has
/// none); the zero-length occurrence witness is absent; or no compiled
/// [`PATH_REACH_MEMBERS`] member fits `(k, max-graph, chain-length)` (fail-closed,
/// never a wrong-bucket member).
///
/// The `commitments` (and every per-graph witness + the public `attribution`) are
/// emitted sorted ASCENDING by commitment (the strict-ordering guard 1b), exactly
/// as [`build_scan`] does, so an honest `k >= 2` proof satisfies the gate for any
/// caller order.
///
/// # SOUNDNESS (load-bearing, NOT a security claim)
/// A bounded path proof is EXISTENCE-ONLY. This builds a satisfying witness; the
/// verifier stack is internally re-audited but NOT externally audited (sq-qhy4);
/// no soundness / privacy property is asserted.
// [OPUS-4.8] sq-3kd2g.6: bounded-depth path-reachability builder.
#[cfg(feature = "extended-fragment")]
pub fn build_path_reach(
    commitments: &[GraphCommitment],
    predicate: &NamedNode,
    source: &Term,
    dest: &Term,
    allow_zero: bool,
) -> Option<BuiltPathReach> {
    if commitments.is_empty() {
        return None;
    }
    let k = commitments.len() as u32;
    // 1b strict-ordering: sort graphs ascending by commitment, lockstep with the
    // witnesses + attribution (the exact order the in-circuit `Field::lt` uses).
    let mut order: Vec<usize> = (0..commitments.len()).collect();
    order.sort_by(|&a, &b| {
        field_to_be_bytes_32(&commitments[a].commitment)
            .cmp(&field_to_be_bytes_32(&commitments[b].commitment))
    });
    let sorted: Vec<&GraphCommitment> = order.iter().map(|&i| &commitments[i]).collect();

    // Constant endpoints/predicate encodings are salt-independent (IRIs/literals);
    // use graph 0's salt as the reference, exactly as `build_scan` does.
    let ref_salt = sorted[0].salt;
    let pred_fr = encode_term(&Term::NamedNode(predicate.clone()), &ref_salt)?;
    let src_fr = encode_term(source, &ref_salt)?;
    let dst_fr = encode_term(dest, &ref_salt)?;

    let mut enc_fr: Vec<Vec<[Fr; 3]>> = Vec::with_capacity(sorted.len());
    let mut enc_hex: Vec<Vec<[FieldHex; 3]>> = Vec::with_capacity(sorted.len());
    let mut counts: Vec<u32> = Vec::with_capacity(sorted.len());
    let mut commit_hex: Vec<FieldHex> = Vec::with_capacity(sorted.len());
    for c in &sorted {
        let (fr, hex) = encode_join_graph(c)?;
        counts.push(fr.len() as u32);
        enc_fr.push(fr);
        enc_hex.push(hex);
        commit_hex.push(hexf(&c.commitment));
    }
    let max_graph = counts.iter().copied().max().unwrap_or(0);
    // The deepest compiled member for this `k` bounds the chain search.
    let max_depth = PATH_REACH_MEMBERS
        .iter()
        .filter(|&&(_, mk, _)| mk == k)
        .map(|&(d, _, _)| d)
        .max()? as usize;

    // Edge set: every committed `(from, predicate, to)` triple across the union.
    let mut edges: Vec<(Fr, Fr)> = Vec::new();
    for g in &enc_fr {
        for t in g {
            if t[1] == pred_fr {
                edges.push((t[0], t[2]));
            }
        }
    }

    // The chain node list [src, ..., dst]; the zero-length case is [src] (l = 0).
    let occurs = |term: Fr| {
        enc_fr
            .iter()
            .any(|g| g.iter().any(|t| t[0] == term || t[2] == term))
    };
    let chain: Vec<Fr> = if allow_zero && src_fr == dst_fr && occurs(src_fr) {
        vec![src_fr]
    } else {
        bfs_path_chain(&edges, src_fr, dst_fr, max_depth)?
    };
    let l = chain.len() - 1;

    // Per-graph source attribution — computed EXACTLY as the circuit does
    // (`path.nr` steps 3/5/6): a chain-relative bit for active steps, or the
    // endpoint-occurrence bit for the zero-length case.
    let mut attribution = vec![false; sorted.len()];
    if l == 0 {
        for (g, graph) in enc_fr.iter().enumerate() {
            attribution[g] = graph.iter().any(|t| t[0] == src_fr || t[2] == src_fr);
        }
    } else {
        for s in 0..l {
            let triple = [chain[s], pred_fr, chain[s + 1]];
            for (g, graph) in enc_fr.iter().enumerate() {
                if graph.contains(&triple) {
                    attribution[g] = true;
                }
            }
        }
    }

    // Pick the smallest compiled member that covers the chain (depth >= l, >= 1).
    let id = smallest_path_reach_id(k, max_graph, (l as u32).max(1))?;
    let CircuitId::PathReach { d, .. } = &id else {
        return None;
    };
    let d = *d as usize;

    // Node array of length d: active nodes then the inert pass-through endpoint.
    let endpoint = if l == 0 { src_fr } else { dst_fr };
    let mut nodes_fr = vec![endpoint; d];
    for (s, slot) in nodes_fr.iter_mut().enumerate().take(l) {
        *slot = chain[s + 1];
    }

    Some(BuiltPathReach {
        inputs: ProofInputs::PathReach {
            id: id.clone(),
            commitments: commit_hex,
            pred_enc: hexf(&pred_fr),
            src_enc: hexf(&src_fr),
            dst_enc: hexf(&dst_fr),
            allow_zero,
            depth_bound: d as u32,
            attribution,
        },
        witness: PathReachWitness {
            path_len: l as u32,
            nodes: nodes_fr.iter().map(hexf).collect(),
            counts,
            enc: enc_hex,
        },
    })
}

// [OPUS-4.8] sq-bif.6: GLUE unit tests for the circuit-family id derivation —
// the (data shape -> compiled member) plumbing the prover AND verifier both call
// so a proof can only fit its member. These cover the NON-cryptographic
// composition logic ONLY (bucket selection, exact-digit-count discipline,
// out-of-family rejection, package-name determinism); they assert NOTHING about
// in-circuit soundness or any privacy guarantee (the verifier remains NOT-yet-sound,
// sq-qhy4 — external accredited-cryptographer sign-off pending).
#[cfg(test)]
mod derive_id_tests {
    use super::*;

    // --- scan (k, n, r) bucket lattice -----------------------------------

    /// `derive_scan_id` maps the largest-graph size / disclosed-row count to the
    /// SMALLEST compiled bucket that fits, and the result is deterministic.
    #[test]
    fn scan_id_picks_smallest_fitting_bucket_and_is_deterministic() {
        // n picks 16 for <=16 triples, 64 for 17..=64. r picks 4 for <=4, 8 for 5..=8.
        let id = derive_scan_id(1, 10, 3).expect("10<=16, 3<=4 fits");
        assert_eq!(id, CircuitId::Scan { k: 1, n: 16, r: 4 });
        // Exactly-N (the bucket boundary) selects that bucket, not the next.
        assert_eq!(
            derive_scan_id(2, 16, 4),
            Some(CircuitId::Scan { k: 2, n: 16, r: 4 }),
            "max_graph == 16 and match_rows == 4 fit the {{16,4}} bucket exactly"
        );
        // One over a boundary rolls up to the next compiled bucket.
        assert_eq!(
            derive_scan_id(1, 17, 5),
            Some(CircuitId::Scan { k: 1, n: 64, r: 8 }),
            "17 triples needs n=64; 5 rows needs r=8"
        );
        // Determinism: same inputs -> identical id across repeated calls.
        assert_eq!(derive_scan_id(2, 64, 8), derive_scan_id(2, 64, 8));
    }

    /// A zero size / zero match-count is clamped to the smallest live bucket
    /// (the circuit always commits at least one slot / discloses at least the
    /// padded row word), not rejected.
    #[test]
    fn scan_id_clamps_zero_to_smallest_bucket() {
        assert_eq!(
            derive_scan_id(1, 0, 0),
            Some(CircuitId::Scan { k: 1, n: 16, r: 4 }),
            "max_graph/match_rows clamp up to 1, selecting the smallest bucket"
        );
    }

    /// Out-of-family inputs return `None` (a clean error at the call site),
    /// never a wrong member: an uncompiled `k`, or a size past every bucket.
    #[test]
    fn scan_id_out_of_family_is_none() {
        assert!(derive_scan_id(3, 10, 3).is_none(), "k=3 is not compiled");
        assert!(derive_scan_id(0, 10, 3).is_none(), "k=0 is not compiled");
        assert!(
            derive_scan_id(1, 65, 3).is_none(),
            "65 triples exceeds the largest n bucket (64)"
        );
        assert!(
            derive_scan_id(1, 10, 9).is_none(),
            "9 rows exceeds the largest r bucket (8)"
        );
    }

    // --- filter_int / filter_f64 EXACT digit-count discipline (sq-wto) ----

    /// `derive_filter_int_id` is an EXACT digit-count match: every compiled D in
    /// `FILTER_INT_D_VALUES` yields its own member; the count drives the id.
    #[test]
    fn filter_int_id_exact_match_per_compiled_digit_count() {
        for &d in FILTER_INT_D_VALUES {
            assert_eq!(
                derive_filter_int_id(d),
                Some(CircuitId::FilterInt { d }),
                "compiled D={} derives its own filter_int member",
                d
            );
        }
        // 0 clamps to the 1-digit canonical form ("0").
        assert_eq!(derive_filter_int_id(0), Some(CircuitId::FilterInt { d: 1 }));
    }

    /// The sq-wto correctness invariant: an out-of-family digit count returns
    /// `None` — NOT the next-larger member (which would be silently unprovable
    /// because the `digits: [u8; D]` witness pins the count to D exactly).
    #[test]
    fn filter_int_id_out_of_family_is_none_not_wrong_d() {
        // 5 is the first count past the compiled 1..=4 range — the regression
        // the differential fuzzer (sq-61g) caught: 5 must NOT derive d=anything.
        assert_eq!(derive_filter_int_id(5), None, "d=5 has no member (sq-wto)");
        for d in [6u32, 10, 19, 20, 100] {
            assert!(
                derive_filter_int_id(d).is_none(),
                "out-of-family digit count {} must return None, never a wrong-D member",
                d
            );
        }
    }

    /// `filter_f64` shares the EXACT-match discipline with `filter_int`.
    #[test]
    fn filter_f64_id_exact_match_and_out_of_family_none() {
        for &d in FILTER_F64_D_VALUES {
            assert_eq!(derive_filter_f64_id(d), Some(CircuitId::FilterF64 { d }));
        }
        assert_eq!(derive_filter_f64_id(0), Some(CircuitId::FilterF64 { d: 1 }));
        assert_eq!(derive_filter_f64_id(5), None, "no f64 member past the compiled range");
        assert!(derive_filter_f64_id(16).is_none(), "16 digits is past the f64 family");
    }

    // --- filter_signed_int (md magnitude-digit) ---------------------------

    /// `derive_filter_signed_int_id` is an EXACT magnitude-digit-count match over
    /// the compiled `{2, 4}` set; the IN-RANGE gap (3) and out-of-family counts
    /// both return `None` (never a wrong-MD member).
    #[test]
    fn filter_signed_int_id_exact_match_with_gap() {
        for &md in FILTER_SIGNED_INT_MD_VALUES {
            assert_eq!(
                derive_filter_signed_int_id(md),
                Some(CircuitId::FilterSignedInt { md })
            );
        }
        // 3 is INSIDE [2,4] numerically but is NOT a compiled member — exact match.
        assert_eq!(
            derive_filter_signed_int_id(3),
            None,
            "md=3 is not compiled ({{2,4}} only); exact match returns None, not md=4"
        );
        // 1 clamps from 0 but is also uncompiled.
        assert_eq!(derive_filter_signed_int_id(0), None, "md clamps to 1, which is uncompiled");
        assert!(derive_filter_signed_int_id(5).is_none(), "md=5 out of family");
    }

    // --- filter_decimal (id, fd) pair ------------------------------------

    /// `derive_filter_decimal_id` matches the `(int_digits, frac_digits)` PAIR
    /// exactly against the compiled set; a wrong integer OR fraction count gives
    /// `None`.
    #[test]
    fn filter_decimal_id_exact_pair_match() {
        for &(id, fd) in FILTER_DECIMAL_ID_FD_VALUES {
            assert_eq!(
                derive_filter_decimal_id(id, fd),
                Some(CircuitId::FilterDecimal { id, fd }),
                "compiled ({},{}) pair derives its member",
                id,
                fd
            );
        }
        // The lone compiled member is (3, 2). Either coordinate off => None.
        assert_eq!(derive_filter_decimal_id(3, 2), Some(CircuitId::FilterDecimal { id: 3, fd: 2 }));
        assert_eq!(derive_filter_decimal_id(2, 2), None, "wrong integer-digit count");
        assert_eq!(derive_filter_decimal_id(3, 3), None, "wrong fraction-digit count");
        assert_eq!(derive_filter_decimal_id(4, 2), None, "wrong integer-digit count");
        // int_digits clamps to >=1 (canonical "0.xx" has one integer digit).
        assert_eq!(
            derive_filter_decimal_id(0, 2),
            None,
            "id clamps to 1; (1,2) is not the compiled (3,2) member"
        );
    }

    // --- join_eq (n_a, n_b) bucket pair ----------------------------------

    /// `derive_join_eq_id` selects the smallest fitting `JOIN_EQ_N_BUCKETS` bucket
    /// for EACH side independently (mirrors the scan n-bucket discipline), so the
    /// four `(16|64, 16|64)` combinations are all reachable.
    #[test]
    fn join_eq_id_buckets_each_side_independently() {
        assert_eq!(derive_join_eq_id(10, 10), Some(CircuitId::JoinEq { n_a: 16, n_b: 16 }));
        assert_eq!(derive_join_eq_id(16, 64), Some(CircuitId::JoinEq { n_a: 16, n_b: 64 }));
        assert_eq!(derive_join_eq_id(64, 16), Some(CircuitId::JoinEq { n_a: 64, n_b: 16 }));
        assert_eq!(derive_join_eq_id(17, 50), Some(CircuitId::JoinEq { n_a: 64, n_b: 64 }));
        // Exactly-N boundary selects that bucket.
        assert_eq!(derive_join_eq_id(16, 16), Some(CircuitId::JoinEq { n_a: 16, n_b: 16 }));
        // Clamp zero to the smallest bucket on either side.
        assert_eq!(derive_join_eq_id(0, 0), Some(CircuitId::JoinEq { n_a: 16, n_b: 16 }));
    }

    /// Either side past every compiled bucket returns `None`, never a wrong-N member.
    #[test]
    fn join_eq_id_out_of_family_is_none() {
        assert!(derive_join_eq_id(65, 16).is_none(), "n_a=65 exceeds the largest bucket");
        assert!(derive_join_eq_id(16, 65).is_none(), "n_b=65 exceeds the largest bucket");
    }

    // --- package() name determinism + distinctness -----------------------

    /// `CircuitId::package()` is a pure, deterministic function of the id, and it
    /// names DISTINCT members for distinct (family × bucket) ids — the property
    /// the driver relies on to locate the right `zk/compose/<pkg>` directory.
    #[test]
    fn package_names_are_deterministic_and_distinct_per_family_and_bucket() {
        let ids = [
            CircuitId::Scan { k: 1, n: 16, r: 4 },
            CircuitId::Scan { k: 2, n: 16, r: 4 },
            CircuitId::Scan { k: 1, n: 64, r: 4 },
            CircuitId::Scan { k: 1, n: 16, r: 8 },
            CircuitId::FilterInt { d: 1 },
            CircuitId::FilterInt { d: 4 },
            CircuitId::FilterF64 { d: 1 },
            CircuitId::FilterSignedInt { md: 2 },
            CircuitId::FilterDecimal { id: 3, fd: 2 },
            CircuitId::JoinEq { n_a: 16, n_b: 64 },
            CircuitId::JoinEq { n_a: 64, n_b: 16 },
        ];
        // Deterministic: the same id renders the same package name every call.
        for id in &ids {
            assert_eq!(id.package(), id.package(), "package() is pure");
        }
        // Distinct: no two distinct ids collide on a package name (one directory
        // per member — a collision would prove the wrong relation under a shared dir).
        let mut names: Vec<String> = ids.iter().map(|i| i.package()).collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "every distinct id maps to a distinct package dir");
        // Spot-check the exact wire names the zk/compose/ directories use.
        assert_eq!(CircuitId::Scan { k: 2, n: 64, r: 8 }.package(), "scan_k2_n64_r8");
        assert_eq!(CircuitId::FilterInt { d: 3 }.package(), "filter_int_d3");
        assert_eq!(CircuitId::FilterF64 { d: 2 }.package(), "filter_f64_d2");
        assert_eq!(CircuitId::FilterSignedInt { md: 4 }.package(), "filter_signed_int_d4");
        assert_eq!(CircuitId::FilterDecimal { id: 3, fd: 2 }.package(), "filter_decimal_i3_f2");
        assert_eq!(CircuitId::JoinEq { n_a: 64, n_b: 64 }.package(), "join_eq_na64_nb64");
    }

    /// The scan / filter_int builders thread the SAME derive-id discipline: the
    /// id carried in the built `ProofInputs` equals the id `derive_*` returns for
    /// the same shape, and an out-of-family operand makes the whole build `None`.
    #[test]
    fn builders_carry_the_derived_id_and_propagate_out_of_family_none() {
        // A 3-digit operand builds the filter_int_d3 member.
        let enc = encode_int_literal(123);
        let (inputs, digits) =
            build_filter_int(enc, 123, FilterOp::Gt, 100, true).expect("3-digit operand has d3");
        assert_eq!(*inputs.circuit_id(), CircuitId::FilterInt { d: 3 });
        assert_eq!(digits, b"123", "digit witness is the canonical decimal bytes");
        // A 5-digit operand is out of family -> the builder returns None (sq-wto),
        // exactly as derive_filter_int_id(5) does.
        let enc5 = encode_int_literal(12345);
        assert!(
            build_filter_int(enc5, 12345, FilterOp::Lt, 0, false).is_none(),
            "5-digit operand: out-of-family build is None, never a wrong-D witness"
        );
    }
}

// [OPUS-4.8] sq-3kd2g.6: DIRECT unit tests for the bounded-depth path member
// derivation + builder (host-level; the bb round-trip is symmetric to build_join
// and exercised under the nargo-gated e2e when the toolchain is present).
#[cfg(all(test, feature = "extended-fragment"))]
mod path_tests {
    use super::*;
    use oxrdf::{NamedNode, Triple};
    use sparq_zk::commit::commit_triples;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    fn t(s: &str, p: &str, o: &str) -> Triple {
        Triple::new(iri(s), iri(p), iri(o))
    }

    #[test]
    fn derive_path_reach_id_exact_membership_only() {
        assert_eq!(
            derive_path_reach_id(2, 1, 16),
            Some(CircuitId::PathReach { d: 2, k: 1, n: 16 })
        );
        assert_eq!(
            derive_path_reach_id(4, 2, 16),
            Some(CircuitId::PathReach { d: 4, k: 2, n: 16 })
        );
        // (2,2,16) is NOT a compiled member (only (4,2,16) exists for k=2).
        assert_eq!(derive_path_reach_id(2, 2, 16), None);
        assert_eq!(derive_path_reach_id(8, 2, 16), None);
        // A depth with no member, an out-of-family n, and k=3 all fail closed.
        assert_eq!(derive_path_reach_id(3, 1, 16), None);
        assert_eq!(derive_path_reach_id(4, 1, 64), None);
        assert_eq!(derive_path_reach_id(4, 3, 16), None);
    }

    #[test]
    fn smallest_path_reach_id_picks_min_depth_covering_the_chain() {
        // k=1: d in {2,4,8}; smallest >= min_depth.
        assert_eq!(
            smallest_path_reach_id(1, 5, 2),
            Some(CircuitId::PathReach { d: 2, k: 1, n: 16 })
        );
        assert_eq!(
            smallest_path_reach_id(1, 5, 5),
            Some(CircuitId::PathReach { d: 8, k: 1, n: 16 })
        );
        // No k=1 member deeper than 8.
        assert_eq!(smallest_path_reach_id(1, 5, 9), None);
        // k=2: only d=4 exists.
        assert_eq!(
            smallest_path_reach_id(2, 5, 2),
            Some(CircuitId::PathReach { d: 4, k: 2, n: 16 })
        );
        assert_eq!(smallest_path_reach_id(2, 5, 5), None);
        // k=3 has no member; a graph larger than the only bucket (16) has none.
        assert_eq!(smallest_path_reach_id(3, 5, 2), None);
        assert_eq!(smallest_path_reach_id(1, 20, 2), None);
    }

    #[test]
    fn build_path_reach_finds_a_two_step_chain_single_graph() {
        let salt = Fr::from(7u64);
        let g = commit_triples(
            &[
                t("http://ex/a", "http://ex/p", "http://ex/b"),
                t("http://ex/b", "http://ex/p", "http://ex/c"),
            ],
            salt,
        )
        .unwrap();
        let src = Term::NamedNode(iri("http://ex/a"));
        let dst = Term::NamedNode(iri("http://ex/c"));
        let built = build_path_reach(&[g], &iri("http://ex/p"), &src, &dst, false)
            .expect("chain a -> b -> c exists within depth");

        // Smallest k=1 member covering a 2-step chain is d=2.
        assert_eq!(built.inputs.circuit_id(), &CircuitId::PathReach { d: 2, k: 1, n: 16 });
        let ProofInputs::PathReach {
            commitments,
            src_enc,
            dst_enc,
            allow_zero,
            depth_bound,
            attribution,
            ..
        } = &built.inputs
        else {
            panic!("expected PathReach inputs");
        };
        assert_eq!(commitments.len(), 1);
        assert!(!*allow_zero);
        assert_eq!(*depth_bound, 2);
        assert_eq!(attribution, &vec![true]);
        // Endpoints bind to the disclosed terms.
        assert_eq!(src_enc, &hexf(&encode_term(&src, &salt).unwrap()));
        assert_eq!(dst_enc, &hexf(&encode_term(&dst, &salt).unwrap()));
        // Witness: two active steps, node array = [enc(b), enc(c)], count = 2.
        assert_eq!(built.witness.path_len, 2);
        assert_eq!(built.witness.nodes.len(), 2);
        let enc_b = hexf(&encode_term(&Term::NamedNode(iri("http://ex/b")), &salt).unwrap());
        assert_eq!(built.witness.nodes[0], enc_b);
        assert_eq!(built.witness.nodes[1], *dst_enc);
        assert_eq!(built.witness.counts, vec![2]);
    }

    #[test]
    fn build_path_reach_zero_length_needs_occurrence_and_allow_zero() {
        let salt = Fr::from(3u64);
        let g = commit_triples(&[t("http://ex/a", "http://ex/p", "http://ex/b")], salt).unwrap();
        let a = Term::NamedNode(iri("http://ex/a"));
        // `p*` self-path a->a: zero-length, admitted because a occurs (as subject).
        let built = build_path_reach(std::slice::from_ref(&g), &iri("http://ex/p"), &a, &a, true)
            .expect("zero-length path admitted");
        let ProofInputs::PathReach { allow_zero, attribution, .. } = &built.inputs else {
            panic!("expected PathReach inputs");
        };
        assert!(*allow_zero);
        assert_eq!(attribution, &vec![true]);
        assert_eq!(built.witness.path_len, 0);
        // `p+` (allow_zero = false) has no zero-length witness and no 1-step self
        // loop, so no chain is found.
        assert!(build_path_reach(&[g], &iri("http://ex/p"), &a, &a, false).is_none());
    }

    #[test]
    fn build_path_reach_unreachable_destination_is_none() {
        let salt = Fr::from(9u64);
        let g = commit_triples(&[t("http://ex/a", "http://ex/p", "http://ex/b")], salt).unwrap();
        let src = Term::NamedNode(iri("http://ex/a"));
        let unreached = Term::NamedNode(iri("http://ex/z"));
        assert!(build_path_reach(&[g], &iri("http://ex/p"), &src, &unreached, false).is_none());
    }

    #[test]
    fn build_path_reach_two_graph_chain_attributes_both() {
        // Chain a->b in graph 1, b->c in graph 2: k=2 (only d=4), both attributed.
        let g1 = commit_triples(&[t("http://ex/a", "http://ex/p", "http://ex/b")], Fr::from(1u64))
            .unwrap();
        let g2 = commit_triples(&[t("http://ex/b", "http://ex/p", "http://ex/c")], Fr::from(2u64))
            .unwrap();
        let src = Term::NamedNode(iri("http://ex/a"));
        let dst = Term::NamedNode(iri("http://ex/c"));
        let built = build_path_reach(&[g1, g2], &iri("http://ex/p"), &src, &dst, false)
            .expect("cross-graph chain a -> b -> c");
        assert_eq!(built.inputs.circuit_id(), &CircuitId::PathReach { d: 4, k: 2, n: 16 });
        let ProofInputs::PathReach { depth_bound, attribution, .. } = &built.inputs else {
            panic!("expected PathReach inputs");
        };
        assert_eq!(*depth_bound, 4);
        assert_eq!(attribution, &vec![true, true]);
        // d=4, l=2 => two active nodes + two inert pass-through slots (= dst).
        assert_eq!(built.witness.path_len, 2);
        assert_eq!(built.witness.nodes.len(), 4);
        let dst_enc = hexf(&encode_term(&dst, &Fr::from(1u64)).unwrap());
        assert_eq!(built.witness.nodes[1], dst_enc);
        assert_eq!(built.witness.nodes[2], dst_enc);
        assert_eq!(built.witness.nodes[3], dst_enc);
    }
}
