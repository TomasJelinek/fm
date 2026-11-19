//! Which installed icon theme the desktop uses, and which directories of a theme
//! best fit a requested pixel size. Both only read bounded, fixed locations.
use super::safe_theme_name;
use crate::desktop::read_bounded;
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use x11rb::protocol::xproto::ConnectionExt;

/// A settings blob is a few KiB; one larger than this is not used.
const XSETTINGS_LIMIT: u32 = 256 * 1024;
const XSETTINGS_MAX_ENTRIES: usize = 1024;
/// The desktop setting is asked for at most this often, found or not.
const DESKTOP_THEME_LIFETIME: Duration = Duration::from_secs(5);
/// How long an icon lookup waits for the display to answer.
const DESKTOP_THEME_WAIT: Duration = Duration::from_millis(250);
/// hicolor lists several hundred directories; the lookup bounds its own attempts.
const MAX_DIRECTORIES: usize = 1024;

/// The icon theme of the running desktop, then of the GTK 3 settings file. GTK
/// itself prefers the XSETTINGS manager, and GNOME writes no settings file.
pub(super) fn configured_theme(home: Option<&Path>) -> Option<String> {
    desktop_theme().or_else(|| gtk_settings_theme(&gtk_config_root(home)?))
}

fn desktop_theme() -> Option<String> {
    static CACHED: Mutex<Option<(Instant, Option<String>)>> = Mutex::new(None);
    static ASKING: AtomicBool = AtomicBool::new(false);
    let cached = || {
        CACHED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };
    let known = cached().clone();
    if let Some((read, theme)) = &known
        && read.elapsed() < DESKTOP_THEME_LIFETIME
    {
        return theme.clone();
    }
    // Without an answer in time the previous one stands until the next attempt.
    let theme = answer_within(&ASKING, DESKTOP_THEME_WAIT, xsettings_theme)
        .unwrap_or_else(|| known.and_then(|(_, theme)| theme));
    *cached() = Some((Instant::now(), theme.clone()));
    theme
}

/// Runs `ask` on a thread of its own and waits at most `wait` for its answer.
/// The X protocol has no timeout, so a display that stops answering would
/// otherwise stall every icon lookup; this way it costs each attempt `wait`
/// and holds one parked thread, because nothing is asked while `busy`.
fn answer_within<T: Send + 'static>(
    busy: &'static AtomicBool,
    wait: Duration,
    ask: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    if busy.swap(true, Ordering::AcqRel) {
        return None;
    }
    let (answer, waiting) = mpsc::channel();
    let asked = std::thread::Builder::new()
        .name("icon-theme".into())
        .spawn(move || {
            let value = ask();
            busy.store(false, Ordering::Release);
            let _ = answer.send(value);
        });
    if asked.is_err() {
        busy.store(false, Ordering::Release);
        return None;
    }
    waiting.recv_timeout(wait).ok()
}

/// `Net/IconThemeName` of the default screen's XSETTINGS manager. Wayland sessions
/// serve it through Xwayland; without a display or a manager this is `None`.
fn xsettings_theme() -> Option<String> {
    let (connection, screen) = x11rb::connect(None).ok()?;
    let atom = |name: &[u8]| {
        let atom = connection.intern_atom(true, name).ok()?.reply().ok()?.atom;
        (atom != x11rb::NONE).then_some(atom)
    };
    let selection = atom(format!("_XSETTINGS_S{screen}").as_bytes())?;
    let settings = atom(b"_XSETTINGS_SETTINGS")?;
    let owner = connection
        .get_selection_owner(selection)
        .ok()?
        .reply()
        .ok()?
        .owner;
    if owner == x11rb::NONE {
        return None;
    }
    let reply = connection
        .get_property(false, owner, settings, settings, 0, XSETTINGS_LIMIT / 4)
        .ok()?
        .reply()
        .ok()?;
    (reply.format == 8 && reply.bytes_after == 0)
        .then(|| xsettings_icon_theme(&reply.value))
        .flatten()
}

/// Reads one string setting from an XSETTINGS blob: a byte-order byte, three
/// unused bytes, a serial and a count, then per setting a type, an unused byte,
/// a padded name, a serial and a value (integer, padded string or color).
fn xsettings_icon_theme(data: &[u8]) -> Option<String> {
    let big_endian = match *data.first()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let bytes: [u8; 2] = data.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(usize::from(if big_endian {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        }))
    };
    let u32_at = |at: usize| {
        let bytes: [u8; 4] = data.get(at..at.checked_add(4)?)?.try_into().ok()?;
        usize::try_from(if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
        .ok()
    };
    let padded = |length: usize| length.checked_add(3).map(|length| length & !3);
    let count = u32_at(8)?.min(XSETTINGS_MAX_ENTRIES);
    let mut at = 12usize;
    for _ in 0..count {
        let kind = *data.get(at)?;
        let name_length = u16_at(at.checked_add(2)?)?;
        let name_start = at.checked_add(4)?;
        let name = data.get(name_start..name_start.checked_add(name_length)?)?;
        // The value follows the padded name and the setting's change serial.
        at = name_start
            .checked_add(padded(name_length)?)?
            .checked_add(4)?;
        match kind {
            0 => at = at.checked_add(4)?,
            1 => {
                let length = u32_at(at)?;
                let start = at.checked_add(4)?;
                let value = data.get(start..start.checked_add(length)?)?;
                if name == b"Net/IconThemeName" {
                    return std::str::from_utf8(value)
                        .ok()
                        .filter(|theme| safe_theme_name(theme))
                        .map(String::from);
                }
                at = start.checked_add(padded(length)?)?;
            }
            2 => at = at.checked_add(8)?,
            _ => return None,
        }
    }
    None
}

fn gtk_config_root(home: Option<&Path>) -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(".config")))
}

fn gtk_settings_theme(config_root: &Path) -> Option<String> {
    let input = String::from_utf8(read_bounded(
        &config_root.join("gtk-3.0/settings.ini"),
        64 * 1024,
    )?)
    .ok()?;
    input
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .find_map(|(key, value)| {
            let value = value.trim().trim_matches('"');
            (key.trim() == "gtk-icon-theme-name" && safe_theme_name(value)).then(|| value.into())
        })
}

pub(super) fn manifest_field<'a>(input: &'a str, field: &str) -> Option<&'a str> {
    let mut in_theme = false;
    for line in input.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_theme = line == "[Icon Theme]";
            continue;
        }
        if in_theme
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == field
        {
            return Some(value.trim());
        }
    }
    None
}

/// A theme's directories, best fit for `size` device pixels first: the exact
/// pixel size, then larger ones from the nearest up, then smaller ones from the
/// nearest down, then directories of unknown size. A larger source is scaled
/// down once; only a theme without one is enlarged. Lookup stays PNG-only, so
/// directories that hold scalable artwork simply have no match.
pub(super) fn theme_directories(input: &str, size: u16) -> Vec<String> {
    let declared = directory_pixels(input);
    let mut seen = HashSet::new();
    let mut directories: Vec<&str> = ["Directories", "ScaledDirectories"]
        .into_iter()
        .filter_map(|key| manifest_field(input, key))
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 256
                && Path::new(name)
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
        })
        .take(MAX_DIRECTORIES)
        .filter(|name| seen.insert(*name))
        .collect();
    let wanted = u32::from(size);
    // Stable: directories of one size keep the manifest's order.
    directories.sort_by_key(|name| {
        let pixels = declared.get(name).copied().or_else(|| named_pixels(name));
        match pixels {
            Some((pixels, scale)) if pixels == wanted => (0, 0, scale),
            Some((pixels, scale)) if pixels > wanted => (1, pixels, scale),
            Some((pixels, scale)) => (2, u32::MAX - pixels, scale),
            None => (3, 0, 0),
        }
    });
    directories.into_iter().map(String::from).collect()
}

/// Pixel edge (`Size` times `Scale`) and scale of each directory section.
fn directory_pixels(input: &str) -> HashMap<&str, (u32, u32)> {
    type Section<'a> = Option<(&'a str, Option<u32>, u32)>;
    fn finish<'a>(result: &mut HashMap<&'a str, (u32, u32)>, section: Section<'a>) {
        if let Some((name, Some(size), scale)) = section
            && result.len() < MAX_DIRECTORIES
        {
            result.insert(name, (size * scale, scale));
        }
    }
    let mut result = HashMap::new();
    let mut section: Section<'_> = None;
    for line in input.lines() {
        let line = line.trim();
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            finish(&mut result, section.take());
            section = (name != "Icon Theme").then_some((name, None, 1));
        } else if let Some((_, size, scale)) = section.as_mut()
            && let Some((key, value)) = line.split_once('=')
        {
            let value = value.trim().parse::<u32>().ok();
            match key.trim() {
                "Size" => *size = value.filter(|size| (1..=1024).contains(size)),
                "Scale" => *scale = value.filter(|scale| (1..=8).contains(scale)).unwrap_or(1),
                _ => {}
            }
        }
    }
    finish(&mut result, section);
    result
}

/// `32x32/places` and `32x32@2x/places` name their size where no section does.
fn named_pixels(directory: &str) -> Option<(u32, u32)> {
    let first = directory.split('/').next()?;
    let (edge, scale) = match first.split_once('@') {
        Some((edge, scale)) => (edge, scale.strip_suffix('x')?.parse::<u32>().ok()?),
        None => (first, 1),
    };
    let (width, height) = edge.split_once('x')?;
    let size = width.parse::<u32>().ok()?;
    // Multiplied only once both factors are known to be small.
    (width == height && (1..=1024).contains(&size) && (1..=8).contains(&scale))
        .then(|| (size * scale, scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(kind: u8, name: &[u8], value: &[u8]) -> Vec<u8> {
        let mut bytes = vec![kind, 0];
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(name);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        bytes.extend(7u32.to_le_bytes());
        if kind == 1 {
            bytes.extend((value.len() as u32).to_le_bytes());
        }
        bytes.extend(value);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        bytes
    }

    fn blob(settings: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 0, 0];
        bytes.extend(1u32.to_le_bytes());
        bytes.extend((settings.len() as u32).to_le_bytes());
        bytes.extend(settings.concat());
        bytes
    }

    #[test]
    fn xsettings_theme_is_found_after_other_setting_types_in_either_byte_order() {
        let little = blob(&[
            setting(0, b"Xft/DPI", &98304u32.to_le_bytes()),
            setting(2, b"Net/Background", &[0; 8]),
            setting(1, b"Net/ThemeName", b"Yaru-dark"),
            setting(1, b"Net/IconThemeName", b"Yaru"),
        ]);
        assert_eq!(xsettings_icon_theme(&little).as_deref(), Some("Yaru"));

        let mut big = vec![1, 0, 0, 0];
        big.extend(1u32.to_be_bytes());
        big.extend(1u32.to_be_bytes());
        big.extend([1, 0]);
        big.extend(17u16.to_be_bytes());
        big.extend(b"Net/IconThemeName\0\0\0");
        big.extend(3u32.to_be_bytes());
        big.extend(7u32.to_be_bytes());
        big.extend(b"Adwaita\0");
        assert_eq!(xsettings_icon_theme(&big).as_deref(), Some("Adwaita"));
    }

    #[test]
    fn xsettings_rejects_truncated_oversized_unknown_and_unsafe_input() {
        let valid = blob(&[setting(1, b"Net/IconThemeName", b"Yaru")]);
        for length in 0..valid.len() {
            assert_eq!(xsettings_icon_theme(&valid[..length]), None, "{length}");
        }
        let mut huge = valid.clone();
        // The string length field directly follows the name and serial.
        let at = 12 + 4 + 20 + 4;
        huge[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(xsettings_icon_theme(&huge), None);
        assert_eq!(
            xsettings_icon_theme(&blob(&[setting(9, b"Net/IconThemeName", b"Yaru")])),
            None
        );
        for unsafe_name in [&b"../evil"[..], b"", b"a/b", b"\xff\xfe"] {
            assert_eq!(
                xsettings_icon_theme(&blob(&[setting(1, b"Net/IconThemeName", unsafe_name)])),
                None
            );
        }
        // The wanted name carrying a number, and a name that is not text before it.
        assert_eq!(
            xsettings_icon_theme(&blob(&[setting(
                0,
                b"Net/IconThemeName",
                &7u32.to_le_bytes()
            )])),
            None
        );
        assert_eq!(
            xsettings_icon_theme(&blob(&[
                setting(1, b"\xff\xfe/Odd", b"value"),
                setting(1, b"Net/IconThemeName", b"Yaru"),
            ]))
            .as_deref(),
            Some("Yaru")
        );
        let mut claimed = blob(&[]);
        claimed[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(xsettings_icon_theme(&claimed), None);
        assert_eq!(
            xsettings_icon_theme(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            None
        );
    }

    #[test]
    fn theme_directories_are_bounded_relative_names_only() {
        let input = "[Icon Theme]\nDirectories=../evil,/tmp/evil,16x16/mimetypes,32x32/places\n[Other]\nDirectories=outside\n";
        assert_eq!(
            theme_directories(input, 32),
            ["32x32/places", "16x16/mimetypes"]
        );
    }

    /// Directory naming and sections as Yaru ships them.
    const SIZE_FIRST: &str = "[Icon Theme]\nName=Sample\n\
        Directories=16x16/places,16x16@2x/places,24x24/places,24x24@2x/places,32x32/places,48x48/places,48x48@2x/places,256x256/places,scalable/places,16x16/places\n\
        [16x16/places]\nSize=16\nType=Fixed\n\
        [16x16@2x/places]\nScale=2\nSize=16\nType=Fixed\n\
        [24x24/places]\nSize=24\nType=Fixed\n\
        [24x24@2x/places]\nScale=2\nSize=24\nType=Fixed\n\
        [32x32/places]\nSize=32\nType=Fixed\n\
        [48x48/places]\nSize=48\nType=Fixed\n\
        [48x48@2x/places]\nScale=2\nSize=48\nType=Fixed\n\
        [256x256/places]\nSize=256\nType=Fixed\n\
        [scalable/places]\nSize=16\nMinSize=16\nMaxSize=256\nType=Scalable\n";

    #[test]
    fn directories_rank_exact_pixels_then_nearest_larger_then_nearest_smaller() {
        let first = |size| theme_directories(SIZE_FIRST, size);
        assert_eq!(first(16)[..2], ["16x16/places", "scalable/places"]);
        // Among equal pixel sizes, one-to-one artwork precedes the doubled smaller design.
        assert_eq!(
            first(20)[..4],
            [
                "24x24/places",
                "32x32/places",
                "16x16@2x/places",
                "48x48/places"
            ]
        );
        assert_eq!(first(32)[..2], ["32x32/places", "16x16@2x/places"]);
        assert_eq!(first(48)[..2], ["48x48/places", "24x24@2x/places"]);
        assert_eq!(first(96)[..2], ["48x48@2x/places", "256x256/places"]);
        assert_eq!(
            first(300)[..3],
            ["256x256/places", "48x48@2x/places", "48x48/places"]
        );
        let all = first(20);
        assert_eq!(all.len(), 9, "the repeated directory is listed once");
        assert_eq!(all[7..], ["16x16/places", "scalable/places"]);
    }

    #[test]
    fn name_first_and_scaled_directories_use_their_sections() {
        // Humanity names directories by context and lists scaled ones separately.
        let input = "[Icon Theme]\nDirectories=places/16,places/24,places/48\n\
            ScaledDirectories=places@2/16,places@2/24\n\
            [places/16]\nSize=16\n[places/24]\nSize=24\n[places/48]\nSize=48\n\
            [places@2/16]\nScale=2\nSize=16\n[places@2/24]\nScale=2\nSize=24\n";
        assert_eq!(
            theme_directories(input, 32),
            [
                "places@2/16",
                "places/48",
                "places@2/24",
                "places/24",
                "places/16"
            ]
        );
        let undeclared = "[Icon Theme]\nDirectories=misc,24x24/apps,odd@2x/apps,8x8@2x/apps\n";
        assert_eq!(
            theme_directories(undeclared, 16),
            ["8x8@2x/apps", "24x24/apps", "misc", "odd@2x/apps"]
        );
        // Sizes too large to multiply are unknown sizes, not a panic.
        let huge = "[Icon Theme]\nDirectories=4294967295x4294967295@4294967295x/apps,16x16/apps,4000000000x4000000000/apps\n";
        assert_eq!(theme_directories(huge, 16)[0], "16x16/apps");
    }

    #[test]
    fn invalid_declared_sizes_rank_as_unknown() {
        let input = "[Icon Theme]\nDirectories=a,b,c\n\
            [a]\nSize=0\n[b]\nSize=99999\n[c]\nSize=24\nScale=99\n";
        assert_eq!(theme_directories(input, 24), ["c", "a", "b"]);
    }

    #[test]
    fn gtk_settings_theme_reads_only_a_safe_configured_name() {
        let config = tempfile::tempdir().unwrap();
        let directory = config.path().join("gtk-3.0");
        assert_eq!(gtk_settings_theme(config.path()), None, "no settings file");
        std::fs::create_dir_all(&directory).unwrap();
        let read = |text: &str| {
            std::fs::write(directory.join("settings.ini"), text).unwrap();
            gtk_settings_theme(config.path())
        };
        assert_eq!(
            read("[Settings]\ngtk-icon-theme-name = \"Papirus\"\n").as_deref(),
            Some("Papirus")
        );
        assert_eq!(read("[Settings]\ngtk-icon-theme-name=../evil\n"), None);
        assert_eq!(read("[Settings]\ngtk-theme-name=Yaru\n"), None);
    }

    #[test]
    fn a_question_without_an_answer_costs_only_the_wait_and_is_not_repeated() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        assert_eq!(
            answer_within(&BUSY, Duration::from_secs(10), || 7),
            Some(7),
            "an answer in time"
        );
        // The asked party hangs until the test lets it go.
        let (release, held) = mpsc::channel::<()>();
        let started = Instant::now();
        let hung = answer_within(&BUSY, Duration::from_millis(20), move || {
            let _ = held.recv();
            8
        });
        assert_eq!(hung, None);
        assert!(started.elapsed() < Duration::from_secs(5));
        // While it hangs nothing else is asked, so threads do not pile up.
        assert_eq!(
            answer_within(&BUSY, Duration::from_secs(10), || -> i32 {
                panic!("asked a display that is still silent")
            }),
            None
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while BUSY.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "the hung question never ended");
            std::thread::yield_now();
        }
        assert_eq!(answer_within(&BUSY, Duration::from_secs(10), || 9), Some(9));
    }
}
