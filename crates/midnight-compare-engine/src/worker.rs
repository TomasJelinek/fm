use crate::discovery::{Deliver, FixedPitchFonts, ImageViewers};
use crate::file_choice::{self, Picker, resolve};
use crate::{
    Command, ExternalCompareLauncher, FileChoice, IdentityVerdict, Inbox, Input, Mailbox, Message,
    NativeCapture, Output, OwnedFileComparatorPolicy, StorageHint, check_cancel,
    external_compare::{self, Attempt},
    identity::{IdentityTurn, WholeInput},
    images::Images,
    revalidation,
    routing::{self, BuiltInPlan, BuiltInTarget, Target},
    text::TextComparison,
};
use midnight_core::{
    Cancellation, ExternalCompareSessionCredit, FileCompareLaunch, ViewerAppearance, ViewerGeometry,
};
use midnight_plugin_host::CapturedFile;
use std::{
    sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::{Receiver, RecvTimeoutError, TryRecvError},
    },
    time::Duration,
};

enum Content {
    Text(Box<TextComparison>),
    Images(Box<Images>),
}

impl Content {
    fn identity_turn(
        &mut self,
        sources: &[CapturedFile; 2],
        confirmed_ssd: bool,
    ) -> Option<WholeInput> {
        match self {
            Self::Text(text) => text
                .identity_turn()
                .map(|equal| WholeInput::new(sources, Some(equal), confirmed_ssd)),
            Self::Images(_) => Some(WholeInput::new(sources, None, confirmed_ssd)),
        }
    }

    /// The retained text; images retain decoded pixels, not their source bytes.
    fn text(&self) -> Option<&TextComparison> {
        match self {
            Self::Text(text) => Some(text),
            Self::Images(_) => None,
        }
    }

    fn describe_identity(&self, identity: IdentityVerdict, status: &mut String) {
        let label = match identity {
            IdentityVerdict::Pending => "Whole-input identity pending",
            IdentityVerdict::IdenticalFiles => "Files are identical (every raw byte compared)",
            IdentityVerdict::IdenticalText => "Text is identical (raw file bytes differ)",
            IdentityVerdict::Different => "Inputs differ",
        };
        status.push_str(" · ");
        status.push_str(label);
        if let Self::Text(text) = self
            && let Some([left, right]) = text.encodings
        {
            status.push_str(" · File A ");
            status.push_str(left.label());
            status.push_str(" · File B ");
            status.push_str(right.label());
        }
    }
}

struct Viewport {
    geometry: ViewerGeometry,
    revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputPoll {
    NonBlocking,
    Wait(Duration),
    Block,
}

impl InputPoll {
    fn after_turn(turn: IdentityTurn, identity_ready: bool) -> Self {
        match (identity_ready, turn) {
            (true, IdentityTurn::Advanced) => Self::NonBlocking,
            (true, IdentityTurn::Blocked) => Self::Wait(Duration::from_millis(10)),
            _ => Self::Block,
        }
    }

    fn receive<T>(self, receiver: &Receiver<T>) -> Result<T, RecvTimeoutError> {
        match self {
            Self::NonBlocking => receiver.try_recv().map_err(|error| match error {
                TryRecvError::Empty => RecvTimeoutError::Timeout,
                TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
            }),
            Self::Wait(delay) => receiver.recv_timeout(delay),
            Self::Block => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        }
    }
}

pub(crate) struct Capture<'a> {
    pub factory: &'a NativeCapture,
    pub storage: Option<&'a StorageHint>,
    pub viewers: &'a ImageViewers,
    pub fonts: &'a Arc<FixedPitchFonts>,
    pub inbox: &'a Arc<Inbox>,
    pub cancel: &'a Arc<AtomicBool>,
    #[cfg(test)]
    pub turns: Option<&'a std::sync::atomic::AtomicUsize>,
}

pub(crate) struct Context<'a> {
    pub receiver: &'a Receiver<Message>,
    pub output: &'a Mailbox,
    pub capture: Capture<'a>,
    pub policy: Option<Box<dyn OwnedFileComparatorPolicy>>,
    pub launcher: Option<Arc<dyn ExternalCompareLauncher>>,
    pub credit: Arc<dyn ExternalCompareSessionCredit>,
    pub cancellation: Cancellation,
}

impl Capture<'_> {
    /// Leaves the reply to font catalog request `request` in this session's inbox and
    /// wakes the worker. It runs on the scan thread and never waits for room in the queue.
    fn font_delivery(&self, request: u64) -> Deliver {
        let inbox = Arc::clone(self.inbox);
        Box::new(move |families| inbox.answer(request, families))
    }

    #[cfg(test)]
    fn turn(&self) {
        if let Some(turns) = self.turns {
            turns.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
    }
    fn open(&self, path: &std::path::Path) -> Result<CapturedFile, String> {
        check_cancel(self.cancel)?;
        let result = (self.factory)(path, self.cancel);
        check_cancel(self.cancel)?;
        result
    }

    /// Only a positive local-SSD answer admits this source's reads as shared I/O.
    fn confirmed_ssd(&self, path: &std::path::Path) -> bool {
        self.storage.is_some_and(|storage| storage(path))
    }

    fn image_providers(
        &self,
        output: &Mailbox,
        generation: u64,
    ) -> Result<Arc<[midnight_image_viewer_host::Provider]>, String> {
        self.viewers
            .get(|| {
                output.put(
                    generation,
                    Output::Status("Checking installed image viewers…".into()),
                );
            })
            .map_err(|error| {
                let reason = format!("Installed image viewers could not be checked: {error}");
                output.put(generation, Output::Status(reason.clone()));
                reason
            })
    }
}

#[cfg(test)]
#[path = "worker_schedule_tests.rs"]
mod schedule_tests;
#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;

pub(crate) fn run(
    launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    mut generation: u64,
    context: Context<'_>,
    picker: Picker,
) -> Result<(), String> {
    let output = context.output;
    let cancel = context.capture.cancel;
    check_cancel(cancel)?;
    output.put(
        generation,
        Output::Status("Preparing file identities…".into()),
    );
    let mut pair = resolve(&launch, cancel);
    let mut viewport = Viewport {
        geometry,
        revision: 0,
    };
    loop {
        check_cancel(cancel)?;
        pair = match pair {
            Ok(paths) => {
                if let Err(message) = file_choice::validate(&paths) {
                    Err(FileChoice {
                        paths,
                        message,
                        is_error: true,
                        external_outcome: None,
                    })
                } else {
                    let target = context
                        .policy
                        .as_deref()
                        .map_or(Target::BuiltIn(BuiltInTarget::Automatic), |policy| {
                            routing::select(policy, &paths)
                        });
                    match target {
                        Target::External(program) => match external_compare::compare(
                            program,
                            &paths,
                            generation,
                            output,
                            context.launcher.as_ref(),
                            &context.credit,
                            context.cancellation.clone(),
                        ) {
                            Attempt::Choice(choice) => Err(choice),
                            Attempt::Started | Attempt::Closed => return Ok(()),
                            Attempt::Superseded => {
                                let Some(command) = wait_replacement(&context, &mut viewport)?
                                else {
                                    return Ok(());
                                };
                                replacement(command, &mut generation, &picker)
                            }
                        },
                        Target::BuiltIn(target) => {
                            let mut viewers_unavailable = None;
                            let result =
                                routing::plan(target, &paths, &launch.disabled_plugins, || {
                                    context
                                        .capture
                                        .image_providers(output, generation)
                                        .inspect_err(|reason| {
                                            viewers_unavailable = Some(reason.clone())
                                        })
                                })
                                .and_then(|plan| {
                                    compare(
                                        &paths,
                                        &appearance,
                                        &mut viewport,
                                        generation,
                                        &context,
                                        plan,
                                    )
                                    .map_err(|message| {
                                        match viewers_unavailable {
                                            Some(reason) => format!("{message}\n{reason}"),
                                            None => message,
                                        }
                                    })
                                });
                            match result {
                                Ok(None) => return Ok(()),
                                Ok(Some(command)) => replacement(command, &mut generation, &picker),
                                Err(message) => Err(FileChoice {
                                    paths,
                                    message,
                                    is_error: true,
                                    external_outcome: None,
                                }),
                            }
                        }
                    }
                }
            }
            Err(choice) => {
                output.put(generation, Output::ChooseFiles(choice));
                let Some(command) = wait_replacement(&context, &mut viewport)? else {
                    return Ok(());
                };
                replacement(command, &mut generation, &picker)
            }
        };
    }
}

fn wait_replacement(
    context: &Context<'_>,
    viewport: &mut Viewport,
) -> Result<Option<Command>, String> {
    loop {
        check_cancel(context.capture.cancel)?;
        // Close cannot be lost to a full command queue. Clear a font reply belonging
        // to the previous attempt before sleeping for the next choice.
        context.capture.inbox.check(None)?;
        let Ok(message) = context.receiver.recv() else {
            return Ok(None);
        };
        match message {
            Message::Input(Command {
                input: Input::Close,
                ..
            }) => return Ok(None),
            Message::Input(Command {
                input: Input::Resize(value, revision),
                ..
            }) => {
                viewport.geometry = value;
                viewport.revision = revision;
            }
            #[cfg(test)]
            Message::Input(Command {
                input: Input::ChooserBarrier(entered),
                ..
            }) => {
                let _ = entered.try_send(std::thread::current().id());
            }
            Message::Input(command)
                if context.output.is_current(command.generation)
                    && matches!(command.input, Input::Compare(_) | Input::Browse { .. }) =>
            {
                return Ok(Some(command));
            }
            _ => {}
        }
    }
}

fn replacement(
    command: Command,
    generation: &mut u64,
    picker: &Picker,
) -> Result<[std::path::PathBuf; 2], FileChoice> {
    *generation = command.generation;
    match command.input {
        Input::Compare(paths) => Ok(paths),
        Input::Browse { side, paths, owner } => Err(picker.browse(side, paths, owner)),
        _ => unreachable!("only comparison replacements leave the active comparison loop"),
    }
}

fn compare(
    paths: &[std::path::PathBuf; 2],
    appearance: &ViewerAppearance,
    viewport: &mut Viewport,
    generation: u64,
    context: &Context<'_>,
    plan: BuiltInPlan,
) -> Result<Option<Command>, String> {
    let receiver = context.receiver;
    let output = context.output;
    let capture = &context.capture;
    let cancel = capture.cancel;
    let left = capture.open(&paths[0]).map_err(|error| {
        format!(
            "File A could not be opened: {error}\n{}",
            paths[0].display()
        )
    })?;
    let right = capture.open(&paths[1]).map_err(|error| {
        format!(
            "File B could not be opened: {error}\n{}",
            paths[1].display()
        )
    })?;
    let mut sources = [left, right];
    let ssd = paths.each_ref().map(|path| capture.confirmed_ssd(path));
    check_cancel(cancel)?;
    let mut awaiting_fonts = None;
    let mut content = match plan {
        BuiltInPlan::Images {
            providers,
            selected,
        } => {
            output.put(
                generation,
                Output::Status("Decoding images in restricted workers…".into()),
            );
            Content::Images(Box::new(Images::open(
                [&providers[selected[0]], &providers[selected[1]]],
                &sources,
                ssd,
                cancel,
            )?))
        }
        BuiltInPlan::Text => {
            // The scan overlaps the reads and diff, and is shared by the service.
            let request = capture.inbox.font_request();
            let catalog = capture
                .fonts
                .request(cancel, capture.font_delivery(request));
            output.put(
                generation,
                Output::Status("Reading and comparing text…".into()),
            );
            let mut read = |side: usize| {
                let recapture = || capture.open(&paths[side]);
                revalidation::read_text(&mut sources[side], recapture, ssd[side], cancel)
            };
            let left = read(0)?;
            let right = read(1)?;
            let mut text = TextComparison::new(&left.text, &right.text, cancel)?;
            text.encodings = Some([left.encoding, right.encoding]);
            match catalog {
                Some(families) => text.set_font_catalog(families, &appearance.family),
                None => {
                    text.set_family(capture.fonts.resolve(&appearance.family, cancel));
                    awaiting_fonts = Some(request);
                }
            }
            Content::Text(Box::new(text))
        }
    };
    viewport.geometry = bounded(viewport.geometry);
    let mut dirty = true;
    let mut painted_appearance = 0;
    let mut first_frame = true;
    let mut identity = IdentityVerdict::Pending;
    let mut whole_input = None;
    let mut last_identity_turn = IdentityTurn::Blocked;
    loop {
        check_cancel(cancel)?;
        if let Some(reply) = capture.inbox.check(awaiting_fonts)? {
            awaiting_fonts = None;
            if let (Some(families), Content::Text(text)) = (reply, &mut content) {
                text.set_font_catalog(families, &appearance.family);
                dirty = true;
            }
        }
        #[cfg(test)]
        capture.turn();
        let find_pending = if let Content::Text(text) = &mut content {
            dirty |= text.find_turn(cancel)?;
            text.find_pending()
        } else {
            false
        };
        let requested = output.appearance();
        dirty |= requested.revision != painted_appearance;
        if dirty && output.is_current(generation) {
            revalidate(
                &mut sources,
                &content,
                &mut whole_input,
                paths,
                capture,
                ssd,
            )?;
            let mut frame = match &mut content {
                Content::Text(text) => {
                    text.paint(viewport.geometry, appearance, requested.palette, cancel)?
                }
                Content::Images(images) => images.paint(viewport.geometry, cancel)?,
            };
            frame.geometry_revision = viewport.revision;
            frame.comparison_generation = generation;
            frame.identity = identity;
            content.describe_identity(identity, &mut frame.status);
            check_cancel(cancel)?;
            revalidate(
                &mut sources,
                &content,
                &mut whole_input,
                paths,
                capture,
                ssd,
            )?;
            check_cancel(cancel)?;
            let published = output.put(
                generation,
                Output::Frame {
                    frame,
                    names: paths.each_ref().map(|path| path.display().to_string()),
                    appearance_revision: requested.revision,
                },
            );
            if published && first_frame {
                first_frame = false;
            }
            if published {
                painted_appearance = requested.revision;
            }
            dirty = !published;
        }
        let identity_ready = !first_frame
            && !dirty
            && identity == IdentityVerdict::Pending
            && output.is_current(generation)
            && !output.frame_waiting(generation);
        let poll = if find_pending {
            InputPoll::NonBlocking
        } else {
            InputPoll::after_turn(last_identity_turn, identity_ready)
        };
        let first = match poll.receive(receiver) {
            Ok(message) => Some(message),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let remaining = crate::INPUT_QUEUE - usize::from(first.is_some());
        for message in first.into_iter().chain(receiver.try_iter().take(remaining)) {
            let Message::Input(command) = message else {
                continue;
            };
            if matches!(command.input, Input::Close) {
                return Ok(None);
            }
            if output.is_current(command.generation)
                && matches!(command.input, Input::Compare(_) | Input::Browse { .. })
            {
                return Ok(Some(command));
            }
            if command.generation != generation {
                continue;
            }
            dirty |= apply(
                command.input,
                &mut content,
                &mut viewport.geometry,
                &mut viewport.revision,
            );
        }
        // Initial image presentation and its mailbox consumption precede any
        // whole-file identity reads. No extra worker, thread or queue is needed.
        last_identity_turn = IdentityTurn::Blocked;
        if !first_frame
            && !dirty
            && identity == IdentityVerdict::Pending
            && output.is_current(generation)
            && !output.frame_waiting(generation)
        {
            if whole_input.is_none() {
                whole_input = content.identity_turn(&sources, ssd[0] && ssd[1]);
            }
            last_identity_turn = if let Some(verifier) = whole_input.as_mut() {
                match verifier.step(&sources, cancel, || output.is_current(generation)) {
                    Ok(turn) => turn,
                    // Replacing a source whose change time alone moved, with unchanged
                    // content, restarts verification on the new pair. Other failures stand.
                    Err(error) => {
                        let replaced = output.is_current(generation)
                            && revalidate(
                                &mut sources,
                                &content,
                                &mut whole_input,
                                paths,
                                capture,
                                ssd,
                            )
                            .unwrap_or(false);
                        if !replaced {
                            return Err(error);
                        }
                        IdentityTurn::Advanced
                    }
                }
            } else {
                // An incomplete decoded-text turn always advanced its bounded
                // cursors; unlike unavailable I/O admission, it needs no wait.
                IdentityTurn::Advanced
            };
            if let IdentityTurn::Complete(verdict) = last_identity_turn {
                identity = verdict;
                dirty = true;
            }
        }
    }
    Ok(None)
}

/// Revalidate both sources (`revalidation::revalidate`). A replaced source discards
/// whole-input work keyed to the previous pair, which restarts from its first byte.
/// Returns whether a source was replaced.
fn revalidate(
    sources: &mut [CapturedFile; 2],
    content: &Content,
    whole_input: &mut Option<WholeInput>,
    paths: &[std::path::PathBuf; 2],
    capture: &Capture<'_>,
    ssd: [bool; 2],
) -> Result<bool, String> {
    let recapture = |side: usize| capture.open(&paths[side]);
    let replaced =
        revalidation::revalidate(sources, content.text(), recapture, ssd, capture.cancel)?;
    if replaced {
        *whole_input = None;
    }
    Ok(replaced)
}

fn apply(
    input: Input,
    content: &mut Content,
    geometry: &mut ViewerGeometry,
    geometry_revision: &mut u64,
) -> bool {
    if let Input::Resize(value, revision) = input {
        *geometry = bounded(value);
        *geometry_revision = revision;
        match content {
            Content::Images(images) => images.invalidate(),
            Content::Text(text) => text.reveal_visible_find_on_resize(),
        }
        return true;
    }
    match (content, input) {
        (Content::Text(text), Input::FindStep { delta, revision }) => {
            text.advance_find(delta, revision);
        }
        (
            Content::Text(text),
            Input::Find {
                query,
                backwards,
                revision,
            },
        ) => {
            text.begin_find(query, backwards, revision);
        }
        (Content::Text(text), Input::FontFamily(family)) => return text.change_font(&family),
        (Content::Text(text), Input::Scroll { x, y }) if x.is_finite() && y.is_finite() => {
            text.scroll_pixels(y);
            text.horizontal = (text.horizontal - x).clamp(0.0, 262144.0);
        }
        (Content::Text(text), Input::Lines(lines)) => text.scroll(i64::from(lines)),
        (Content::Text(text), Input::Pages(pages)) => text.scroll_pages(pages),
        (Content::Text(text), Input::Seek(value)) => text.seek(value),
        (Content::Text(text), Input::Difference { backwards }) => text.difference(backwards),
        (Content::Text(text), Input::DifferenceEdge { last }) => text.difference_edge(last),
        (Content::Images(images), Input::Mode(mode)) => images.set_mode(mode),
        (Content::Images(images), Input::Zoom(factor)) => images.zoom(factor),
        (Content::Images(images), Input::ActualPixels) => images.actual(),
        (Content::Images(images), Input::Fit) => images.fit(),
        (Content::Images(images), Input::Scroll { x, y }) => {
            images.pan(x * geometry.scale, y * geometry.scale)
        }
        // A key line pans 24 logical pixels; a page pans one viewport height.
        (Content::Images(images), Input::Lines(lines)) => {
            images.pan(0.0, -(lines as f32) * 24.0 * geometry.scale)
        }
        (Content::Images(images), Input::Pages(pages)) => {
            images.pan(0.0, -(pages as f32) * geometry.height as f32)
        }
        _ => return false,
    }
    true
}

fn bounded(mut geometry: ViewerGeometry) -> ViewerGeometry {
    geometry.width = geometry.width.clamp(2, 4096);
    geometry.height = geometry.height.clamp(1, 4096);
    geometry.scale = if geometry.scale.is_finite() {
        geometry.scale.clamp(0.5, 4.0)
    } else {
        1.0
    };
    let pixels = f64::from(geometry.width) * f64::from(geometry.height);
    if pixels > 4_194_304.0 {
        let reduction = (4_194_304.0 / pixels).sqrt();
        geometry.width = (f64::from(geometry.width) * reduction).max(2.0) as u32;
        geometry.height = (f64::from(geometry.height) * reduction).max(1.0) as u32;
        geometry.scale *= reduction as f32;
    }
    geometry
}
