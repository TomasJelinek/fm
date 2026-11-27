//! Synthetic ELF objects, written at test time, stand in for the browser and its libraries;
//! one of them is a real program that the system's own loader refuses for a missing library.
use super::*;
use std::{
    io::Read,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};

/// This machine's ELF machine number, which every object of one process shares.
const MACHINE: u16 = if cfg!(target_arch = "aarch64") {
    EM_AARCH64
} else {
    EM_X86_64
};
/// 32-bit x86: never loaded into a 64-bit process.
const EM_386: u16 = 3;
const BASE: u64 = 0x40_0000;

/// What a synthetic ELF object declares.
#[derive(Clone, Copy, Default)]
struct Spec<'a> {
    needed: &'a [&'a str],
    soname: Option<&'a str>,
    rpath: Option<&'a str>,
    runpath: Option<&'a str>,
    /// A program with this interpreter; otherwise a shared object.
    interpreter: Option<&'a str>,
    machine: Option<u16>,
}

/// An ELF64 little-endian object: one loaded segment holding the headers, the interpreter's
/// path, the dynamic section and its string table. A program written this way is complete
/// enough for the kernel and glibc's loader to start loading its libraries.
fn elf(spec: Spec<'_>) -> Vec<u8> {
    let mut strings = vec![0u8];
    let mut text = |value: &str| {
        let offset = strings.len() as u64;
        strings.extend_from_slice(value.as_bytes());
        strings.push(0);
        offset
    };
    let mut dynamic: Vec<(u64, u64)> = spec
        .needed
        .iter()
        .map(|name| (DT_NEEDED, text(name)))
        .collect();
    for (tag, value) in [
        (DT_SONAME, spec.soname),
        (DT_RPATH, spec.rpath),
        (DT_RUNPATH, spec.runpath),
    ] {
        if let Some(value) = value {
            dynamic.push((tag, text(value)));
        }
    }
    let interpreter = spec
        .interpreter
        .map(|path| [path.as_bytes(), &[0]].concat());
    let count: u64 = if interpreter.is_some() { 4 } else { 2 };
    let interpreter_at = 64 + count * 56;
    let interpreter_size = interpreter.as_ref().map_or(0, |bytes| bytes.len() as u64);
    let dynamic_at = (interpreter_at + interpreter_size + 7) & !7;
    let entries = dynamic.len() as u64 + 3;
    let strings_at = dynamic_at + entries * 16;
    let total = strings_at + strings.len() as u64;
    dynamic.extend([
        (DT_STRTAB, BASE + strings_at),
        (DT_STRSZ, strings.len() as u64),
    ]);
    dynamic.push((DT_NULL, 0));
    let mut bytes = b"\x7fELF\x02\x01\x01".to_vec();
    bytes.resize(16, 0);
    let kind = if interpreter.is_some() {
        ET_EXEC
    } else {
        ET_DYN
    };
    bytes.extend(kind.to_le_bytes());
    bytes.extend(spec.machine.unwrap_or(MACHINE).to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend((BASE + total).to_le_bytes()); // entry: never reached
    bytes.extend(64u64.to_le_bytes()); // program headers
    bytes.extend(0u64.to_le_bytes()); // no section headers
    bytes.extend(0u32.to_le_bytes());
    for half in [64u16, 56, count as u16, 0, 0, 0] {
        bytes.extend(half.to_le_bytes());
    }
    // Type, flags, file offset (also the offset from BASE), size and alignment.
    let mut segments: Vec<(u32, u32, u64, u64, u64)> = Vec::new();
    if let Some(interpreter) = &interpreter {
        segments.push((6, 4, 64, count * 56, 8)); // PT_PHDR
        segments.push((3, 4, interpreter_at, interpreter.len() as u64, 1)); // PT_INTERP
    }
    segments.push((PT_LOAD, 6, 0, total, 0x1000));
    segments.push((PT_DYNAMIC, 6, dynamic_at, entries * 16, 8));
    for (kind, flags, offset, size, align) in segments {
        bytes.extend(kind.to_le_bytes());
        bytes.extend(flags.to_le_bytes());
        bytes.extend(offset.to_le_bytes());
        bytes.extend((BASE + offset).to_le_bytes());
        bytes.extend((BASE + offset).to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend(align.to_le_bytes());
    }
    bytes.extend(interpreter.unwrap_or_default());
    bytes.resize(dynamic_at as usize, 0);
    for (tag, value) in dynamic {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(strings);
    assert_eq!(bytes.len() as u64, total);
    bytes
}

fn write(path: &Path, bytes: &[u8]) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    path.to_path_buf()
}

fn object_at(path: &Path, spec: Spec<'_>) -> PathBuf {
    write(path, &elf(spec))
}

/// A system whose only default folders are `defaults`, without a cache.
fn system(defaults: &[&Path]) -> System {
    System {
        cache: HashMap::new(),
        defaults: defaults.iter().map(|path| path.to_path_buf()).collect(),
    }
}

/// An `ld.so.cache` in glibc's current format listing `entries`, after an empty old-format
/// cache when `compat`.
fn cache_file(entries: &[(&str, &Path)], compat: bool) -> Vec<u8> {
    let mut new = b"glibc-ld.so.cache1.1".to_vec();
    new.extend((entries.len() as u32).to_ne_bytes());
    let strings_at = 48 + entries.len() * 24;
    let mut strings = Vec::new();
    let mut table = Vec::new();
    for (name, path) in entries {
        let key = (strings_at + strings.len()) as u32;
        strings.extend_from_slice(name.as_bytes());
        strings.push(0);
        let value = (strings_at + strings.len()) as u32;
        strings.extend_from_slice(path.as_os_str().as_bytes());
        strings.push(0);
        table.extend(0x0303i32.to_ne_bytes()); // ELF libc6, x86-64
        table.extend(key.to_ne_bytes());
        table.extend(value.to_ne_bytes());
        table.extend(0u32.to_ne_bytes());
        table.extend(0u64.to_ne_bytes());
    }
    new.extend((strings.len() as u32).to_ne_bytes());
    new.push(if cfg!(target_endian = "little") { 2 } else { 3 });
    new.extend([0; 3]);
    new.extend([0; 16]); // extension offset and unused words
    assert_eq!(new.len(), 48);
    new.extend(table);
    new.extend(strings);
    if !compat {
        return new;
    }
    // An old header with two (ignored) entries: 16 + 24 bytes, then aligned to 8.
    let mut old = b"ld.so-1.7.0\0".to_vec();
    old.extend(2u32.to_ne_bytes());
    old.extend([0; 24]);
    old.extend(new);
    old
}

#[test]
fn needed_libraries_are_looked_up_as_the_loader_looks_them_up() {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("viewers/midnight-viewer-html");
    let lib = root.path().join("system/lib");
    // The browser finds the CEF library and its own copies of system libraries beside it,
    // through its `$ORIGIN` DT_RPATH, which also applies to what CEF needs.
    let browser = object_at(
        &package.join("midnight-viewer-html"),
        Spec {
            needed: &["libcef.so"],
            rpath: Some("$ORIGIN"),
            ..Spec::default()
        },
    );
    object_at(
        &package.join("libcef.so"),
        Spec {
            needed: &[
                "libpresent.so.1",
                "libabsent.so.2",
                "libnested.so.0",
                "libsibling.so.4",
            ],
            ..Spec::default()
        },
    );
    object_at(&package.join("libsibling.so.4"), Spec::default());
    object_at(
        &lib.join("libpresent.so.1"),
        Spec {
            needed: &["libabsent.so.2"],
            ..Spec::default()
        },
    );
    object_at(
        &lib.join("libnested.so.0"),
        Spec {
            needed: &["libdeep.so.3"],
            ..Spec::default()
        },
    );
    assert_eq!(
        missing_with(&browser, &system(&[&lib])),
        ["libabsent.so.2", "libdeep.so.3"],
        "each missing library once, transitive ones included, in the loader's order"
    );
    object_at(&lib.join("libabsent.so.2"), Spec::default());
    object_at(&lib.join("libdeep.so.3"), Spec::default());
    assert!(missing_with(&browser, &system(&[&lib])).is_empty());
}

#[test]
fn a_library_is_satisfied_by_the_soname_of_one_already_loaded() {
    let root = tempfile::tempdir().unwrap();
    let lib = root.path().join("lib");
    // Found under one name, it answers to its soname as well.
    object_at(
        &lib.join("libalias.so"),
        Spec {
            soname: Some("libreal.so.5"),
            ..Spec::default()
        },
    );
    let program = object_at(
        &root.path().join("program"),
        Spec {
            needed: &["libalias.so", "libreal.so.5"],
            ..Spec::default()
        },
    );
    assert!(missing_with(&program, &system(&[&lib])).is_empty());
}

#[test]
fn a_runpath_ends_the_rpath_chain_for_its_own_libraries() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    object_at(&first.join("libtwo.so"), Spec::default());
    let program = |runpath| {
        object_at(
            &first.join("libone.so"),
            Spec {
                needed: &["libtwo.so"],
                runpath,
                ..Spec::default()
            },
        );
        object_at(
            &root.path().join("program"),
            Spec {
                needed: &["libone.so"],
                rpath: Some(first.to_str().unwrap()),
                ..Spec::default()
            },
        )
    };
    // Without its own DT_RUNPATH, libone.so's needs use the program's DT_RPATH.
    assert!(missing_with(&program(None), &System::default()).is_empty());
    // With one, glibc searches only that list for them, not its loaders' DT_RPATH.
    let runpath = second.to_str().unwrap();
    assert_eq!(
        missing_with(&program(Some(runpath)), &System::default()),
        ["libtwo.so"]
    );
}

#[test]
fn the_cache_lists_libraries_outside_the_default_folders() {
    let root = tempfile::tempdir().unwrap();
    let cached = object_at(&root.path().join("opt/lib/libcached.so.7"), Spec::default());
    let program = object_at(
        &root.path().join("program"),
        Spec {
            needed: &["libcached.so.7"],
            ..Spec::default()
        },
    );
    assert_eq!(
        missing_with(&program, &System::default()),
        ["libcached.so.7"]
    );
    for compat in [false, true] {
        let cache = cache_entries(&cache_file(&[("libcached.so.7", &cached)], compat));
        assert_eq!(cache.get("libcached.so.7"), Some(&vec![cached.clone()]));
        let system = System {
            cache,
            defaults: Vec::new(),
        };
        assert!(
            missing_with(&program, &system).is_empty(),
            "compat {compat}"
        );
    }
    // A damaged, truncated or foreign cache lists nothing.
    let mut damaged = cache_file(&[("libcached.so.7", &cached)], false);
    damaged[28] = 1;
    for bytes in [
        damaged,
        cache_file(&[("libcached.so.7", &cached)], false)[..40].to_vec(),
        b"not a cache".to_vec(),
        Vec::new(),
    ] {
        assert!(cache_entries(&bytes).is_empty());
    }
    // Entry offsets past the end are ignored, not followed.
    let mut stray = cache_file(&[("libcached.so.7", &cached)], false);
    stray[48 + 8..48 + 12].copy_from_slice(&u32::MAX.to_ne_bytes());
    assert!(cache_entries(&stray).is_empty());
}

#[test]
fn a_candidate_for_another_machine_is_skipped_as_the_loader_skips_it() {
    let root = tempfile::tempdir().unwrap();
    let foreign = root.path().join("foreign");
    let native = root.path().join("native");
    object_at(
        &foreign.join("libdual.so.1"),
        Spec {
            machine: Some(EM_386),
            ..Spec::default()
        },
    );
    let program = object_at(
        &root.path().join("program"),
        Spec {
            needed: &["libdual.so.1"],
            ..Spec::default()
        },
    );
    assert_eq!(
        missing_with(&program, &system(&[&foreign])),
        ["libdual.so.1"]
    );
    object_at(&native.join("libdual.so.1"), Spec::default());
    assert!(missing_with(&program, &system(&[&foreign, &native])).is_empty());
}

#[test]
fn special_and_damaged_files_are_not_libraries_and_are_never_blocked_on() {
    let root = tempfile::tempdir().unwrap();
    let lib = root.path().join("lib");
    fs::create_dir_all(&lib).unwrap();
    // Opening a FIFO for reading would block until a writer comes; it is never waited for.
    rustix::fs::mknodat(
        rustix::fs::CWD,
        lib.join("libfifo.so.1"),
        rustix::fs::FileType::Fifo,
        Mode::RUSR | Mode::WUSR,
        0,
    )
    .unwrap();
    write(&lib.join("libshort.so.1"), &elf(Spec::default())[..40]);
    write(&lib.join("libtext.so.1"), b"INPUT(-lc)\n");
    fs::create_dir(lib.join("libfolder.so.1")).unwrap();
    // A NEEDED offset past its string table.
    let mut stray = elf(Spec {
        needed: &["libx.so"],
        ..Spec::default()
    });
    // The dynamic section's four entries (NEEDED, STRTAB, STRSZ, NULL) end where the
    // string table starts.
    let at = stray.len() - b"\0libx.so\0".len() - 4 * 16;
    stray[at + 8..at + 16].copy_from_slice(&u64::MAX.to_le_bytes());
    write(&lib.join("libstray.so.1"), &stray);
    let names = [
        "libfifo.so.1",
        "libshort.so.1",
        "libtext.so.1",
        "libfolder.so.1",
        "libstray.so.1",
    ];
    let program = object_at(
        &root.path().join("program"),
        Spec {
            needed: &names,
            ..Spec::default()
        },
    );
    assert_eq!(missing_with(&program, &system(&[&lib])), names);
    // A browser that is no ELF program of its own has no diagnosis.
    assert!(missing_with(&lib.join("libtext.so.1"), &System::default()).is_empty());
    assert!(missing_with(&root.path().join("absent"), &System::default()).is_empty());
}

#[test]
fn origin_is_substituted_and_other_tokens_leave_their_folder_out() {
    let origin = Path::new("/opt/me/viewers/html");
    assert_eq!(
        expand(
            "$ORIGIN:${ORIGIN}/lib:$ORIGIN/../x::relative:/absolute:$LIB/y:${PLATFORM}:$ORIGINAL",
            origin
        ),
        [
            PathBuf::from("/opt/me/viewers/html"),
            PathBuf::from("/opt/me/viewers/html/lib"),
            PathBuf::from("/opt/me/viewers/html/../x"),
            PathBuf::from("/relative"),
            PathBuf::from("/absolute"),
        ]
    );
}

#[test]
fn the_message_names_the_libraries_and_the_command_of_the_distribution() {
    let missing = |names: &[&str]| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    let chromium = missing(&["libnss3.so", "libnssutil3.so", "libatk-1.0.so.0"]);
    let start = "The HTML viewer needs system libraries that are not installed: \
                 libnss3.so, libnssutil3.so, libatk-1.0.so.0.";
    assert_eq!(
        message(&chromium, Family::Debian, false),
        format!("{start} Install them with: sudo apt install libnss3 libatk1.0-0t64")
    );
    assert_eq!(
        message(&chromium, Family::Debian, true),
        format!(
            "{start} Install them with scripts/setup-linux.sh --install in the source \
             checkout, or with: sudo apt install libnss3 libatk1.0-0t64"
        )
    );
    assert_eq!(
        message(&chromium, Family::Fedora, false),
        format!(
            "{start} Install them with: sudo dnf install 'libnss3.so()(64bit)' \
             'libnssutil3.so()(64bit)' 'libatk-1.0.so.0()(64bit)'"
        )
    );
    assert_eq!(
        message(&chromium[..1], Family::Suse, false),
        "The HTML viewer needs system libraries that are not installed: libnss3.so. \
         Install them with: sudo zypper install 'libnss3.so()(64bit)'"
    );
    // No command for an unknown distribution, and none from a checkout's script there.
    for checkout in [false, true] {
        assert_eq!(
            message(&chromium, Family::Other, checkout),
            format!(
                "{start} Install the packages of your distribution that provide them \
                 (see system-requirements.md)."
            )
        );
    }
    // A library without a known Debian package is named for the user to find.
    assert_eq!(
        message(
            &missing(&["libnss3.so", "libunusual.so.9"]),
            Family::Debian,
            false
        ),
        "The HTML viewer needs system libraries that are not installed: libnss3.so, \
         libunusual.so.9. Install them with: sudo apt install libnss3, and the packages \
         that provide libunusual.so.9"
    );
    // Names come from files: anything but a plain soname is masked and never becomes
    // part of a command.
    let odd = missing(&["lib$(touch x)'.so", "libnss3.so"]);
    for family in [Family::Fedora, Family::Debian] {
        let text = message(&odd, family, false);
        assert!(
            text.starts_with(
                "The HTML viewer needs system libraries that are not installed: \
                 lib??touch?x??.so, libnss3.so."
            ),
            "{text}"
        );
        assert!(!text.contains('$') && !text.contains("(touch"), "{text}");
    }
    let many: Vec<String> = (0..15).map(|index| format!("libm{index}.so")).collect();
    assert!(
        message(&many, Family::Other, false).contains("libm11.so, and 3 more."),
        "{}",
        message(&many, Family::Other, false)
    );
    // The libraries without a known package are bounded the same way.
    let mut with_known = vec!["libnss3.so".to_owned()];
    with_known.extend(many);
    let text = message(&with_known, Family::Debian, false);
    assert!(
        text.ends_with(
            "Install them with: sudo apt install libnss3, and the packages that provide \
             libm0.so, libm1.so, libm2.so, libm3.so, libm4.so, libm5.so, libm6.so, libm7.so, \
             libm8.so, libm9.so, libm10.so, libm11.so, and 3 more"
        ),
        "{text}"
    );
}

#[test]
fn os_release_names_the_package_manager() {
    for (text, expected) in [
        (
            "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n",
            Family::Debian,
        ),
        ("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n", Family::Debian),
        ("ID=debian\n", Family::Debian),
        ("ID=fedora\nVERSION_ID=44\n", Family::Fedora),
        (
            "ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n",
            Family::Fedora,
        ),
        (
            "ID=\"opensuse-leap\"\nID_LIKE=\"suse opensuse\"\n",
            Family::Suse,
        ),
        ("ID=opensuse-tumbleweed\n", Family::Suse),
        ("ID=sles\n", Family::Suse),
        ("ID=arch\n", Family::Other),
        ("", Family::Other),
    ] {
        assert_eq!(family(text), expected, "{text:?}");
    }
}

#[test]
fn only_a_development_build_names_the_checkouts_setup_script() {
    let root = tempfile::tempdir().unwrap();
    let checkout_root = root.path().join("checkout with space");
    let executable =
        checkout_root.join("_build/debug/viewers/midnight-viewer-html/midnight-viewer-html");
    assert!(!checkout(&executable), "no setup script");
    write(
        &checkout_root.join("scripts/setup-linux.sh"),
        b"#!/bin/sh\n",
    );
    assert!(checkout(&executable));
    // A package beside `midnight-explorer`, or a build output that is not the checkout's `_build`.
    for other in [
        root.path()
            .join("opt/me/viewers/midnight-viewer-html/midnight-viewer-html"),
        checkout_root.join("target/debug/viewers/midnight-viewer-html/midnight-viewer-html"),
    ] {
        assert!(!checkout(&other), "{}", other.display());
    }
}

#[test]
fn every_library_of_this_running_test_program_is_found() {
    let executable = std::env::current_exe().unwrap();
    assert_eq!(missing(&executable), Vec::<String>::new());
}

/// The loader of this system, if it is where glibc puts it for this machine.
fn loader() -> Option<&'static str> {
    let path = if cfg!(target_arch = "aarch64") {
        "/lib/ld-linux-aarch64.so.1"
    } else {
        "/lib64/ld-linux-x86-64.so.2"
    };
    Path::new(path).exists().then_some(path)
}

#[test]
fn the_loader_stops_a_program_without_its_library_and_the_diagnosis_names_it() {
    let Some(interpreter) = loader() else {
        eprintln!("skipped: no glibc loader at its standard path");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let program = object_at(
        &root
            .path()
            .join("viewers/midnight-viewer-html/midnight-viewer-html"),
        Spec {
            needed: &["libmidnight-absent-test.so.1", "libc.so.6"],
            interpreter: Some(interpreter),
            ..Spec::default()
        },
    );
    // Run it as the browser runs: no environment, the root as working folder.
    let status = Command::new(&program)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(127), "the loader's status: {status:?}");
    let error = diagnose(status, &program).expect("a diagnosis");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    let missing = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<LibrariesMissing>())
        .expect("the typed reason");
    assert_eq!(missing.libraries(), ["libmidnight-absent-test.so.1"]);
    assert!(
        error.to_string().starts_with(
            "The HTML viewer needs system libraries that are not installed: \
             libmidnight-absent-test.so.1."
        ),
        "{error}"
    );
    // Any other end, or a program whose libraries are all there, has no diagnosis.
    let success = Command::new("true").status().unwrap();
    assert!(diagnose(success, &program).is_none());
    let complete = object_at(
        &root.path().join("complete"),
        Spec {
            needed: &["libc.so.6"],
            interpreter: Some(interpreter),
            ..Spec::default()
        },
    );
    assert!(diagnose(status, &complete).is_none());
}

// Run explicitly within a delegated systemd user scope (as the browser runs), for example
// `systemd-run --user --scope -p Delegate=yes cargo test ... -- --ignored`.
#[test]
#[ignore = "requires native Linux delegated user-manager cgroup"]
fn a_browser_that_its_loader_stops_reports_the_missing_library() {
    let interpreter = loader().expect("glibc's loader at its standard path");
    let root = tempfile::tempdir().unwrap();
    let browser_path = object_at(
        &root
            .path()
            .join("viewers/midnight-viewer-html/midnight-viewer-html"),
        Spec {
            needed: &["libmidnight-absent-test.so.1", "libc.so.6"],
            interpreter: Some(interpreter),
            ..Spec::default()
        },
    );
    let profile =
        super::super::super::profile::Profile::new(&root.path().join("profiles")).unwrap();
    let mut browser = super::super::spawn(
        &browser_path,
        profile,
        crate::browser::Extensions::default(),
    )
    .unwrap();
    // The browser's output ends before any message, as the host's transport sees it.
    let mut byte = [0; 1];
    assert_eq!(browser.output.read(&mut byte).unwrap(), 0);
    // What the host's watchdog does once the session stops.
    browser.process.kill().unwrap();
    assert!(!browser.process.wait().unwrap());
    let error = browser
        .process
        .startup_failure()
        .expect("the loader's reason");
    let missing = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<LibrariesMissing>())
        .expect("the typed reason");
    assert_eq!(missing.libraries(), ["libmidnight-absent-test.so.1"]);
}

/// The arguments of every install command in the plan that `scripts/setup-linux.sh
/// --install --dry-run` prints for a distribution, as printed (`Command: [apt-get] ...`).
fn setup_plan(distribution: &str) -> Vec<String> {
    let root = tempfile::tempdir().unwrap();
    let release = write(
        &root.path().join("os-release"),
        format!("ID={distribution}\n").as_bytes(),
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/setup-linux.sh");
    let output = Command::new("sh")
        .arg(script)
        .args(["--install", "--dry-run", "--os-release"])
        .arg(release)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let plan = String::from_utf8(output.stdout).unwrap();
    plan.lines()
        .filter(|line| line.starts_with("Command:") && line.contains("[install]"))
        .flat_map(|line| line.split(['[', ']']).skip(1).step_by(2))
        .map(str::to_owned)
        .collect()
}

/// `scripts/setup-linux.sh --install` installs every library that the message can name:
/// its Debian package with apt, its soname as the package capability with dnf and zypper.
#[test]
fn the_setup_script_installs_every_library_that_the_message_names() {
    let (apt, dnf, zypper) = (
        setup_plan("ubuntu"),
        setup_plan("fedora"),
        setup_plan("opensuse-tumbleweed"),
    );
    assert!(apt.contains(&"apt-get".to_owned()) && zypper.contains(&"zypper".to_owned()));
    for (soname, package) in DEBIAN_PACKAGES {
        assert!(apt.iter().any(|word| word == package), "apt-get: {package}");
        let capability = format!("{soname}()(64bit)");
        assert!(dnf.contains(&capability), "dnf: {capability}");
        assert!(zypper.contains(&capability), "zypper: {capability}");
    }
}
