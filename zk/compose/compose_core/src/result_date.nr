// Canonical date comparison over the original string commitment; research-stage, not externally audited.
use crate::hashes::{h2, TYPE_CODE_LITERAL};
use crate::result::ResultInteger;

/// Parses the complete canonical token. Its static array length fixes the lexical
/// length; fixed quotes, separators and datatype bytes constrain every other position.
/// The main expected cost is BLAKE3 over this same token.
pub fn date_key<let L: u32>(token: [u8; L]) -> u64 {
    assert_eq(L, 53, "date token length");
    if L == 53 {
        assert((token[0] == 34) & (token[11] == 34), "date quotes");
        assert((token[5] == 45) & (token[8] == 45), "date separator");
        let suffix = "^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
        for i in 0..41 { assert_eq(token[12 + i], suffix[i], "date datatype"); }
        let positions = [1, 2, 3, 4, 6, 7, 9, 10];
        let mut digits: [u32; 8] = [0; 8];
        for i in 0..8 {
            let byte = token[positions[i]];
            assert((byte >= 48) & (byte <= 57), "date digit");
            digits[i] = byte as u32 - 48;
        }
        let year = digits[0] * 1000 + digits[1] * 100 + digits[2] * 10 + digits[3];
        let month = digits[4] * 10 + digits[5];
        let day = digits[6] * 10 + digits[7];
        assert((year >= 1) & (year <= 9999), "date year range");
        assert((month >= 1) & (month <= 12), "date month range");
        let leap = (year % 4 == 0) & ((year % 100 != 0) | (year % 400 == 0));
        let short_month = (month == 4) | (month == 6) | (month == 9) | (month == 11);
        let days = if month == 2 { if leap { 29 } else { 28 } } else { if short_month { 30 } else { 31 } };
        assert((day >= 1) & (day <= days), "date day range");
        (year * 10000 + month * 100 + day) as u64
    } else { 0 }
}

/// The key and literal are private inputs; the key is constrained to these bytes.
pub struct DateLiteral {
    pub token: [u8; 53],
    pub key: u64,
}

impl ResultInteger for DateLiteral {
    fn check(self, operand: Field, op: u32, bound: u64, bits: u32) {
        assert_eq(bits, 64, "date key capacity");
        assert_eq(self.key, date_key(self.token), "date key mismatch");
        assert_eq(encoding(self.token), operand, "operand encoding mismatch");
        assert(op <= 5, "unknown comparison operator");
        let lt = self.key < bound;
        let eq = self.key == bound;
        let result = if op == 0 { lt } else if op == 1 { lt | eq }
            else if op == 2 { !lt & !eq } else if op == 3 { !lt }
            else if op == 4 { eq } else { !eq };
        assert(result, "filter verdict mismatch");
    }
}

fn encoding<let L: u32>(token: [u8; L]) -> Field {
    let digest = std::hash::blake3(token);
    let mut hs: Field = 0;
    for i in 1..32 { hs = hs * 256 + digest[i] as Field; }
    h2(TYPE_CODE_LITERAL, hs)
}

#[test]
fn month_ends_and_year_bounds() {
    assert_eq(date_key("\"0001-01-01\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 10101);
    { let token = "\"0001-01-01\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 10101 }.check(encoding(token), 4, 10101, 64); }
    assert_eq(date_key("\"9999-12-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 99991231);
    { let token = "\"9999-12-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 99991231 }.check(encoding(token), 4, 99991231, 64); }
    assert_eq(date_key("\"2000-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20000229);
    { let token = "\"2000-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20000229 }.check(encoding(token), 4, 20000229, 64); }
    assert_eq(date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20240229);
    { let token = "\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20240229 }.check(encoding(token), 4, 20240229, 64); }
    assert_eq(date_key("\"2100-02-28\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 21000228);
    { let token = "\"2100-02-28\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 21000228 }.check(encoding(token), 4, 21000228, 64); }
    assert_eq(date_key("\"2023-01-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230131);
    { let token = "\"2023-01-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230131 }.check(encoding(token), 4, 20230131, 64); }
    assert_eq(date_key("\"2023-02-28\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230228);
    { let token = "\"2023-02-28\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230228 }.check(encoding(token), 4, 20230228, 64); }
    assert_eq(date_key("\"2023-03-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230331);
    { let token = "\"2023-03-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230331 }.check(encoding(token), 4, 20230331, 64); }
    assert_eq(date_key("\"2023-04-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230430);
    { let token = "\"2023-04-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230430 }.check(encoding(token), 4, 20230430, 64); }
    assert_eq(date_key("\"2023-05-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230531);
    { let token = "\"2023-05-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230531 }.check(encoding(token), 4, 20230531, 64); }
    assert_eq(date_key("\"2023-06-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230630);
    { let token = "\"2023-06-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230630 }.check(encoding(token), 4, 20230630, 64); }
    assert_eq(date_key("\"2023-07-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230731);
    { let token = "\"2023-07-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230731 }.check(encoding(token), 4, 20230731, 64); }
    assert_eq(date_key("\"2023-08-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230831);
    { let token = "\"2023-08-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230831 }.check(encoding(token), 4, 20230831, 64); }
    assert_eq(date_key("\"2023-09-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20230930);
    { let token = "\"2023-09-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20230930 }.check(encoding(token), 4, 20230930, 64); }
    assert_eq(date_key("\"2023-10-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20231031);
    { let token = "\"2023-10-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20231031 }.check(encoding(token), 4, 20231031, 64); }
    assert_eq(date_key("\"2023-11-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20231130);
    { let token = "\"2023-11-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20231130 }.check(encoding(token), 4, 20231130, 64); }
    assert_eq(date_key("\"2023-12-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()), 20231231);
    { let token = "\"2023-12-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes(); DateLiteral { token, key: 20231231 }.check(encoding(token), 4, 20231231, 64); }
}

#[test(should_fail_with = "date day range")]
fn month_1_next_day() { let _ = date_key("\"2023-01-32\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_2_next_day() { let _ = date_key("\"2023-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_3_next_day() { let _ = date_key("\"2023-03-32\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_4_next_day() { let _ = date_key("\"2023-04-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_5_next_day() { let _ = date_key("\"2023-05-32\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_6_next_day() { let _ = date_key("\"2023-06-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_7_next_day() { let _ = date_key("\"2023-07-32\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_8_next_day() { let _ = date_key("\"2023-08-32\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_9_next_day() { let _ = date_key("\"2023-09-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_10_next_day() { let _ = date_key("\"2023-10-32\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_11_next_day() { let _ = date_key("\"2023-11-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn month_12_next_day() { let _ = date_key("\"2023-12-32\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn century() { let _ = date_key("\"2100-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn ordinary() { let _ = date_key("\"2023-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date day range")]
fn day_zero() { let _ = date_key("\"2024-01-00\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date month range")]
fn month_zero() { let _ = date_key("\"2024-00-01\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date month range")]
fn month_thirteen() { let _ = date_key("\"2024-13-01\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date year range")]
fn year_zero() { let _ = date_key("\"0000-01-01\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date digit")]
fn digit_0() { let _ = date_key("\":024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date digit")]
fn digit_1() { let _ = date_key("\"2:24-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date digit")]
fn digit_2() { let _ = date_key("\"20:4-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date digit")]
fn digit_3() { let _ = date_key("\"202:-02-28\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date digit")]
fn digit_5() { let _ = date_key("\"2024-:2-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date digit")]
fn digit_6() { let _ = date_key("\"2024-0:-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date digit")]
fn digit_8() { let _ = date_key("\"2024-02-:9\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date digit")]
fn digit_9() { let _ = date_key("\"2024-02-2:\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date separator")]
fn separator_4() { let _ = date_key("\"2024/02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date separator")]
fn separator_7() { let _ = date_key("\"2024-02/29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date token length")]
fn timezone_z() { let _ = date_key("\"2024-02-29Z\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date token length")]
fn timezone_offset() { let _ = date_key("\"2024-02-29+01:00\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date token length")]
fn short_year() { let _ = date_key("\"024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date token length")]
fn long_year() { let _ = date_key("\"02024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn different_datatype() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#time>".as_bytes()); }

#[test]
fn date_binding_accepts_exact_token() {
    let token = "\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20240229 }.check(encoding(token), 4, 20240229, 64);
}

#[test(should_fail_with = "date key mismatch")]
fn different_key_is_refused() {
    let token = "\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20240301 }.check(encoding(token), 4, 20240301, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn different_committed_literal_is_refused() {
    let token = "\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let other = "\"2024-02-28\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20240229 }.check(encoding(other), 4, 20240229, 64);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn false_verdict_is_refused() {
    let token = "\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20240229 }.check(encoding(token), 1, 20240228, 64);
}

#[test(should_fail_with = "date quotes")]
fn fixed_byte_0() { let _ = date_key("!2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date quotes")]
fn fixed_byte_11() { let _ = date_key("\"2024-02-29!^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_12() { let _ = date_key("\"2024-02-29\"!^<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_13() { let _ = date_key("\"2024-02-29\"^!<http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_14() { let _ = date_key("\"2024-02-29\"^^!http://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_15() { let _ = date_key("\"2024-02-29\"^^<!ttp://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_16() { let _ = date_key("\"2024-02-29\"^^<h!tp://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_17() { let _ = date_key("\"2024-02-29\"^^<ht!p://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_18() { let _ = date_key("\"2024-02-29\"^^<htt!://www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_19() { let _ = date_key("\"2024-02-29\"^^<http!//www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_20() { let _ = date_key("\"2024-02-29\"^^<http:!/www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_21() { let _ = date_key("\"2024-02-29\"^^<http:/!www.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_22() { let _ = date_key("\"2024-02-29\"^^<http://!ww.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_23() { let _ = date_key("\"2024-02-29\"^^<http://w!w.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_24() { let _ = date_key("\"2024-02-29\"^^<http://ww!.w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_25() { let _ = date_key("\"2024-02-29\"^^<http://www!w3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_26() { let _ = date_key("\"2024-02-29\"^^<http://www.!3.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_27() { let _ = date_key("\"2024-02-29\"^^<http://www.w!.org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_28() { let _ = date_key("\"2024-02-29\"^^<http://www.w3!org/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_29() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.!rg/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_30() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.o!g/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_31() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.or!/2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_32() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org!2001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_33() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/!001/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_34() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2!01/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_35() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/20!1/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_36() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/200!/XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_37() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001!XMLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_38() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/!MLSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_39() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/X!LSchema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_40() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XM!Schema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_41() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XML!chema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_42() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLS!hema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_43() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSc!ema#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_44() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSch!ma#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_45() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSche!a#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_46() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchem!#date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_47() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema!date>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_48() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#!ate>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_49() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#d!te>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_50() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#da!e>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_51() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#dat!>".as_bytes()); }

#[test(should_fail_with = "date datatype")]
fn fixed_byte_52() { let _ = date_key("\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date!".as_bytes()); }

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_0() {
    let token = "\"0001-01-01\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 10101 }.check(encoding(committed), 4, 10101, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_1() {
    let token = "\"9999-12-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 99991231 }.check(encoding(committed), 4, 99991231, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_2() {
    let token = "\"2000-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20000229 }.check(encoding(committed), 4, 20000229, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_3() {
    let token = "\"2024-02-29\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20240229 }.check(encoding(committed), 4, 20240229, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_4() {
    let token = "\"2100-02-28\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 21000228 }.check(encoding(committed), 4, 21000228, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_5() {
    let token = "\"2023-01-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230131 }.check(encoding(committed), 4, 20230131, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_6() {
    let token = "\"2023-02-28\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230228 }.check(encoding(committed), 4, 20230228, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_7() {
    let token = "\"2023-03-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230331 }.check(encoding(committed), 4, 20230331, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_8() {
    let token = "\"2023-04-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230430 }.check(encoding(committed), 4, 20230430, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_9() {
    let token = "\"2023-05-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230531 }.check(encoding(committed), 4, 20230531, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_10() {
    let token = "\"2023-06-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230630 }.check(encoding(committed), 4, 20230630, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_11() {
    let token = "\"2023-07-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230731 }.check(encoding(committed), 4, 20230731, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_12() {
    let token = "\"2023-08-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230831 }.check(encoding(committed), 4, 20230831, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_13() {
    let token = "\"2023-09-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20230930 }.check(encoding(committed), 4, 20230930, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_14() {
    let token = "\"2023-10-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20231031 }.check(encoding(committed), 4, 20231031, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_15() {
    let token = "\"2023-11-30\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20231130 }.check(encoding(committed), 4, 20231130, 64);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn calendar_opening_16() {
    let token = "\"2023-12-31\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    let committed = "\"1998-06-15\"^^<http://www.w3.org/2001/XMLSchema#date>".as_bytes();
    DateLiteral { token, key: 20231231 }.check(encoding(committed), 4, 20231231, 64);
}
