//! Terminal choices are input state. Launching and credentials stay external.
use super::*;

#[cfg(test)]
#[path = "tests/command_history.rs"]
mod command_history_tests;

impl Presenter {
    /// Rotation fills the draft using the currently selected terminal. The token
    /// is exact text, so edits and history changes cannot reuse a stale row index.
    pub(crate) fn rotate_command_history(&mut self, older: bool, draft: &str) -> Option<String> {
        let history = &self.preferences.values.command_history;
        if history.is_empty() {
            self.desktop.command_history_cursor = None;
            return None;
        }
        let position = self
            .desktop
            .command_history_cursor
            .as_deref()
            .filter(|previous| *previous == draft)
            .and_then(|previous| history.iter().position(|entry| entry.command == previous));
        let next = match (position, older) {
            (Some(index), true) => (index + 1) % history.len(),
            (Some(index), false) => (index + history.len() - 1) % history.len(),
            (None, true) => 0,
            (None, false) => history.len() - 1,
        };
        let command = history[next].command.clone();
        self.track_draft(!command.is_empty());
        self.desktop.command_history_cursor = Some(command.clone());
        Some(command)
    }

    pub(crate) fn reset_command_history_cursor(&mut self) {
        self.desktop.command_history_cursor = None;
    }

    /// Recall edits the draft only. It cannot launch or revive elevated permissions.
    pub(crate) fn recall_command(&mut self, token: &str) -> Option<String> {
        self.reset_command_history_cursor();
        let entry = self
            .preferences
            .values
            .command_history
            .iter()
            .find(|entry| entry.command == token)?
            .clone();
        let supported = matches!(
            (self.desktop.shell, entry.shell),
            (Some(ShellFlavor::Posix), ShellFlavor::Posix)
                | (
                    Some(ShellFlavor::PowerShell | ShellFlavor::Cmd),
                    ShellFlavor::PowerShell | ShellFlavor::Cmd
                )
        );
        if !supported {
            self.desktop.report(
                "Command shell unavailable",
                "This command was recorded for a shell unavailable on this platform.".into(),
            );
            return None;
        }
        self.desktop.draft_nonempty = true;
        self.desktop.draft_shell = Some(entry.shell);
        self.desktop.terminal_mode = TerminalMode::Standard;
        self.desktop.terminal_guidance = None;
        Some(entry.command)
    }

    fn preferred_shell(&self) -> Option<ShellFlavor> {
        self.desktop.shell.map(|host| match host {
            ShellFlavor::Posix => ShellFlavor::Posix,
            ShellFlavor::PowerShell | ShellFlavor::Cmd => {
                match self.preferences.values.windows_shell {
                    WindowsShell::PowerShell => ShellFlavor::PowerShell,
                    WindowsShell::Cmd => ShellFlavor::Cmd,
                }
            }
        })
    }

    pub(crate) fn effective_shell(&self) -> Option<ShellFlavor> {
        self.desktop.draft_shell.or_else(|| self.preferred_shell())
    }

    /// Bind a nonempty draft before any settings event or terminal selection can
    /// reinterpret it. Clearing the editor releases the saved default for reuse.
    pub(crate) fn track_draft(&mut self, nonempty: bool) -> bool {
        let previous = (
            self.desktop.draft_nonempty,
            self.desktop.draft_shell,
            self.desktop.terminal_guidance,
        );
        self.desktop.draft_nonempty = nonempty;
        if nonempty {
            if self.desktop.draft_shell.is_none() {
                self.desktop.draft_shell = self.preferred_shell();
            }
        } else {
            self.desktop.draft_shell = None;
            self.desktop.terminal_guidance = None;
        }
        previous
            != (
                self.desktop.draft_nonempty,
                self.desktop.draft_shell,
                self.desktop.terminal_guidance,
            )
    }

    pub(crate) fn effective_terminal_mode(&self) -> TerminalMode {
        if self.effective_shell() == Some(ShellFlavor::Posix) {
            self.desktop.terminal_mode
        } else {
            TerminalMode::Standard
        }
    }

    pub(crate) fn terminal_choices(&self) -> &'static [&'static str] {
        match self.desktop.shell {
            Some(ShellFlavor::PowerShell | ShellFlavor::Cmd) => &["PowerShell", "cmd"],
            Some(ShellFlavor::Posix) => &["Terminal", "Terminal (sudo)"],
            None => &["Terminal"],
        }
    }

    pub(crate) fn terminal_index(&self) -> i32 {
        match self.effective_shell() {
            Some(ShellFlavor::Cmd) => 1,
            Some(ShellFlavor::Posix)
                if self.effective_terminal_mode() == TerminalMode::Elevated =>
            {
                1
            }
            _ => 0,
        }
    }

    pub(crate) fn terminal_hint(&self) -> &'static str {
        match self.effective_shell() {
            Some(ShellFlavor::PowerShell) => "PowerShell · Run in the active panel's folder",
            Some(ShellFlavor::Cmd) => "cmd · Run in the active panel's folder",
            Some(ShellFlavor::Posix)
                if self.effective_terminal_mode() == TerminalMode::Elevated =>
            {
                "Terminal (sudo) · Authentication stays in the external terminal"
            }
            Some(ShellFlavor::Posix) => "Terminal · Run in the active panel's folder",
            None => "Loading terminal choices…",
        }
    }

    pub(crate) fn change_terminal(
        &mut self,
        index: i32,
        has_draft: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.track_draft(has_draft);
        let requested_shell = match self.desktop.shell {
            Some(ShellFlavor::Posix) => ShellFlavor::Posix,
            Some(ShellFlavor::PowerShell | ShellFlavor::Cmd) => {
                if index == 1 {
                    ShellFlavor::Cmd
                } else {
                    ShellFlavor::PowerShell
                }
            }
            None => return,
        };
        if has_draft
            && self
                .desktop
                .draft_shell
                .is_some_and(|bound| bound != requested_shell)
        {
            self.desktop.terminal_guidance =
                Some("Clear the command line before changing shell type.");
            return;
        }
        match self.desktop.shell {
            Some(ShellFlavor::Posix) => {
                self.desktop.terminal_mode = if index == 1 {
                    TerminalMode::Elevated
                } else {
                    TerminalMode::Standard
                };
            }
            Some(ShellFlavor::PowerShell | ShellFlavor::Cmd) => {
                self.preferences.edited.windows_shell = true;
                self.update_preferences(
                    UserPreferences {
                        windows_shell: if index == 1 {
                            WindowsShell::Cmd
                        } else {
                            WindowsShell::PowerShell
                        },
                        ..self.preferences.values.clone()
                    },
                    send,
                );
            }
            None => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn open_terminal_preserves_draft_and_history_with_selected_shell_and_active_path() {
        for (host, index, shell, mode) in [
            (
                ShellFlavor::PowerShell,
                0,
                ShellFlavor::PowerShell,
                TerminalMode::Standard,
            ),
            (
                ShellFlavor::PowerShell,
                1,
                ShellFlavor::Cmd,
                TerminalMode::Standard,
            ),
            (
                ShellFlavor::Posix,
                0,
                ShellFlavor::Posix,
                TerminalMode::Standard,
            ),
            (
                ShellFlavor::Posix,
                1,
                ShellFlavor::Posix,
                TerminalMode::Elevated,
            ),
        ] {
            let right = PathBuf::from("right/文档 👩🏽‍💻");
            let mut p = Presenter::new("left".into(), right.clone());
            p.active = PaneId::Right;
            p.apply(Event::DesktopContext {
                system_locations: Vec::new(),
                network_position: 0,
                shell: host,
                locations: vec![],
            });
            p.change_terminal(index, false, &|_| panic!("a shell choice cannot launch"));
            p.track_draft(true);
            p.desktop.terminal_guidance = Some("Keep this draft bound to its shell.");
            p.preferences
                .values
                .command_history
                .push(CommandHistoryEntry {
                    command: "echo remembered".into(),
                    shell,
                });
            let preferences = p.preferences.values.clone();
            let revision = p.preferences.revision;
            let draft = (
                p.desktop.draft_nonempty,
                p.desktop.draft_shell,
                p.desktop.terminal_guidance,
            );
            let sent = RefCell::new(Vec::new());
            assert!(p.open_terminal(&|command| {
                sent.borrow_mut().push(command);
                true
            }));
            assert_eq!(sent.borrow().len(), 1);
            let request = match &sent.borrow()[0] {
                Command::LaunchTerminal { request, terminal } => {
                    assert!(terminal.command.is_empty());
                    assert_eq!(terminal.shell, shell);
                    assert_eq!(terminal.mode, mode);
                    assert_eq!(terminal.working_directory, right);
                    *request
                }
                other => panic!("unexpected terminal dispatch: {other:?}"),
            };
            p.active = PaneId::Left;
            p.apply(Event::DesktopActionFinished {
                request,
                result: Ok(DesktopActionResult::TerminalStarted {
                    process_id: 1,
                    history: CommandHistoryEntry {
                        command: String::new(),
                        shell,
                    },
                }),
            });
            assert_eq!(
                (
                    p.desktop.draft_nonempty,
                    p.desktop.draft_shell,
                    p.desktop.terminal_guidance,
                ),
                draft
            );
            assert_eq!(p.preferences.values, preferences);
            assert_eq!(p.preferences.revision, revision);
            assert!(p.desktop.terminal_pending.is_none());
            assert!(p.desktop.requests.is_empty());
        }
    }

    #[test]
    fn open_terminal_shares_backpressure_and_pending_guard_with_submission() {
        let mut p = Presenter::new("left".into(), "right".into());
        assert!(!p.open_terminal(&|_| panic!("desktop context is not available")));
        p.apply(Event::DesktopContext {
            system_locations: Vec::new(),
            network_position: 0,
            shell: ShellFlavor::PowerShell,
            locations: vec![],
        });
        for text in ["", " \t\n"] {
            assert!(!p.submit_command(text, &|_| panic!("blank submission cannot launch")));
        }
        assert!(!p.open_terminal(&|_| false));
        assert!(p.desktop.terminal_pending.is_none());
        assert!(p.desktop.requests.is_empty());
        assert!(p.desktop.draft_nonempty);
        assert_eq!(p.desktop.draft_shell, Some(ShellFlavor::PowerShell));
        assert!(p.open_terminal(&|_| true));
        let request = p.desktop.terminal_pending.unwrap();
        assert!(!p.open_terminal(&|_| panic!("pending launch must not repeat")));
        assert!(!p.submit_command("echo draft", &|_| panic!(
            "pending launch blocks submission"
        )));
        p.apply(Event::DesktopActionFinished {
            request: DesktopRequestId(request.0 + 1),
            result: Err(FsError::from_io(
                "launch",
                Path::new(""),
                std::io::ErrorKind::NotFound.into(),
            )),
        });
        assert_eq!(p.desktop.terminal_pending, Some(request));
        p.apply(Event::DesktopActionFinished {
            request,
            result: Err(FsError::from_io(
                "launch",
                Path::new(""),
                std::io::ErrorKind::NotFound.into(),
            )),
        });
        let sent = RefCell::new(Vec::new());
        assert!(p.submit_command("echo draft", &|command| {
            sent.borrow_mut().push(command);
            true
        }));
        assert!(
            matches!(&sent.borrow()[0], Command::LaunchTerminal { terminal, .. }
            if terminal.command == "echo draft" && terminal.shell == ShellFlavor::PowerShell)
        );
        assert!(!p.open_terminal(&|_| panic!("pending submission blocks empty launch")));
    }

    #[test]
    fn bound_draft_defers_loaded_defaults_and_rejects_cross_grammar_selection() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.apply(Event::DesktopContext {
            system_locations: Vec::new(),
            network_position: 0,
            shell: ShellFlavor::PowerShell,
            locations: vec![],
        });
        p.track_draft(true);
        p.apply(Event::PreferencesLoaded(Ok(Box::new(
            midnight_core::AcceptedPreferences {
                preferences: UserPreferences {
                    windows_shell: WindowsShell::Cmd,
                    ..Default::default()
                },
                comparators: midnight_core::FileComparatorSnapshot::default(),
            },
        ))));
        assert_eq!(p.preferences.values.windows_shell, WindowsShell::Cmd);
        assert_eq!(p.effective_shell(), Some(ShellFlavor::PowerShell));
        p.change_terminal(1, true, &|_| {
            panic!("a blocked switch cannot save or launch")
        });
        assert_eq!(p.effective_shell(), Some(ShellFlavor::PowerShell));
        assert!(p.desktop.terminal_guidance.is_some());
        p.track_draft(false);
        assert_eq!(p.effective_shell(), Some(ShellFlavor::Cmd));
        assert!(p.desktop.terminal_guidance.is_none());
    }

    #[test]
    fn input_before_desktop_context_binds_as_soon_as_context_arrives() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.track_draft(true);
        assert_eq!(p.effective_shell(), None);
        p.apply(Event::DesktopContext {
            system_locations: Vec::new(),
            network_position: 0,
            shell: ShellFlavor::PowerShell,
            locations: vec![],
        });
        assert_eq!(p.desktop.draft_shell, Some(ShellFlavor::PowerShell));
        p.apply(Event::PreferencesLoaded(Ok(Box::new(
            midnight_core::AcceptedPreferences {
                preferences: UserPreferences {
                    windows_shell: WindowsShell::Cmd,
                    ..Default::default()
                },
                comparators: midnight_core::FileComparatorSnapshot::default(),
            },
        ))));
        assert_eq!(p.effective_shell(), Some(ShellFlavor::PowerShell));
    }

    #[test]
    fn posix_elevation_is_session_only_and_waits_for_explicit_submission() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.apply(Event::DesktopContext {
            system_locations: Vec::new(),
            network_position: 0,
            shell: ShellFlavor::Posix,
            locations: vec![],
        });
        p.change_terminal(1, true, &|_| {
            panic!("selecting sudo must not save or launch")
        });
        assert_eq!(p.effective_terminal_mode(), TerminalMode::Elevated);
        assert_eq!(p.preferences.revision, PreferencesRevision(0));
        let sent = RefCell::new(vec![]);
        assert!(p.submit_command("printf '文档 👩🏽‍💻'", &|command| {
            sent.borrow_mut().push(command);
            true
        }));
        assert!(
            matches!(&sent.borrow()[0], Command::LaunchTerminal { terminal, .. } if terminal.shell == ShellFlavor::Posix && terminal.mode == TerminalMode::Elevated && terminal.command == "printf '文档 👩🏽‍💻'")
        );
        let mut restarted = Presenter::new("left".into(), "right".into());
        restarted.apply(Event::DesktopContext {
            system_locations: Vec::new(),
            network_position: 0,
            shell: ShellFlavor::Posix,
            locations: vec![],
        });
        assert_eq!(restarted.effective_terminal_mode(), TerminalMode::Standard);
    }

    #[test]
    fn windows_choice_persists_and_wins_over_late_preferences_loading() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.apply(Event::DesktopContext {
            system_locations: Vec::new(),
            network_position: 0,
            shell: ShellFlavor::PowerShell,
            locations: vec![],
        });
        let sent = RefCell::new(vec![]);
        p.change_terminal(1, false, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert_eq!(p.effective_shell(), Some(ShellFlavor::Cmd));
        assert!(sent.borrow().is_empty(), "saves wait for the first load");
        p.apply_with_sender(Event::PreferencesLoaded(Ok(Box::default())), &|command| {
            // The first read also requests comparator image metadata; keep the saves.
            if !matches!(command, Command::ListComparatorCapabilities { .. }) {
                sent.borrow_mut().push(command);
            }
            true
        });
        assert!(matches!(
            &sent.borrow()[0],
            Command::SavePreferences {
                preferences,
                ..
            } if preferences.windows_shell == WindowsShell::Cmd
        ));
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(p.effective_shell(), Some(ShellFlavor::Cmd));
        p.change_preferences(2, 1, -1, &|_| true);
        assert_eq!(p.preferences.values.windows_shell, WindowsShell::Cmd);
        assert_eq!(p.effective_terminal_mode(), TerminalMode::Standard);
    }

    /// A presenter on `host` with the right panel active in a folder whose name would
    /// break out of a quoted shell string.
    fn elevation_presenter(host: ShellFlavor, index: i32) -> (Presenter, PathBuf) {
        let right = PathBuf::from("right/x\" & calc & \" 文档");
        let mut p = Presenter::new("left".into(), right.clone());
        p.active = PaneId::Right;
        p.apply(Event::DesktopContext {
            system_locations: Vec::new(),
            network_position: 0,
            shell: host,
            locations: vec![],
        });
        p.change_terminal(index, false, &|_| panic!("a shell choice cannot launch"));
        (p, right)
    }

    /// Opens a terminal (Ctrl+click when `elevated`) and returns what was sent.
    fn launch(p: &mut Presenter, elevated: bool) -> (DesktopRequestId, TerminalRequest) {
        let sent = RefCell::new(Vec::new());
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        assert!(if elevated {
            p.open_terminal_elevated(&send)
        } else {
            p.open_terminal(&send)
        });
        match sent.into_inner().pop() {
            Some(Command::LaunchTerminal { request, terminal }) => (request, terminal),
            other => panic!("unexpected terminal dispatch: {other:?}"),
        }
    }

    fn finish(
        p: &mut Presenter,
        request: DesktopRequestId,
        result: Result<DesktopActionResult, FsError>,
    ) {
        p.apply(Event::DesktopActionFinished { request, result });
    }

    fn started(shell: ShellFlavor) -> Result<DesktopActionResult, FsError> {
        Ok(DesktopActionResult::TerminalStarted {
            process_id: 1,
            history: CommandHistoryEntry {
                command: String::new(),
                shell,
            },
        })
    }

    #[test]
    fn ctrl_click_opens_the_selected_shell_elevated_and_a_plain_click_as_before() {
        for (host, index, shell, plain) in [
            (
                ShellFlavor::PowerShell,
                0,
                ShellFlavor::PowerShell,
                TerminalMode::Standard,
            ),
            (
                ShellFlavor::PowerShell,
                1,
                ShellFlavor::Cmd,
                TerminalMode::Standard,
            ),
            (
                ShellFlavor::Posix,
                0,
                ShellFlavor::Posix,
                TerminalMode::Standard,
            ),
            // The sudo choice is already elevated; Ctrl+click opens the same.
            (
                ShellFlavor::Posix,
                1,
                ShellFlavor::Posix,
                TerminalMode::Elevated,
            ),
        ] {
            let (mut p, right) = elevation_presenter(host, index);
            for (elevated, mode) in [(false, plain), (true, TerminalMode::Elevated)] {
                let (request, terminal) = launch(&mut p, elevated);
                assert_eq!(
                    (terminal.shell, terminal.mode),
                    (shell, mode),
                    "{host:?} choice {index}, Ctrl {elevated}"
                );
                assert_eq!(terminal.working_directory, right);
                assert!(
                    terminal.command.is_empty(),
                    "an interactive shell, no command"
                );
                assert!(!launch_admitted(&mut p), "one launch at a time");
                finish(&mut p, request, started(shell));
                let opened = if elevated && host == ShellFlavor::PowerShell {
                    "Administrator terminal opened"
                } else {
                    "Terminal opened"
                };
                assert_eq!(p.desktop.status, opened);
            }
            // Ctrl+click neither changes the choice nor makes it elevated.
            assert_eq!(p.effective_shell(), Some(shell));
            assert_eq!(p.effective_terminal_mode(), plain);
        }
    }

    fn launch_admitted(p: &mut Presenter) -> bool {
        p.open_terminal_elevated(&|_| panic!("a pending launch admits no other"))
    }

    #[test]
    fn a_declined_administrator_prompt_is_a_quiet_status_and_other_failures_are_reported() {
        let (mut p, right) = elevation_presenter(ShellFlavor::PowerShell, 1);
        let (request, _) = launch(&mut p, true);
        finish(
            &mut p,
            request,
            Err(midnight_core::terminal_consent_declined(&right)),
        );
        assert_eq!(
            p.desktop.status,
            "The administrator terminal was not started."
        );
        assert!(p.desktop.notice.is_none(), "declining is not an error");
        assert!(p.desktop.terminal_pending.is_none());

        // A failed elevated launch names its cause.
        let (request, _) = launch(&mut p, true);
        finish(
            &mut p,
            request,
            Err(FsError::from_io(
                "start administrator terminal",
                &right,
                std::io::ErrorKind::NotFound.into(),
            )),
        );
        assert!(p.desktop.notice.is_some(), "a failure is reported");

        // Only an elevated launch can end quietly.
        p.desktop.notice = None;
        let (request, _) = launch(&mut p, false);
        finish(
            &mut p,
            request,
            Err(midnight_core::terminal_consent_declined(&right)),
        );
        assert!(p.desktop.notice.is_some());
    }
}
