//! Bounded content sniffing; an extension never overrides the sampled bytes.
pub(crate) fn is_text(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        return true;
    }
    if bytes.contains(&0) {
        return nul_delimited_text(bytes);
    }
    let controls = bytes
        .iter()
        .filter(|&&byte| byte < 32 && !matches!(byte, b'\n' | b'\r' | b'\t' | 12))
        .count();
    controls <= bytes.len() / 20
}

fn nul_delimited_text(bytes: &[u8]) -> bool {
    let zeros = bytes.iter().filter(|&&byte| byte == 0).count();
    // Accept ordinary zero-delimited names/arguments without treating dense
    // zero padding or unmarked UTF-16 code units as UTF-8 text.
    if zeros > bytes.len() / 4 || bytes.windows(3).any(|part| part == [0, 0, 0]) {
        return false;
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => {
            // The fixed sniffing window can stop inside the last UTF-8 scalar.
            match std::str::from_utf8(&bytes[..error.valid_up_to()]) {
                Ok(text) => text,
                Err(_) => return false,
            }
        }
        Err(_) => return false,
    };
    text.chars()
        .all(|value| !value.is_control() || matches!(value, '\0' | '\n' | '\r' | '\t' | '\x0c'))
}

#[cfg(test)]
mod tests {
    use super::is_text;

    #[test]
    fn compiler_flags_and_unicode_zero_delimited_records_are_text() {
        assert!(is_text(b"-IC:\\project\\include\0-DENABLE_FEATURE=1\0"));
        assert!(is_text("obrázky/写真.png\0notes/readme.txt\0".as_bytes()));
        assert!(is_text(b"one\0two\0"));
        assert!(is_text(b"--long-option\0\0--another-option\0"));
    }

    #[test]
    fn null_padding_unmarked_utf16_and_nontext_controls_remain_binary() {
        assert!(!is_text(b"readable header\0\0\0\0"));
        assert!(!is_text(b"a\0b\0c\0d\0"));
        assert!(!is_text(b"readable payload\0\x01binary"));
        assert!(!is_text(b"readable payload\0\xffinvalid utf8"));
        assert!(!is_text(&[0; 16]));
    }

    #[test]
    fn existing_empty_plain_and_bom_text_detection_is_preserved() {
        assert!(is_text(b""));
        assert!(is_text("Příliš žluťoučký\n你好\tworld".as_bytes()));
        assert!(is_text(&[0xff, 0xfe, b'a', 0]));
        assert!(is_text(&[0xfe, 0xff, 0, b'a']));
        assert!(!is_text(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn utf8_sniff_boundary_may_split_only_the_final_scalar() {
        assert!(is_text(b"record-one\0record-two\0\xe4\xb8"));
        assert!(!is_text(b"record-one\0\xe4\xb8\0record-two"));
    }
}
