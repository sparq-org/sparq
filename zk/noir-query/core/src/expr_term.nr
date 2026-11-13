// Retained pass-two work; not wired into lib.nr at the pass-2a checkpoint.
// Research grade; not externally audited.
use crate::expr_core::{check_string, Lane, lane, StringLane, truth};
use crate::expr_term_contract::{
    CheckedLexicalOpening, DT_BYTE, DT_DECIMAL, DT_DOUBLE, DT_FLOAT, DT_INT, DT_INTEGER, DT_LONG,
    DT_NEGATIVEINTEGER, DT_NONNEGATIVEINTEGER, DT_NONPOSITIVEINTEGER, DT_POSITIVEINTEGER, DT_SHORT,
    DT_STRING, DT_UNSIGNEDBYTE, DT_UNSIGNEDINT, DT_UNSIGNEDLONG, DT_UNSIGNEDSHORT, KIND_BLANK_NODE,
    KIND_IRI, KIND_LITERAL, LexicalOpening, read,
};
use std::collections::bounded_vec::BoundedVec;

// Constructors return derived term parts. Encoding these computed parts is the
// term module's separate responsibility; no encoding witness is accepted here.
pub struct TermParts<let L: u32> {
    pub kind: u8,
    pub lexical: StringLane<L>,
    pub datatype: StringLane<L>,
    pub language: StringLane<L>,
}
pub fn empty<let L: u32>() -> StringLane<L> {
    StringLane { bytes: [0; L], len: 0, lang: 0 }
}
pub fn text<let L: u32, let N: u32>(bytes: BoundedVec<u8, N>) -> Lane<StringLane<L>> {
    crate::expr_term_contract::check_bytes(bytes);
    let mut value = empty();
    value.len = if bytes.len() <= L { bytes.len() } else { 0 };
    for i in 0..L {
        if i < bytes.len() {
            value.bytes[i] = bytes.get(i);
        }
    }
    Lane { value, capacity: bytes.len() > L }
}
pub fn from_bytes<let L: u32>(bytes: [u8; L], len: u32) -> StringLane<L> {
    let out = StringLane { bytes, len, lang: 0 };
    check_string(out);
    out
}
pub fn parts<let L: u32>(
    kind: u8,
    lexical: StringLane<L>,
    datatype: StringLane<L>,
    language: StringLane<L>,
) -> TermParts<L> {
    TermParts { kind, lexical, datatype, language }
}
pub fn kind_test<T, let L: u32, let D: u32, let G: u32>(
    a: T,
    error: bool,
    kind: u8,
) -> (Lane<bool>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let a = read(a);
    truth(a.cell.kind == kind, error | !a.cell.bound)
}
pub fn is_iri<T, let L: u32, let D: u32, let G: u32>(a: T, e: bool) -> (Lane<bool>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    kind_test(a, e, KIND_IRI)
}
pub fn is_blank<T, let L: u32, let D: u32, let G: u32>(a: T, e: bool) -> (Lane<bool>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    kind_test(a, e, KIND_BLANK_NODE)
}
pub fn is_literal<T, let L: u32, let D: u32, let G: u32>(a: T, e: bool) -> (Lane<bool>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    kind_test(a, e, KIND_LITERAL)
}
pub fn str_value<T, let L: u32, let D: u32, let G: u32>(
    a: T,
    e: bool,
) -> (Lane<StringLane<L>>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let a = read(a);
    let error = e | !a.cell.bound | ((a.cell.kind != KIND_IRI) & (a.cell.kind != KIND_LITERAL));
    (text(a.lexical), error)
}
pub fn lang<T, let L: u32, let D: u32, let G: u32>(a: T, e: bool) -> (Lane<StringLane<G>>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let a = read(a);
    (text(a.language), e | !a.cell.bound | (a.cell.kind != KIND_LITERAL))
}
pub fn datatype<T, let L: u32, let D: u32, let G: u32>(a: T, e: bool) -> (Lane<TermParts<D>>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let a = read(a);
    let out = text(a.datatype);
    (
        lane(parts(KIND_IRI, out.value, empty(), empty())),
        e | !a.cell.bound | (a.cell.kind != KIND_LITERAL),
    )
}
fn whitespace(b: u8) -> bool {
    (b == 32) | (b == 9) | (b == 10) | (b == 13)
}
fn digit(b: u8) -> bool {
    (b >= 48) & (b <= 57)
}
fn at<let L: u32>(b: BoundedVec<u8, L>, i: u32) -> u8 {
    if i < b.len() {
        b.get(i)
    } else {
        0
    }
}
fn equals<let L: u32, let N: u32>(
    b: BoundedVec<u8, L>,
    start: u32,
    end: u32,
    word: [u8; N],
) -> bool {
    let mut yes = end - start == N;
    for i in 0..N {
        yes = yes & (at(b, start + i) == word[i]);
    }
    yes
}
// Lexical validity is separate from numeric capacity: isNumeric of a valid huge
// integer or a high-precision decimal is true. Facet arithmetic saturates at u64.
pub fn numeric_valid<let L: u32, let D: u32, let G: u32>(a: LexicalOpening<L, D, G>) -> bool {
    let dt = a.cell.dt;
    let integer = (dt == DT_INTEGER)
        | (dt == DT_NONPOSITIVEINTEGER)
        | (dt == DT_NEGATIVEINTEGER)
        | (dt == DT_LONG)
        | (dt == DT_INT)
        | (dt == DT_SHORT)
        | (dt == DT_BYTE)
        | (dt == DT_NONNEGATIVEINTEGER)
        | (dt == DT_UNSIGNEDLONG)
        | (dt == DT_UNSIGNEDINT)
        | (dt == DT_UNSIGNEDSHORT)
        | (dt == DT_UNSIGNEDBYTE)
        | (dt == DT_POSITIVEINTEGER);
    let binary = (dt == DT_FLOAT) | (dt == DT_DOUBLE);
    let mut start: u32 = 0;
    let mut end = a.lexical.len();
    let mut leading = true;
    for i in 0..L {
        if i < a.lexical.len() {
            if leading & whitespace(at(a.lexical, i)) {
                start = i + 1;
            } else {
                leading = false;
            }
        }
    }
    let mut trailing = true;
    for j in 0..L {
        let i = L - 1 - j;
        if (i >= start) & (i < a.lexical.len()) {
            if trailing & whitespace(at(a.lexical, i)) {
                end = i;
            } else {
                trailing = false;
            }
        }
    }
    let negative = at(a.lexical, start) == 45;
    let mut i0 = start;
    if (at(a.lexical, start) == 43) | negative {
        i0 = i0 + 1;
    }
    let mut valid = end > i0;
    let mut digits: u32 = 0;
    let mut exponent_digits: u32 = 0;
    let mut dot = false;
    let mut exp = false;
    let mut nonzero = false;
    let mut magnitude: u128 = 0;
    let mut overflow = false;
    for i in 0..L {
        if (i >= i0) & (i < end) {
            let b = at(a.lexical, i);
            if digit(b) {
                if exp {
                    exponent_digits = exponent_digits + 1;
                } else {
                    digits = digits + 1;
                    nonzero = nonzero | (b != 48);
                    let next = magnitude * 10 + (b - 48) as u128;
                    overflow = overflow | (next > 18446744073709551615);
                    magnitude = if next <= 18446744073709551615 {
                        next
                    } else {
                        0
                    };
                }
            } else if (b == 46) & !dot & !exp & !integer {
                dot = true;
            } else if ((b == 69) | (b == 101)) & binary & !exp & (digits > 0) {
                exp = true;
            } else if ((b == 43) | (b == 45))
                & exp
                & (i > i0)
                & ((at(a.lexical, i - 1) == 69) | (at(a.lexical, i - 1) == 101)) {} else {
                valid = false;
            }
        }
    }
    valid = valid & (digits > 0) & (!exp | (exponent_digits > 0));
    if binary {
        valid = valid
            | equals(a.lexical, start, end, "INF".as_bytes())
            | equals(a.lexical, start, end, "-INF".as_bytes())
            | equals(a.lexical, start, end, "NaN".as_bytes());
    }
    if dt == DT_NONPOSITIVEINTEGER {
        valid = valid & (negative | !nonzero);
    }
    if dt == DT_NEGATIVEINTEGER {
        valid = valid & negative & nonzero;
    }
    if dt == DT_NONNEGATIVEINTEGER {
        valid = valid & (!negative | !nonzero);
    }
    if dt == DT_POSITIVEINTEGER {
        valid = valid & !negative & nonzero;
    }
    let mut bound: u128 = 0;
    if dt == DT_LONG {
        bound = if negative {
            9223372036854775808
        } else {
            9223372036854775807
        };
    }
    if dt == DT_INT {
        bound = if negative { 2147483648 } else { 2147483647 };
    }
    if dt == DT_SHORT {
        bound = if negative { 32768 } else { 32767 };
    }
    if dt == DT_BYTE {
        bound = if negative { 128 } else { 127 };
    }
    if dt == DT_UNSIGNEDLONG {
        bound = 18446744073709551615;
        valid = valid & (!negative | !nonzero);
    }
    if dt == DT_UNSIGNEDINT {
        bound = 4294967295;
        valid = valid & (!negative | !nonzero);
    }
    if dt == DT_UNSIGNEDSHORT {
        bound = 65535;
        valid = valid & (!negative | !nonzero);
    }
    if dt == DT_UNSIGNEDBYTE {
        bound = 255;
        valid = valid & (!negative | !nonzero);
    }
    if bound != 0 {
        valid = valid & !overflow & (magnitude <= bound);
    }
    (integer | binary | (dt == DT_DECIMAL)) & valid
}
pub fn is_numeric<T, let L: u32, let D: u32, let G: u32>(a: T, e: bool) -> (Lane<bool>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let a = read(a);
    truth(
        (a.cell.kind == KIND_LITERAL) & numeric_valid(a),
        e | !a.cell.bound,
    )
}
pub fn lang_matches<let L: u32, let M: u32>(
    a: (Lane<StringLane<L>>, bool),
    b: (Lane<StringLane<M>>, bool),
) -> (Lane<bool>, bool) {
    let (a, ae) = a;
    let (b, be) = b;
    check_string(a.value);
    check_string(b.value);
    let error = ae | be | (a.value.lang != 0) | (b.value.lang != 0);
    let mut nul = false;
    for i in 0..L {
        nul = nul | ((i < a.value.len) & (a.value.bytes[i] == 0));
    }
    for i in 0..M {
        nul = nul | ((i < b.value.len) & (b.value.bytes[i] == 0));
    }
    let result = if !nul {
        sparq_xpath::lang_matches(str::from(a.value.bytes), str::from(b.value.bytes))
    } else {
        let mut prefix = (b.value.len > 0) & (b.value.len <= a.value.len);
        for i in 0..M {
            if i < b.value.len {
                let x = if i < L { a.value.bytes[i] } else { 0 };
                let y = b.value.bytes[i];
                let x = if (x >= 65) & (x <= 90) { x + 32 } else { x };
                let y = if (y >= 65) & (y <= 90) { y + 32 } else { y };
                prefix = prefix & (x == y);
            }
        }
        let mut boundary = a.value.len == b.value.len;
        for i in 0..L {
            boundary =
                boundary | ((i == b.value.len) & (i < a.value.len) & (a.value.bytes[i] == 45));
        }
        let mut first: u8 = 0;
        for i in 0..M {
            if i == 0 {
                first = b.value.bytes[i];
            }
        }
        let wildcard = (b.value.len == 1) & (first == 42);
        (wildcard & (a.value.len > 0)) | (prefix & boundary)
    };
    (Lane { value: result & !error, capacity: a.capacity | b.capacity }, error)
}
fn plain<let L: u32, let D: u32, let G: u32>(a: LexicalOpening<L, D, G>) -> bool {
    (a.cell.kind == KIND_LITERAL) & (a.cell.dt == DT_STRING) & (a.language.len() == 0)
}
pub fn strdt<T, U, let L: u32, let D: u32, let G: u32, let N: u32, let E: u32, let H: u32>(
    a: T,
    b: U,
    error: bool,
) -> (Lane<TermParts<L>>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
    U: CheckedLexicalOpening<N, E, H>,
{
    let a = read(a);
    let b = read(b);
    let dt = text::<L, N>(b.lexical);
    let err = error | !a.cell.bound | !b.cell.bound | !plain(a) | (b.cell.kind != KIND_IRI);
    (
        Lane {
            value: parts(
                KIND_LITERAL,
                text::<L, L>(a.lexical).value,
                dt.value,
                empty(),
            ),
            capacity: dt.capacity,
        }, err,
    )
}
pub fn strlang<T, U, let L: u32, let D: u32, let G: u32, let N: u32, let E: u32, let H: u32>(
    a: T,
    b: U,
    error: bool,
) -> (Lane<TermParts<L>>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
    U: CheckedLexicalOpening<N, E, H>,
{
    let a = read(a);
    let b = read(b);
    let mut language = text::<L, N>(b.lexical);
    let mut valid = b.lexical.len() > 0;
    let mut first = true;
    let mut segment = true;
    for i in 0..N {
        if i < b.lexical.len() {
            let c = b.lexical.get(i);
            let alpha = ((c >= 65) & (c <= 90)) | ((c >= 97) & (c <= 122));
            if c == 45 {
                valid = valid & segment;
                segment = false;
                first = false;
            } else {
                valid = valid & (alpha | (!first & digit(c)));
                segment = true;
            }
        }
    }
    valid = valid & segment;
    for i in 0..L {
        let c = language.value.bytes[i];
        if (i < language.value.len) & (c >= 65) & (c <= 90) {
            language.value.bytes[i] = c + 32;
        }
    }
    let dt = text::<L, 53>(BoundedVec::from_array(
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".as_bytes(),
    ));
    let err = error | !a.cell.bound | !b.cell.bound | !plain(a) | !plain(b) | !valid;
    (
        Lane {
            value: parts(
                KIND_LITERAL,
                text::<L, L>(a.lexical).value,
                dt.value,
                language.value,
            ),
            capacity: dt.capacity | language.capacity,
        }, err,
    )
}

fn put<let L: u32>(out: &mut StringLane<L>, byte: u8, capacity: &mut bool) {
    if out.len < L {
        out.bytes[out.len] = byte;
        out.len = out.len + 1;
    } else {
        *capacity = true;
    }
}
fn copy_range<let L: u32>(
    out: &mut StringLane<L>,
    a: StringLane<L>,
    start: u32,
    end: u32,
    capacity: &mut bool,
) {
    for i in 0..L {
        if (i >= start) & (i < end) {
            put(out, a.bytes[i], capacity);
        }
    }
}
fn hex(c: u8) -> bool {
    digit(c) | ((c >= 65) & (c <= 70)) | ((c >= 97) & (c <= 102))
}
fn reference<let L: u32>(a: StringLane<L>) -> (bool, u32, u32, u32, u32, bool) {
    let mut scheme: u32 = 0;
    let mut path: u32 = 0;
    let mut stop = a.len;
    let mut hash = a.len;
    let mut valid = true;
    let mut can_scheme = true;
    let mut scheme_chars = true;
    for i in 0..L {
        if i < a.len {
            let c = a.bytes[i];
            valid = valid
                & (c > 32)
                & (c != 127)
                & (c != 60)
                & (c != 62)
                & (c != 34)
                & (c != 123)
                & (c != 125)
                & (c != 124)
                & (c != 92)
                & (c != 94)
                & (c != 96);
            if c == 37 {
                let x = if i + 1 < L { a.bytes[i + 1] } else { 0 };
                let y = if i + 2 < L { a.bytes[i + 2] } else { 0 };
                valid = valid & (i + 2 < a.len) & hex(x) & hex(y);
            }
            if (c == 35) & (hash == a.len) {
                hash = i;
            }
            if ((c == 63) | (c == 35)) & (stop == a.len) {
                stop = i;
            }
            if can_scheme {
                let alpha = ((c >= 65) & (c <= 90)) | ((c >= 97) & (c <= 122));
                if c == 58 {
                    valid = valid & scheme_chars & (i > 0);
                    scheme = i + 1;
                    can_scheme = false;
                } else if (c == 47) | (c == 63) | (c == 35) {
                    can_scheme = false;
                } else {
                    scheme_chars = scheme_chars
                        & (alpha | ((i > 0) & (digit(c) | (c == 43) | (c == 45) | (c == 46))));
                }
            }
        }
    }
    let x = if scheme < L { a.bytes[scheme] } else { 0 };
    let y = if scheme + 1 < L {
        a.bytes[scheme + 1]
    } else {
        0
    };
    let authority = (scheme + 1 < a.len) & (x == 47) & (y == 47);
    path = scheme;
    if authority {
        path = stop;
        for j in 0..L {
            let i = L - 1 - j;
            if (i >= scheme + 2) & (i < stop) & (a.bytes[i] == 47) {
                path = i;
            }
        }
    }
    (valid, scheme, path, stop, hash, authority)
}
fn normalized_path<let L: u32>(p: StringLane<L>) -> Lane<StringLane<L>> {
    let mut out = empty();
    let mut cap = false;
    let mut start: u32 = 0;
    for i in 0..L + 1 {
        let slash = if i < L { p.bytes[i] == 47 } else { false };
        if (i <= p.len) & ((i == p.len) | slash) {
            let one = (i == start + 1)
                & (
                    if start < L {
                        p.bytes[start] == 46
                    } else {
                        false
                    }
                );
            let two = (i == start + 2)
                & (
                    if start + 1 < L {
                        (p.bytes[start] == 46) & (p.bytes[start + 1] == 46)
                    } else {
                        false
                    }
                );
            if two {
                let mut length: u32 = 0;
                for j in 0..L {
                    if (j + 1 < out.len) & (out.bytes[j] == 47) {
                        length = j + 1;
                    }
                }
                if (out.len == 1) & (out.bytes[0] == 47) {
                    length = 1;
                }
                out.len = length;
                for j in 0..L {
                    if j >= length {
                        out.bytes[j] = 0;
                    }
                }
            } else if !one {
                copy_range(&mut out, p, start, i, &mut cap);
                if slash {
                    put(&mut out, 47, &mut cap);
                }
            }
            start = i + 1;
        }
    }
    Lane { value: out, capacity: cap }
}
// `base` is the public query base parameter. RFC reference resolution is bounded
// by L, including dot-segment removal; no host-supplied resolved value is used.
pub fn iri<T, let L: u32, let D: u32, let G: u32>(
    a: T,
    base: StringLane<L>,
    error: bool,
) -> (Lane<TermParts<L>>, bool)
where
    T: CheckedLexicalOpening<L, D, G>,
{
    let a = read(a);
    check_string(base);
    let source = text::<L, L>(a.lexical).value;
    let (valid, scheme, path, end, hash, auth) = reference(source);
    let (base_valid, base_scheme, base_path, base_end, base_hash, _) = reference(base);
    let mut out = empty();
    let mut p = empty();
    let mut cap = false;
    let mut query = source;
    let mut query_start = end;
    let mut query_end = hash;
    if scheme > 0 {
        copy_range(&mut out, source, 0, path, &mut cap);
        copy_range(&mut p, source, path, end, &mut cap);
    } else if auth {
        copy_range(&mut out, base, 0, base_scheme, &mut cap);
        copy_range(&mut out, source, 0, path, &mut cap);
        copy_range(&mut p, source, path, end, &mut cap);
    } else {
        copy_range(&mut out, base, 0, base_path, &mut cap);
        if path == end {
            copy_range(&mut p, base, base_path, base_end, &mut cap);
            if end == hash {
                query = base;
                query_start = base_end;
                query_end = base_hash;
            }
        } else {
            let first = if path < L { source.bytes[path] } else { 0 };
            if first != 47 {
                let mut directory = base_path;
                for i in 0..L {
                    if (i >= base_path) & (i < base_end) & (base.bytes[i] == 47) {
                        directory = i + 1;
                    }
                }
                copy_range(&mut p, base, base_path, directory, &mut cap);
                if (base_path == base_end) & (base_path > base_scheme) {
                    put(&mut p, 47, &mut cap);
                }
            }
            copy_range(&mut p, source, path, end, &mut cap);
        }
    }
    let p = normalized_path(p);
    cap = cap | p.capacity;
    copy_range(&mut out, p.value, 0, p.value.len, &mut cap);
    copy_range(&mut out, query, query_start, query_end, &mut cap);
    copy_range(&mut out, source, hash, source.len, &mut cap);
    let err = error
        | !a.cell.bound
        | !((a.cell.kind == KIND_IRI) | plain(a))
        | !valid
        | ((scheme == 0) & (!base_valid | (base_scheme == 0)));
    (Lane { value: parts(KIND_IRI, out, empty(), empty()), capacity: cap }, err)
}
