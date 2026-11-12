//! Observe sandboxed CEF zygote descendants through public /proc state.
use std::{collections::HashSet, fs, path::PathBuf};

pub(super) fn verify_renderer_sandbox(browser: u32) {
    let processes: Vec<_> = fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            let status = fs::read_to_string(entry.path().join("status")).ok()?;
            let parent = status
                .lines()
                .find_map(|line| line.strip_prefix("PPid:"))?
                .trim()
                .parse::<u32>()
                .ok()?;
            // A sandboxed ancestor can prohibit cmdline reads without hiding public PID/PPid.
            let command = fs::read(entry.path().join("cmdline")).unwrap_or_default();
            Some((pid, parent, status, command))
        })
        .collect();
    let mut owned = HashSet::from([browser]);
    loop {
        let before = owned.len();
        for (pid, parent, _, _) in &processes {
            if owned.contains(parent) {
                owned.insert(*pid);
            }
        }
        if before == owned.len() {
            break;
        }
    }
    let mut report = String::new();
    let mut count = 0;
    // CEF fork children can retain the Rust launcher's original zygote argv in /proc. Identify
    // children of the namespaced zygote as well as explicitly titled renderer processes.
    let zygotes: HashSet<_> = processes
        .iter()
        .filter(|(_, _, status, command)| {
            command
                .split(|byte| *byte == 0)
                .any(|arg| arg == b"--type=zygote")
                && status.lines().any(|line| {
                    line.strip_prefix("NSpid:")
                        .is_some_and(|ids| ids.split_whitespace().count() > 1)
                })
        })
        .map(|(pid, _, _, _)| *pid)
        .collect();
    for (pid, parent, status, command) in processes {
        if !owned.contains(&pid)
            || !(zygotes.contains(&parent)
                && status.lines().any(|line| {
                    line.strip_prefix("NSpid:")
                        .is_some_and(|ids| ids.split_whitespace().count() > 2)
                })
                || command
                    .split(|byte| *byte == 0)
                    .any(|arg| arg == b"--type=renderer"))
        {
            continue;
        }
        let value = |name: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .unwrap()
                .trim()
        };
        assert_eq!(
            value("NoNewPrivs:"),
            "1",
            "renderer must prohibit privilege gain"
        );
        assert_eq!(
            value("Seccomp:"),
            "2",
            "renderer must use seccomp filtering"
        );
        assert!(
            value("NSpid:").split_whitespace().count() > 1,
            "renderer needs PID namespace"
        );
        count += 1;
        report.push_str(&format!("sandboxed_zygote_child={pid}\n{status}\n"));
    }
    assert!(
        count > 0,
        "rendering must have an observed sandboxed renderer descendant"
    );
    eprintln!(
        "Observed {count} Linux CEF zygote descendants: namespace, seccomp and NoNewPrivs enabled"
    );
    if let Some(directory) = std::env::var_os("MIDNIGHT_HTML_LINUX_EVIDENCE_DIR") {
        fs::write(
            PathBuf::from(directory).join("renderer-sandbox.txt"),
            report,
        )
        .unwrap();
    }
}
