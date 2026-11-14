// BCP 47 well-formedness for STRLANG. No registry membership is required.
use std::collections::bounded_vec::BoundedVec;
fn alpha(c: u8) -> bool {
    (c >= 97) & (c <= 122)
}
fn digit(c: u8) -> bool {
    (c >= 48) & (c <= 57)
}
fn index(cursor: u32, count: u32) -> u32 {
    if cursor < count {
        cursor
    } else {
        0
    }
}
pub fn valid<let N: u32>(text: BoundedVec<u8, N>) -> bool {
    let mut lens: [u32; N + 1] = [0; N + 1];
    let mut words: [u64; N + 1] = [0; N + 1];
    let mut letters = [true; N + 1];
    let mut digits = [true; N + 1];
    let mut first_digit = [false; N + 1];
    let mut count: u32 = 1;
    let mut shape = text.len() > 0;
    let mut whole: Field = 0;
    for i in 0..N {
        if i < text.len() {
            let byte = text.get_unchecked(i);
            let c = if (byte >= 65) & (byte <= 90) {
                byte + 32
            } else {
                byte
            };
            if i < 11 {
                whole = whole * 256 + c as Field;
            }
            if c == 45 {
                shape = shape & (lens[count - 1] > 0);
                count = count + 1;
            } else {
                let p = count - 1;
                shape = shape & (alpha(c) | digit(c));
                if lens[p] == 0 {
                    first_digit[p] = digit(c);
                }
                if lens[p] < 8 {
                    words[p] = words[p] * 256 + c as u64;
                }
                lens[p] = lens[p] + 1;
                letters[p] = letters[p] & alpha(c);
                digits[p] = digits[p] & digit(c);
            }
        }
    }
    for i in 0..N + 1 {
        if i < count {
            shape = shape & (lens[i] >= 1) & (lens[i] <= 8);
        }
    }
    let grandfathered = (text.len() <= 11) & grandfathered(whole);
    let private = (lens[0] == 1) & (words[0] == 120);
    let mut result = letters[0] & (lens[0] >= 2) & (lens[0] <= 8);
    let mut cursor: u32 = 1;
    if (lens[0] >= 2) & (lens[0] <= 3) {
        for _ in 0..3 {
            let i = index(cursor, count);
            if (cursor < count) & (lens[i] == 3) & letters[i] {
                cursor = cursor + 1;
            }
        }
    }
    let i = index(cursor, count);
    if (cursor < count) & (lens[i] == 4) & letters[i] {
        cursor = cursor + 1;
    }
    let i = index(cursor, count);
    if (cursor < count) & (((lens[i] == 2) & letters[i]) | ((lens[i] == 3) & digits[i])) {
        cursor = cursor + 1;
    }
    let mut variants: [u64; N + 1] = [0; N + 1];
    let mut variant_count: u32 = 0;
    for _ in 0..N {
        let i = index(cursor, count);
        if (cursor < count)
            & (((lens[i] >= 5) & (lens[i] <= 8)) | ((lens[i] == 4) & first_digit[i])) {
            for j in 0..N + 1 {
                if j < variant_count {
                    result = result & (variants[j] != words[i]);
                }
            }
            variants[variant_count] = words[i];
            variant_count = variant_count + 1;
            cursor = cursor + 1;
        }
    }
    let mut seen = [false; 36];
    let mut extension = false;
    let mut pieces: u32 = 0;
    for _ in 0..N {
        let i = index(cursor, count);
        if (cursor < count) & (lens[i] == 1) & (words[i] != 120) {
            result = result & (!extension | (pieces > 0));
            let c = words[i] as u8;
            let slot = if digit(c) {
                (c - 48) as u32
            } else if alpha(c) {
                (c - 97) as u32 + 10
            } else {
                0
            };
            result = result & !seen[slot];
            seen[slot] = true;
            extension = true;
            pieces = 0;
            cursor = cursor + 1;
        } else if (cursor < count) & extension & (lens[i] >= 2) & (lens[i] <= 8) {
            pieces = pieces + 1;
            cursor = cursor + 1;
        }
    }
    result = result & (!extension | (pieces > 0));
    let i = index(cursor, count);
    if (cursor < count) & (lens[i] == 1) & (words[i] == 120) {
        cursor = cursor + 1;
        result = result & (cursor < count);
        for _ in 0..N {
            if cursor < count {
                cursor = cursor + 1;
            }
        }
    }
    result = result & (cursor == count);
    if private {
        result = count > 1;
    }
    shape & (result | grandfathered)
}
fn grandfathered(word: Field) -> bool {
    (word == 0x656e2d67622d6f6564)
        | (word == 0x692d616d69)
        | (word == 0x692d626e6e)
        | (word == 0x692d64656661756c74)
        | (word == 0x692d656e6f636869616e)
        | (word == 0x692d68616b)
        | (word == 0x692d6b6c696e676f6e)
        | (word == 0x692d6c7578)
        | (word == 0x692d6d696e676f)
        | (word == 0x692d6e6176616a6f)
        | (word == 0x692d70776e)
        | (word == 0x692d74616f)
        | (word == 0x692d746179)
        | (word == 0x692d747375)
        | (word == 0x73676e2d62652d6672)
        | (word == 0x73676e2d62652d6e6c)
        | (word == 0x73676e2d63682d6465)
        | (word == 0x6172742d6c6f6a62616e)
        | (word == 0x63656c2d6761756c697368)
        | (word == 0x6e6f2d626f6b)
        | (word == 0x6e6f2d6e796e)
        | (word == 0x7a682d67756f7975)
        | (word == 0x7a682d68616b6b61)
        | (word == 0x7a682d6d696e)
        | (word == 0x7a682d6d696e2d6e616e)
        | (word == 0x7a682d7869616e67)
}
