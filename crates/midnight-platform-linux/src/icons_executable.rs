//! Installed application metadata lookup for an exact configured executable.
//! Desktop entries are data: no command, field substitution or shell is evaluated.
use midnight_core::{Cancellation, IconPixels};
use std::{
    ffi::OsStr,
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

const APPLICATION_ROOTS: &[&str] = &["/usr/share/applications", "/usr/local/share/applications"];
const MAX_ENTRIES: usize = 512;
const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_PATH_ENTRIES: usize = 64;

pub(super) fn load(
    home: Option<&Path>,
    executable: &Path,
    size: u16,
    cancel: &Cancellation,
) -> Option<IconPixels> {
    if cancel.is_cancelled() || !(1..=64).contains(&size) {
        return None;
    }
    let roots: Vec<_> = APPLICATION_ROOTS.iter().map(Path::new).collect();
    let search_path = std::env::var_os("PATH").unwrap_or_default();
    let name = icon_name(executable, &roots, &search_path, cancel)?;
    let icon = super::load_named_icon(home, &[&name], size, cancel);
    if cancel.is_cancelled() { None } else { icon }
}

fn icon_name(
    executable: &Path,
    roots: &[&Path],
    search_path: &OsStr,
    cancel: &Cancellation,
) -> Option<String> {
    if cancel.is_cancelled() || !executable.is_absolute() {
        return None;
    }
    let executable = canonical_executable(executable, cancel)?;
    let mut count = 0;
    let mut bytes = 0;
    let mut found = None;
    // Scan only direct entries in fixed system directories, never user XDG roots
    // or arbitrary directories adjacent to a supplied executable.
    for root in roots {
        if cancel.is_cancelled() {
            return None;
        }
        let canonical_root = match root.canonicalize() {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            // An inaccessible or cyclic root cannot be treated as an empty catalog.
            Err(_) => return None,
        };
        if !roots.iter().any(|root| canonical_root.starts_with(root)) {
            continue;
        }
        // An existing but unreadable catalog may hide a conflicting mapping.
        let entries = fs::read_dir(&canonical_root).ok()?;
        for entry in entries {
            if cancel.is_cancelled() {
                return None;
            }
            count += 1;
            if count > MAX_ENTRIES {
                // A partial enumeration cannot rule out a conflicting icon mapping.
                return None;
            }
            let entry = entry.ok()?;
            if entry.path().extension() != Some(OsStr::new("desktop")) {
                continue;
            }
            let Ok(path) = entry.path().canonicalize() else {
                continue;
            };
            if !path.starts_with(&canonical_root) {
                continue;
            }
            let Ok(metadata) = path.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            // Leave one byte for the bounded reader's oversize probe. The existing
            // descriptor reader also verifies regular-file type after opening.
            let limit = MAX_FILE_BYTES.min((MAX_TOTAL_BYTES - bytes).saturating_sub(1));
            if limit == 0 {
                return None;
            }
            // Incomplete trusted metadata may contain another exact match; do not
            // publish an icon from the readable subset of the installed catalog.
            let input = crate::desktop::read_bounded(&path, limit)?;
            bytes += input.len();
            if cancel.is_cancelled() {
                return None;
            }
            let Some(application) = std::str::from_utf8(&input).ok().and_then(parse_entry) else {
                continue;
            };
            if resolve_program(&application.program, search_path, cancel).as_ref()
                != Some(&executable)
            {
                continue;
            }
            if let Some(required) = &application.try_exec
                && resolve_program(required, search_path, cancel).as_ref() != Some(&executable)
            {
                continue;
            }
            if found.as_ref().is_some_and(|name| name != &application.icon) {
                return None;
            }
            found = Some(application.icon);
        }
    }
    if cancel.is_cancelled() { None } else { found }
}

fn canonical_executable(path: &Path, cancel: &Cancellation) -> Option<PathBuf> {
    if cancel.is_cancelled() || path.as_os_str().as_bytes().contains(&0) {
        return None;
    }
    let path = path.canonicalize().ok()?;
    let metadata = path.metadata().ok()?;
    (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0).then_some(path)
}

fn resolve_program(program: &str, search_path: &OsStr, cancel: &Cancellation) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return canonical_executable(path, cancel);
    }
    if path.components().count() != 1
        || program.contains('/')
        || search_path.as_bytes().len() > MAX_PATH_BYTES
    {
        return None;
    }
    for directory in std::env::split_paths(search_path).take(MAX_PATH_ENTRIES) {
        if cancel.is_cancelled() {
            return None;
        }
        // Relative/empty PATH entries would add panel/CWD authority to artwork lookup.
        if directory.is_absolute()
            && let Some(path) = canonical_executable(&directory.join(path), cancel)
        {
            return Some(path);
        }
    }
    None
}

struct Application {
    program: String,
    try_exec: Option<String>,
    icon: String,
}
fn parse_entry(input: &str) -> Option<Application> {
    if input.contains('\0') {
        return None;
    }
    let (mut kind, mut command, mut try_exec, mut icon, mut hidden) =
        (None, None, None, None, None);
    let mut in_entry = false;
    let mut seen_entry = false;
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            if in_entry && std::mem::replace(&mut seen_entry, true) {
                return None;
            }
            continue;
        }
        if !in_entry {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        let target = match key.trim() {
            "Type" => &mut kind,
            "Exec" => &mut command,
            "TryExec" => &mut try_exec,
            "Icon" => &mut icon,
            "Hidden" => &mut hidden,
            _ => continue,
        };
        if target.replace(value.trim()).is_some() {
            return None;
        }
    }
    if kind != Some("Application") || hidden.is_some_and(|value| value != "false") {
        return None;
    }
    let program = parse_exec(&unescape(command?)?)?;
    let try_exec = match try_exec {
        Some(value) => Some(unescape(value)?),
        None => None,
    };
    if try_exec
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
    {
        return None;
    }
    let icon = unescape(icon?)?;
    let icon = icon.strip_suffix(".png").unwrap_or(&icon);
    super::safe_theme_name(icon).then(|| Application {
        program,
        try_exec,
        icon: icon.into(),
    })
}

// String escapes are decoded before the desktop Exec quoting grammar. This does
// not implement shell syntax; unsupported/malformed metadata simply has no icon.
fn unescape(value: &str) -> Option<String> {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        result.push(if character == '\\' {
            match chars.next()? {
                's' => ' ',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                _ => return None,
            }
        } else {
            character
        });
    }
    Some(result)
}
fn parse_exec(value: &str) -> Option<String> {
    let mut chars = value.chars().peekable();
    let mut program = None;
    while chars.peek().is_some() {
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let quoted = chars.peek() == Some(&'"');
        if quoted {
            chars.next();
        }
        let mut token = String::new();
        let mut closed = !quoted;
        while let Some(character) = chars.next() {
            if character.is_control() {
                return None;
            }
            if quoted && character == '"' {
                closed = true;
                break;
            }
            if !quoted && character == ' ' {
                break;
            }
            if quoted && character == '\\' {
                let escaped = chars.next()?;
                if !matches!(escaped, '"' | '\\' | '$' | '`') {
                    return None;
                }
                token.push(escaped);
            } else {
                if (quoted && matches!(character, '$' | '`'))
                    || (!quoted && "\"'\\><~|&;$*?#()`".contains(character))
                {
                    return None;
                }
                token.push(character);
            }
        }
        if !closed || (quoted && chars.peek().is_some_and(|character| *character != ' ')) {
            return None;
        }
        let mut decoded = String::new();
        let mut fields = token.chars();
        while let Some(character) = fields.next() {
            if character != '%' {
                decoded.push(character);
                continue;
            }
            let code = fields.next()?;
            if code == '%' {
                decoded.push('%');
            } else if quoted || program.is_none() || !"fFuUick".contains(code) {
                return None;
            }
        }
        if program.is_none() {
            if decoded.is_empty() || decoded.contains('=') {
                return None;
            }
            program = Some(decoded);
        }
    }
    program
}

#[cfg(test)]
#[path = "icons_executable_tests.rs"]
mod tests;
