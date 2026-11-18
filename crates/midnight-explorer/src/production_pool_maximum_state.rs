//! Post-migration maximum of real native-backed Viewer and Compare sessions.
//! Watch, drag and Clipboard use controlled platform edges.
use super::*;

const CLIPBOARD_PROBE: &str = "maximum state fixture";
const ARCHIVE_MEMBER_SIZE: u64 = 22;
// Stored ZIP entry folder/member.txt containing "resident archive lane\n".
const ARCHIVE: &[u8] = &[
    0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x00, 0xb8, 0x9e,
    0xdf, 0x54, 0x16, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x66, 0x6f,
    0x6c, 0x64, 0x65, 0x72, 0x2f, 0x6d, 0x65, 0x6d, 0x62, 0x65, 0x72, 0x2e, 0x74, 0x78, 0x74, 0x72,
    0x65, 0x73, 0x69, 0x64, 0x65, 0x6e, 0x74, 0x20, 0x61, 0x72, 0x63, 0x68, 0x69, 0x76, 0x65, 0x20,
    0x6c, 0x61, 0x6e, 0x65, 0x0a, 0x50, 0x4b, 0x01, 0x02, 0x14, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x21, 0x00, 0xb8, 0x9e, 0xdf, 0x54, 0x16, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00,
    0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x66, 0x6f, 0x6c, 0x64, 0x65, 0x72, 0x2f, 0x6d, 0x65, 0x6d, 0x62, 0x65, 0x72,
    0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
    0x3f, 0x00, 0x00, 0x00, 0x45, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[derive(Default)]
struct ClipboardTrace {
    opened: Mutex<Vec<ThreadId>>,
    reads: Mutex<Vec<ThreadId>>,
    dropped: Mutex<Vec<ThreadId>>,
    writes: AtomicUsize,
}

struct ControlledClipboard(Arc<ClipboardTrace>);

impl ClipboardService for ControlledClipboard {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        let thread = thread::current().id();
        self.0.opened.lock().unwrap().push(thread);
        Ok(Box::new(ClipboardActor {
            trace: self.0.clone(),
            thread,
        }))
    }
}

struct ClipboardActor {
    trace: Arc<ClipboardTrace>,
    thread: ThreadId,
}

impl ClipboardSession for ClipboardActor {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        self.trace.writes.fetch_add(1, Ordering::SeqCst);
        Err(FsError::from_io(
            "maximum state Clipboard fixture",
            Path::new(""),
            io::Error::other("the fixture permits reads only"),
        ))
    }

    fn read_text(&mut self) -> Result<String, FsError> {
        let thread = thread::current().id();
        assert_eq!(thread, self.thread);
        self.trace.reads.lock().unwrap().push(thread);
        Ok(CLIPBOARD_PROBE.to_owned())
    }
}

impl Drop for ClipboardActor {
    fn drop(&mut self) {
        self.trace
            .dropped
            .lock()
            .unwrap()
            .push(thread::current().id());
    }
}

fn clipboard_ready(
    harness: &mut Harness,
    trace: &ClipboardTrace,
    deadline: Instant,
) -> TestResult<ThreadId> {
    let request = DesktopRequestId(51);
    harness.send_until(Command::ReadPathClipboardText { request }, deadline)?;
    let Event::PathClipboardTextRead { result, .. } = harness.event_until(
        |e| matches!(e, Event::PathClipboardTextRead { request: id, .. } if *id == request),
        deadline,
    )?
    else {
        unreachable!()
    };
    let text = result.map_err(|error| error.to_string())?;
    require(
        text.as_ref() == format!("{CLIPBOARD_PROBE}{}", std::path::MAIN_SEPARATOR),
        "controlled Clipboard result did not pass through Engine's literal path policy",
    )?;
    let opened = trace.opened.lock().unwrap();
    let reads = trace.reads.lock().unwrap();
    let [worker] = opened.as_slice() else {
        return Err("expected one controlled Clipboard session".into());
    };
    require(
        reads.as_slice() == [*worker] && trace.dropped.lock().unwrap().is_empty(),
        "controlled Clipboard remained live on its real actor",
    )?;
    eprintln!("controlled Clipboard real actor: open/read thread={worker:?}");
    Ok(*worker)
}

fn tree_and_presence(
    harness: &mut Harness,
    files: &Fixtures,
    listings: &[Listing; 2],
    deadline: Instant,
) -> TestResult<()> {
    for (side, pane) in [PaneId::Left, PaneId::Right].into_iter().enumerate() {
        let listing = &listings[side];
        let root = TreeRootSnapshot {
            pane,
            navigation: listing.navigation,
            entries: listing.entries.clone(),
            index: listing.index.clone(),
        };
        let child = files.panes[side].join("child");
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.path == child && entry.kind == EntryKind::Directory)
            .ok_or("owned subdirectory missing from native listing")?;
        let revision = FolderPresenceRevision(1);
        harness.send_until(
            Command::ProbeFolderPresence(Arc::new(FolderPresenceRequest {
                root: root.clone(),
                revision,
                visible: vec![entry.id],
                cancel: Cancellation::default(),
            })),
            deadline,
        )?;
        let Event::FolderPresence(presence) = harness.event_until(
            |e| {
                matches!(e, Event::FolderPresence(value)
                if value.revision == revision && value.root.same_snapshot(&root))
            },
            deadline,
        )?
        else {
            unreachable!()
        };
        require(
            presence.values.as_ref() == [(entry.id, FolderPresence::HasFolders)],
            "resident folder-presence lane missed the live native child directory",
        )?;
        harness.send_until(
            Command::Tree(Box::new(TreeCommand::Enable {
                root: root.clone(),
                selection_revision: SelectionRevision(1),
                selected: TreeRootSelection::Empty,
            })),
            deadline,
        )?;
        let Event::Tree(event) = harness.event_until(
            |e| {
                matches!(e, Event::Tree(event) if match event.as_ref() {
                    TreeEvent::Published(publication) => publication.view.root.same_snapshot(&root),
                    TreeEvent::Unavailable { root: failed, .. } => failed.same_snapshot(&root),
                    _ => false,
                })
            },
            deadline,
        )?
        else {
            unreachable!()
        };
        let publication = match *event {
            TreeEvent::Published(publication) => publication,
            TreeEvent::Unavailable { error, .. } => return Err(error.as_str().into()),
            _ => unreachable!(),
        };
        require(
            publication.selection_revision == SelectionRevision(1),
            "resident Tree lane published the wrong selection revision",
        )?;
        harness.send_until(
            Command::Tree(Box::new(TreeCommand::Commit(publication.view))),
            deadline,
        )?;
        eprintln!(
            "real Tree and FolderPresence lane results pane={pane:?} navigation={:?}",
            root.navigation
        );
    }
    Ok(())
}

fn qualified_archive_setup(harness: &mut Harness, deadline: Instant) -> TestResult<()> {
    let running = std::env::current_exe();
    let resolved = midnight_plugin_host::installed_executable();
    let supplied = std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION");
    eprintln!(
        "maximum-state archive setup: current_exe={running:?} installed_executable={resolved:?} supplied_root={supplied:?}"
    );

    let root = PathBuf::from(
        supplied.ok_or("maximum-state archive setup: set MIDNIGHT_NATIVE_TEST_INSTALLATION")?,
    );
    require(
        root.is_absolute() && root.is_dir(),
        "maximum-state archive setup: the supplied product root must be an absolute directory",
    )?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("maximum-state archive setup: product root identity: {error}"))?;
    // Path equality can absorb lexical aliases. Qualify the original spelling.
    require(
        root.as_os_str() == canonical_root.as_os_str(),
        "maximum-state archive setup: the supplied product root must already be canonical",
    )?;
    midnight_plugin_host::validate_plugin_path(&root).map_err(|error| {
        format!("maximum-state archive setup: product root protection: {error}")
    })?;

    let running = running.and_then(fs::canonicalize).map_err(|error| {
        format!("maximum-state archive setup: running libtest identity: {error}")
    })?;
    let resolved = resolved
        .map_err(|error| format!("maximum-state archive setup: installed executable: {error}"))?;
    require(
        running.parent().map(Path::as_os_str) == Some(root.as_os_str())
            && resolved.parent().map(Path::as_os_str) == Some(root.as_os_str())
            && running.as_os_str() == resolved.as_os_str(),
        "maximum-state archive setup: the running libtest and installed resolver must select the same executable at the supplied product root",
    )?;
    midnight_plugin_host::validate_plugin_path(&running).map_err(|error| {
        format!("maximum-state archive setup: running libtest protection: {error}")
    })?;

    let Event::ArchiveAvailability(formats) = harness
        .event_until(
            |event| matches!(event, Event::ArchiveAvailability(_)),
            deadline,
        )
        .map_err(|error| {
            format!("maximum-state archive setup: ZIP availability receipt: {error}")
        })?
    else {
        unreachable!()
    };
    eprintln!("maximum-state archive setup: Engine archive formats={formats:?}");
    require(
        formats.contains(&ArchiveFormat::Zip),
        "maximum-state archive setup: Engine did not discover an installed ZIP provider",
    )?;
    require(
        Instant::now() < deadline,
        "maximum-state archive setup: activation deadline expired",
    )
}

fn archive_lane(
    harness: &mut Harness,
    pane: PaneId,
    path: &Path,
    deadline: Instant,
) -> TestResult<()> {
    let navigation = NavigationId(2);
    harness.send_until(
        Command::ArchiveNavigate {
            pane,
            navigation,
            action: ArchiveAction::Open {
                path: path.to_owned(),
                format: ArchiveFormat::Zip,
            },
            sort: SortSpec::default(),
        },
        deadline,
    )?;
    let Event::ArchiveListed { result, .. } = harness.event_until(
        |e| {
            matches!(e, Event::ArchiveListed { pane: p, navigation: n, .. }
            if *p == pane && *n == navigation)
        },
        deadline,
    )?
    else {
        unreachable!()
    };
    let listing = result.map_err(|error| error.to_string())?;
    require(
        listing.location.address.container == path,
        "archive listed a different container",
    )?;
    let mount = listing
        .location
        .mount_id()
        .ok_or("archive has no native capture mount")?;
    let folder = listing
        .entries
        .iter()
        .find_map(|entry| {
            let node = entry.virtual_node.as_ref()?;
            (node.raw_name.as_ref() == b"folder" && node.kind == EntryKind::Directory)
                .then_some(node.id)
        })
        .ok_or("archive folder missing from installed provider listing")?;
    let scan = FolderSizeScanId(1);
    harness.send_until(
        Command::ArchiveSizes(ArchiveSizeCommand::Compute {
            root: TreeRootSnapshot {
                pane,
                navigation,
                entries: listing.entries,
                index: listing.index,
            },
            scan,
            selection: None,
        }),
        deadline,
    )?;
    let mut size_verified = false;
    let mut finished = false;
    while !size_verified || !finished {
        let Event::ArchiveSizes(event) = harness.event_until(
            |e| {
                matches!(e, Event::ArchiveSizes(event) if match event.as_ref() {
                    ArchiveSizeEvent::Snapshot { pane: p, scan: Some(s), .. }
                    | ArchiveSizeEvent::Finished { pane: p, scan: s, .. }
                    | ArchiveSizeEvent::Rejected { pane: p, scan: Some(s), .. }
                        => *p == pane && *s == scan,
                    _ => false,
                })
            },
            deadline,
        )?
        else {
            unreachable!()
        };
        match *event {
            ArchiveSizeEvent::Snapshot { sizes, .. } => {
                require(
                    sizes.mount == mount,
                    "archive snapshot changed mount identity",
                )?;
                if let Some(size) = sizes.get(folder).filter(|size| size.finished) {
                    require(
                        size.bytes == ARCHIVE_MEMBER_SIZE && size.files == 1 && size.errors == 0,
                        "resident archive-size lane did not compute the owned member",
                    )?;
                    size_verified = true;
                }
                harness.send_until(
                    Command::ArchiveSizes(ArchiveSizeCommand::Acknowledge {
                        pane,
                        mount,
                        version: sizes.version,
                    }),
                    deadline,
                )?;
            }
            ArchiveSizeEvent::Finished { cancelled, .. } => {
                require(!cancelled, "real archive-size scan was cancelled")?;
                finished = true;
            }
            ArchiveSizeEvent::Rejected { message, .. } => return Err(message),
            _ => unreachable!(),
        }
    }
    eprintln!("real ArchiveSize lane pane={pane:?} mount={mount} bytes={ARCHIVE_MEMBER_SIZE}");
    Ok(())
}

fn listing_until(
    harness: &mut Harness,
    pane: PaneId,
    navigation: NavigationId,
    path: &Path,
    deadline: Instant,
) -> TestResult<Listing> {
    let event = harness.event_until(|e| matches!(e,
        Event::ListingFinished { pane: p, navigation: n, .. } | Event::ListingFailed { pane: p, navigation: n, .. }
        if *p == pane && *n == navigation), deadline)?;
    let Event::ListingFinished { entries, index, .. } = event else {
        return Err(format!("owned native listing failed for pane {pane:?}"));
    };
    harness.send_until(
        Command::PaneCommitted(VisibleDirectory {
            pane,
            navigation,
            path: path.to_owned(),
            entries: entries.clone(),
        }),
        deadline,
    )?;
    Ok(Listing {
        entries,
        index,
        navigation,
    })
}

fn restored_watch_threads(
    harness: &mut Harness,
    paths: &[PathBuf; 2],
    deadline: Instant,
) -> TestResult<[ThreadId; 2]> {
    loop {
        harness.pump()?;
        {
            let calls = harness.edges.watches.lock().unwrap();
            let started = paths.each_ref().map(|restored| {
                calls
                    .iter()
                    .find(|(path, _)| path == restored)
                    .map(|(_, worker)| *worker)
            });
            if let [Some(left), Some(right)] = started {
                require(
                    left != right,
                    "restored native Watch panes shared one worker",
                )?;
                eprintln!(
                    "real Watch lanes, controlled subscriptions: left={left:?} right={right:?}"
                );
                return Ok([left, right]);
            }
        }
        require(
            Instant::now() < deadline,
            "restored native Watch callbacks did not enter",
        )?;
        thread::sleep(Duration::from_millis(2));
    }
}

fn pressure(
    harness: &mut Harness,
    snapshot: &FileComparatorSnapshot,
    path: &Path,
) -> TestResult<()> {
    let entered_before = harness.settings.prepared_entered.load(Ordering::SeqCst);
    let completed_before = harness.settings.prepared_complete.load(Ordering::SeqCst);
    let mut next = preferences();
    next.file_comparators.rules[0].expression = "*.txt;*.md".into();
    let expected = next.file_comparators.clone();
    let pane = PaneId::Left;
    let navigation = NavigationId(4);
    let revision = PreferencesRevision(3);
    let started = Instant::now();
    let deadline = started + DEADLINE;
    harness.send_until(
        Command::SavePreferences {
            revision,
            preferences: Box::new(next),
        },
        deadline,
    )?;
    harness.send_until(
        Command::Navigate {
            pane,
            navigation,
            path: path.to_owned(),
            sort: SortSpec::default(),
        },
        deadline,
    )?;
    let Event::PreferencesSaved { result, .. } = harness.event_until(
        |e| matches!(e, Event::PreferencesSaved { revision: found, .. } if *found == revision),
        deadline,
    )?
    else {
        unreachable!()
    };
    let accepted =
        result.map_err(|error| format!("maximum-state prepared save failed: {error:?}"))?;
    let published = harness
        .compare_service
        .as_ref()
        .unwrap()
        .comparator_snapshot();
    require(
        accepted.id != snapshot.id
            && accepted.id == published.id
            && Arc::ptr_eq(&accepted.preferences, &published.preferences)
            && *accepted.preferences == expected
            && harness.settings.value.lock().unwrap().file_comparators == expected,
        "changed raw comparator policy was not persisted and published",
    )?;
    require(
        harness.settings.prepared_entered.load(Ordering::SeqCst) == entered_before + 1
            && harness.settings.prepared_complete.load(Ordering::SeqCst) == completed_before + 1
            && harness.settings.unprepared.load(Ordering::SeqCst) == 0,
        "maximum-state save did not enter and complete exactly one real prepared callback",
    )?;
    let listing = listing_until(harness, pane, navigation, path, deadline)?;
    require(
        listing
            .entries
            .iter()
            .any(|entry| entry.path == path.join("pressure.txt") && entry.kind == EntryKind::File),
        "maximum-state native listing did not contain the owned marker",
    )?;
    require(
        Instant::now() < deadline,
        "maximum-state save/navigation budget expired",
    )?;
    eprintln!(
        "maximum-state progress: revision={} navigation={} elapsed_ms={} prepared_entered={}",
        revision.0,
        navigation.0,
        started.elapsed().as_millis(),
        entered_before + 1
    );
    Ok(())
}

fn maximum_state(
    harness: &mut Harness,
    files: &Fixtures,
    archives: &[PathBuf; 2],
    viewer_files: &[PathBuf; 2],
    clipboard: &ClipboardTrace,
) -> TestResult<()> {
    let (snapshot, listings) = harness.prepare_residents(files)?;
    let viewer = harness.viewer_service.as_ref().unwrap();
    viewer.prepare_image_workers(Some(Arc::from([])), Arc::default());
    eprintln!(
        "real native image spare preparation requested; live decoder readiness and worker ID are unavailable"
    );
    let started = Instant::now();
    let deadline = started + DEADLINE;
    qualified_archive_setup(harness, deadline)?;
    let clipboard_worker = clipboard_ready(harness, clipboard, deadline)?;
    tree_and_presence(harness, files, &listings, deadline)?;
    for (pane, path) in [PaneId::Left, PaneId::Right].into_iter().zip(archives) {
        archive_lane(harness, pane, path, deadline)?;
    }
    let restore = NavigationId(3);
    let watch_paths = files.panes.each_ref().map(|pane| pane.join("child"));
    let mut restored = Vec::with_capacity(2);
    for (pane, path) in [PaneId::Left, PaneId::Right].into_iter().zip(&watch_paths) {
        harness.send_until(
            Command::Navigate {
                pane,
                navigation: restore,
                path: path.clone(),
                sort: SortSpec::default(),
            },
            deadline,
        )?;
        restored.push(listing_until(harness, pane, restore, path, deadline)?);
    }
    let watch = restored_watch_threads(harness, &watch_paths, deadline)?;
    let drag = harness.edges.drag_opens.lock().unwrap().clone();
    let reports = harness.reports.entered.lock().unwrap().clone();
    require(
        drag.len() == 1 && reports.len() == 2,
        "drag native or diagnostic lane evidence missing",
    )?;
    let threads: HashSet<_> = watch
        .into_iter()
        .chain([clipboard_worker])
        .chain(drag.iter().copied())
        .chain(reports.iter().copied())
        .collect();
    require(
        threads.len() == 6,
        "observed Engine roles did not use six distinct worker threads",
    )?;
    eprintln!(
        "real Engine callbacks: controlled native drag={drag:?}, diagnostic readers={reports:?}, diagnostic drops={:?}; drag preparation produced a real Prepared event",
        harness.reports.dropped.lock().unwrap()
    );
    for (index, side) in [0, 1, 0, 1].into_iter().enumerate() {
        harness.start_viewer(&restored[side], &viewer_files[side])?;
        harness.viewer_frame_until(index, 400, deadline)?;
        eprintln!("real Viewer frame ready session={index}");
    }
    for index in 0..2 {
        harness.start_compare(files, &snapshot)?;
        harness.compare_frame_until(index, None, deadline)?;
        eprintln!("real Compare frame ready session={index}");
    }
    require(
        clipboard.dropped.lock().unwrap().is_empty()
            && harness.edges.drag_drops.lock().unwrap().is_empty(),
        "controlled Clipboard and native drag actors must remain live at pressure",
    )?;
    eprintln!(
        "setup: configured Engine13/Viewer5/Compare2; six Engine thread IDs observed, other activated roles witnessed by results; four Viewer and two Compare initial frames elapsed_ms={}",
        started.elapsed().as_millis()
    );
    pressure(harness, &snapshot, &watch_paths[0].join("deeper"))?;
    // Check the same six sessions after the idle progress interval.
    let deadline = Instant::now() + DEADLINE;
    for index in 0..4 {
        require(
            harness.viewers[index].send(ViewerInput::Resize(geometry(412))),
            "original Viewer was not live after idle progress",
        )?;
        harness.viewer_frame_until(index, 412, deadline)?;
    }
    for index in 0..2 {
        require(
            harness.compares[index]
                .send(midnight_compare_engine::Input::Resize(geometry(416), 77))
                .is_ok(),
            "original Compare was not live after idle progress",
        )?;
        harness.compare_frame_until(index, Some(77), deadline)?;
    }
    Ok(())
}

#[test]
#[ignore = "SERIAL: post-migration maximum resident state; protected installed native/text/archive/image providers; private MIDNIGHT_POOL_REPRO_WORK"]
fn production_pool_preserves_settings_and_navigation_at_legal_window_maximum() {
    let files = Fixtures::new().expect("owned production pool fixtures");
    let archives = ["maximum-left.zip", "maximum-right.zip"].map(|name| files.root.join(name));
    let viewer_files = files
        .panes
        .each_ref()
        .map(|pane| pane.join("child/view.txt"));
    for ((pane, archive), viewer) in files.panes.iter().zip(&archives).zip(&viewer_files) {
        fs::create_dir_all(pane.join("child/deeper")).expect("owned native subdirectories");
        fs::write(viewer, b"POOL_NATIVE_VIEWER\n").expect("owned viewer input");
        fs::write(
            pane.join("child/deeper/pressure.txt"),
            b"owned progress marker\n",
        )
        .expect("owned marker");
        fs::write(archive, ARCHIVE).expect("owned ZIP");
    }
    let clipboard = Arc::new(ClipboardTrace::default());
    let mut harness = Harness::with_services(|engine, filesystem| {
        engine
            .configure_archive_capture()
            .map_err(|error| error.to_string())?
            .install(crate::native_content::archive(filesystem.clone()));
        engine
            .configure_clipboard()
            .map_err(|error| error.to_string())?
            .install(Arc::new(ControlledClipboard(clipboard.clone())));
        Ok(())
    })
    .expect("production maximum-state composition");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        maximum_state(&mut harness, &files, &archives, &viewer_files, &clipboard)
    }));
    let deadline = Instant::now() + DEADLINE;
    let cleanup = harness.cleanup(deadline);
    let readers = files.check_readers_released(deadline);
    let extra_readers = check_readers(
        viewer_files
            .iter()
            .chain(&archives)
            .map(PathBuf::as_path)
            .chain([files.report.as_path()]),
        deadline,
    );
    let opened = clipboard.opened.lock().unwrap();
    let dropped = clipboard.dropped.lock().unwrap();
    eprintln!(
        "maximum-state cleanup: domains={cleanup:?} native comparison readers={readers:?} viewer/archive/report readers={extra_readers:?} controlled Clipboard open={opened:?} drop={dropped:?}"
    );
    let outcome = match outcome {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    };
    cleanup.expect("real Engine, image spare and external resident pools must retire");
    readers.expect("installed text capture readers must release");
    extra_readers.expect("installed Viewer, archive and diagnostic readers must release");
    assert_eq!(*opened, *dropped, "Clipboard drops must match affine opens");
    assert_eq!(clipboard.writes.load(Ordering::SeqCst), 0);
    outcome.expect("maximum-state role readiness and progress");
}

fn check_readers<'a>(
    paths: impl IntoIterator<Item = &'a Path>,
    deadline: Instant,
) -> TestResult<()> {
    for path in paths {
        loop {
            require(
                Instant::now() < deadline,
                &format!("exclusive reopen deadline elapsed for {}", path.display()),
            )?;
            match OpenOptions::new().read(true).share_mode(0).open(path) {
                Ok(_) => break,
                Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => {
                    return Err(format!(
                        "exclusive reopen {} failed: {error}",
                        path.display()
                    ));
                }
            }
        }
    }
    Ok(())
}
