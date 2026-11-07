use midnight_core::EntryMetadata;

pub(crate) fn explanation(_metadata: &EntryMetadata) -> String {
    String::new()
}

pub(crate) fn fields(metadata: &EntryMetadata) -> Vec<(&'static str, String)> {
    let Some(mode) = metadata.unix_mode else {
        return Vec::new();
    };
    let kind = match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '-',
    };
    let mut text = String::from(kind);
    for (read, write, execute, special, special_on, special_off) in [
        (0o400, 0o200, 0o100, 0o4000, 's', 'S'),
        (0o040, 0o020, 0o010, 0o2000, 's', 'S'),
        (0o004, 0o002, 0o001, 0o1000, 't', 'T'),
    ] {
        text.push(if mode & read != 0 { 'r' } else { '-' });
        text.push(if mode & write != 0 { 'w' } else { '-' });
        text.push(match (mode & execute != 0, mode & special != 0) {
            (true, true) => special_on,
            (false, true) => special_off,
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    text.push_str(&format!(" ({:04o})", mode & 0o7777));
    vec![("Permissions", text)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permissions_include_file_type_octal_and_special_mode_bits() {
        for (mode, expected) in [
            (0o100755, "-rwxr-xr-x (0755)"),
            (0o041777, "drwxrwxrwt (1777)"),
            (0o104644, "-rwSr--r-- (4644)"),
            (0o102750, "-rwxr-s--- (2750)"),
        ] {
            let metadata = EntryMetadata {
                unix_mode: Some(mode),
                ..Default::default()
            };
            assert_eq!(fields(&metadata)[0].1, expected);
        }
        assert!(fields(&EntryMetadata::default()).is_empty());
    }
}
