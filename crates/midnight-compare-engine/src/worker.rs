use crate::discovery::{Deliver, FixedPitchFonts, ImageViewers};
use crate::file_choice::{self, Picker, resolve};
use crate::{
    Command, ExternalCompareLauncher, FileChoice, IdentityVerdict, Inbox, Input, Mailbox, Message,
    NativeCapture, Output, OwnedFileComparatorPolicy, StorageHint,
    binary::BinaryComparison,
    check_cancel,
    external_compare::{self, Attempt},
    identity::{IdentityTurn, WholeInput},
    images::Images,
    revalidation,
    routing::{self, BuiltInPlan, BuiltInTarget, Target},
    text::{ReadError, TextComparison, TextInput},
    typeface::Typeface,
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

/// A text pair and, once the Hex view was shown, the byte comparison of the same files.
struct TextPair {
    text: TextComparison,
    bytes: Option<BinaryComparison>,
    hex: bool,
}

enum Content {
    Text(Box<TextPair>),
    Images(Box<Images>),
    /// A pair compared only as bytes; its scan also decides the identity verdict.
    Bytes(Box<BinaryComparison>),
}

impl Content {
    fn identity_turn(
        &mut self,
        sources: &[CapturedFile; 2],
        confirmed_ssd: bool,
    ) -> Option<WholeInput> {
        match self {
            Self::Text(pair) => pair
                .text
                .identity_turn()
                .map(|equal| WholeInput::new(sources, Some(equal), confirmed_ssd)),
            Self::Images(_) => Some(WholeInput::new(sources, None, confirmed_ssd)),
            Self::Bytes(_) => None,
        }
    }

    /// The retained text; images retain decoded pixels, not their source bytes.
    fn text(&self) -> Option<&TextComparison> {
        match self {
            Self::Text(pair) => Some(&pair.text),
            Self::Images(_) | Self::Bytes(_) => None,
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
        if let Self::Text(pair) = self
            && let Some([left, right]) = pair.text.encodings
        {
            status.push_str(" · File A ");
            status.push_str(left.label());
            status.push_str(" · File B ");
            status.push_str(right.label());
        }
    }

    /// The byte comparison on screen, if any.
    fn shown_bytes(&mut self) -> Option<&mut BinaryComparison> {
        match self {
            Self::Bytes(bytes) => Some(bytes.as_mut()),
            Self::Text(pair) if pair.hex => pair.bytes.as_mut(),
            _ => None,
        }
    }

    /// The byte comparison whose scan still runs, shown or not.
    fn scanning(&mut self) -> Option<&mut BinaryComparison> {
        match self {
            Self::Bytes(bytes) => Some(bytes.as_mut()),
            Self::Text(pair) => pair.bytes.as_mut(),
            Self::Images(_) => None,
        }
        .filter(|bytes| !bytes.scan.complete())
    }

    fn typefaces(&mut self) -> Vec<&mut Typeface> {
        match self {
            Self::Text(pair) => {
                let TextPair { text, bytes, .. } = pair.as_mut();
                std::iter::once(&mut text.typeface)
                    .chain(bytes.as_mut().map(|bytes| &mut bytes.typeface))
                    .collect()
            }
            Self::Bytes(bytes) => vec![&mut bytes.typeface],
            Self::Images(_) => Vec::new(),
        }
    }
}

/// The work a worker turn does while no input waits: the scan of a shown byte comparison
/// first, then whole-input identity, then the scan of a hidden one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Background {
    Scan,
    Identity,
}

fn background(content: &mut Content, identity: IdentityVerdict) -> Option<Background> {
    let shown = content
        .shown_bytes()
        .is_some_and(|bytes| !bytes.scan.complete());
    let scanning = content.scanning().is_some();
    if shown || (scanning && matches!(content, Content::Bytes(_))) {
        Some(Background::Scan)
    } else if identity == IdentityVerdict::Pending && !matches!(content, Content::Bytes(_)) {
        Some(Background::Identity)
    } else if scanning {
        Some(Background::Scan)
    } else {
        None
    }
}

/// Read both captured files as text, publishing the progress of large ones. `Ok(Err)`
/// names the file and the reason text comparison refused (the pair is then compared as
/// bytes); `Err` is a failed read, which fails the comparison.
fn read_texts(
    sources: &mut [CapturedFile; 2],
    recapture: impl Fn(usize) -> Result<CapturedFile, String>,
    ssd: [bool; 2],
    generation: u64,
    output: &Mailbox,
    cancel: &AtomicBool,
) -> Result<Result<[TextInput; 2], String>, String> {
    let mut texts = Vec::with_capacity(2);
    for (side, source) in sources.iter_mut().enumerate() {
        let name = if side == 0 { "File A" } else { "File B" };
        let length = source.length();
        let mut progress = |read: u64| {
            output.put(
                generation,
                Output::Status(format!(
                    "Reading {name}: {} of {} MiB…",
                    read >> 20,
                    length.div_ceil(1 << 20)
                )),
            );
        };
        match revalidation::read_text(source, || recapture(side), ssd[side], cancel, &mut progress)
        {
            Ok(text) => texts.push(text),
            Err(ReadError::Refused(reason)) => return Ok(Err(format!("{name}: {reason}"))),
            Err(ReadError::Failed(error)) => return Err(error),
        }
    }
    let [left, right]: [TextInput; 2] = texts
        .try_into()
        .map_err(|_| "Text comparison needs two files.".to_owned())?;
    Ok(Ok([left, right]))
}

struct Viewport {
    geometry: ViewerGeometry,
    revision: u64,
    /// Every `Input::Scroll` taken, summed per axis (`ScrollView::scrolled`).
    scrolled: [f64; 2],
}

impl Viewport {
    /// Count a scroll input whether or not it applies, so the window knows which of the
    /// scrolling it sent a frame includes.
    fn take(&mut self, input: &Input) {
        if let Input::Scroll { x, y } = input {
            self.scrolled[0] += f64::from(*x);
            self.scrolled[1] += f64::from(*y);
        }
    }
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
        scrolled: [0.0; 2],
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
                                        viewers_unavailable.as_deref(),
                                    )
                                    .map_err(|message| {
                                        match &viewers_unavailable {
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
        if let Message::Input(command) = &message {
            viewport.take(&command.input);
        }
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
    viewers_unavailable: Option<&str>,
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
        plan @ (BuiltInPlan::Text | BuiltInPlan::Bytes(_)) => {
            // The scan overlaps the reads and diff, and is shared by the service.
            let request = capture.inbox.font_request();
            let catalog = capture
                .fonts
                .request(cancel, capture.font_delivery(request));
            let refused = match plan {
                BuiltInPlan::Bytes(reason) => Err(reason),
                _ => {
                    output.put(
                        generation,
                        Output::Status("Reading and comparing text…".into()),
                    );
                    // A source whose change time alone moved during its read is read
                    // once more from a fresh capture (`revalidation::read_text`).
                    let recapture = |side: usize| capture.open(&paths[side]);
                    read_texts(&mut sources, recapture, ssd, generation, output, cancel)?
                }
            };
            let mut content = match refused {
                Ok([left, right]) => {
                    if left.lines.len() + right.lines.len() > 100_000 {
                        output.put(generation, Output::Status("Matching lines…".into()));
                    }
                    let mut text = TextComparison::from_lines([left.lines, right.lines], cancel)?;
                    text.encodings = Some([left.encoding, right.encoding]);
                    Content::Text(Box::new(TextPair {
                        text,
                        bytes: None,
                        hex: false,
                    }))
                }
                Err(reason) => {
                    let reason = match viewers_unavailable {
                        Some(viewers) => format!("{reason}; {viewers}"),
                        None => reason,
                    };
                    Content::Bytes(Box::new(BinaryComparison::new(&sources, ssd, Some(reason))))
                }
            };
            let family = match &catalog {
                Some(_) => None,
                None => {
                    awaiting_fonts = Some(request);
                    Some(capture.fonts.resolve(&appearance.family, cancel))
                }
            };
            for typeface in content.typefaces() {
                match (&catalog, &family) {
                    (Some(families), _) => {
                        typeface.set_font_catalog(families.clone(), &appearance.family)
                    }
                    (None, Some(family)) => typeface.set_family(family.clone()),
                    (None, None) => {}
                }
            }
            content
        }
    };
    viewport.geometry = bounded(viewport.geometry);
    let mut dirty = true;
    let mut painted_appearance = 0;
    let mut first_frame = true;
    let mut identity = IdentityVerdict::Pending;
    let mut whole_input = None;
    let mut last_turn = IdentityTurn::Blocked;
    loop {
        check_cancel(cancel)?;
        if let Some(reply) = capture.inbox.check(awaiting_fonts)? {
            awaiting_fonts = None;
            if let Some(families) = reply {
                for typeface in content.typefaces() {
                    typeface.set_font_catalog(families.clone(), &appearance.family);
                    dirty = true;
                }
            }
        }
        #[cfg(test)]
        capture.turn();
        let find_pending = if let Content::Text(pair) = &mut content {
            dirty |= pair.text.find_turn(cancel)?;
            pair.text.find_pending()
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
            let hex_shown = matches!(&content, Content::Text(pair) if pair.hex);
            let mut frame = match &mut content {
                Content::Text(pair) => match pair.bytes.as_mut().filter(|_| hex_shown) {
                    Some(bytes) => bytes.paint(
                        viewport.geometry,
                        appearance,
                        requested.palette,
                        &sources,
                        cancel,
                    )?,
                    None => {
                        pair.text
                            .paint(viewport.geometry, appearance, requested.palette, cancel)?
                    }
                },
                Content::Images(images) => images.paint(viewport.geometry, cancel)?,
                Content::Bytes(bytes) => bytes.paint(
                    viewport.geometry,
                    appearance,
                    requested.palette,
                    &sources,
                    cancel,
                )?,
            };
            frame.geometry_revision = viewport.revision;
            frame.comparison_generation = generation;
            if let Some(view) = &mut frame.scroll {
                view.scrolled = viewport.scrolled;
            }
            frame.identity = identity;
            frame.bytes_only = matches!(content, Content::Bytes(_));
            frame.scanning = content.scanning().is_some();
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
        // Initial presentation and its mailbox consumption precede any whole-file reads
        // (identity or byte scan). No extra worker, thread or queue is needed.
        let ready = |content: &mut Content, dirty: bool| {
            (!first_frame
                && !dirty
                && output.is_current(generation)
                && !output.frame_waiting(generation))
            .then(|| background(content, identity))
            .flatten()
        };
        let poll = if find_pending {
            InputPoll::NonBlocking
        } else {
            InputPoll::after_turn(last_turn, ready(&mut content, dirty).is_some())
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
            viewport.take(&command.input);
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
            dirty |= apply(command.input, &mut content, viewport, &sources, ssd);
        }
        last_turn = IdentityTurn::Blocked;
        match ready(&mut content, dirty) {
            Some(Background::Scan) => {
                let primary = matches!(content, Content::Bytes(_));
                let shown = content.shown_bytes().is_some();
                let Some(bytes) = content.scanning() else {
                    continue;
                };
                last_turn = bytes
                    .scan
                    .step(&sources, cancel, || output.is_current(generation))?;
                let moved = bytes.resolve_pending();
                let progress = bytes.scan.progress_due();
                match last_turn {
                    IdentityTurn::Complete(verdict) => {
                        if primary {
                            identity = verdict;
                        }
                        dirty |= shown || primary;
                        // Whole-input identity may still be pending for a text pair.
                        last_turn = IdentityTurn::Advanced;
                    }
                    IdentityTurn::Advanced => dirty |= shown && (moved || progress),
                    IdentityTurn::Blocked => {}
                }
            }
            Some(Background::Identity) => {
                if whole_input.is_none() {
                    whole_input = content.identity_turn(&sources, ssd[0] && ssd[1]);
                }
                last_turn = if let Some(verifier) = whole_input.as_mut() {
                    match verifier.step(&sources, cancel, || output.is_current(generation)) {
                        Ok(turn) => turn,
                        // Replacing a source whose change time alone moved, with unchanged
                        // content, restarts verification on the new pair. Other failures
                        // stand.
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
                if let IdentityTurn::Complete(verdict) = last_turn {
                    identity = verdict;
                    dirty = true;
                    // A hidden byte scan may still have work.
                    last_turn = IdentityTurn::Advanced;
                }
            }
            None => {}
        }
    }
    Ok(None)
}

/// Rows of text or bytes: the navigation both views share.
trait Rows {
    fn scroll_pixels(&mut self, x: f32, y: f32);
    fn scroll_rows(&mut self, rows: i64);
    fn scroll_pages(&mut self, pages: i32);
    fn seek(&mut self, amount: f64);
    fn difference(&mut self, backwards: bool);
    fn difference_edge(&mut self, last: bool);
}

impl Rows for TextComparison {
    fn scroll_pixels(&mut self, x: f32, y: f32) {
        TextComparison::scroll_pixels(self, y);
        self.scroll_horizontal(x);
    }
    fn scroll_rows(&mut self, rows: i64) {
        self.scroll(rows);
    }
    fn scroll_pages(&mut self, pages: i32) {
        TextComparison::scroll_pages(self, pages);
    }
    fn seek(&mut self, amount: f64) {
        TextComparison::seek(self, amount);
    }
    fn difference(&mut self, backwards: bool) {
        TextComparison::difference(self, backwards);
    }
    fn difference_edge(&mut self, last: bool) {
        TextComparison::difference_edge(self, last);
    }
}

impl Rows for BinaryComparison {
    fn scroll_pixels(&mut self, x: f32, y: f32) {
        BinaryComparison::scroll_pixels(self, y);
        self.scroll_horizontal(x);
    }
    fn scroll_rows(&mut self, rows: i64) {
        self.scroll(rows);
    }
    fn scroll_pages(&mut self, pages: i32) {
        BinaryComparison::scroll_pages(self, pages);
    }
    fn seek(&mut self, amount: f64) {
        BinaryComparison::seek(self, amount);
    }
    fn difference(&mut self, backwards: bool) {
        BinaryComparison::difference(self, backwards);
    }
    fn difference_edge(&mut self, last: bool) {
        BinaryComparison::difference_edge(self, last);
    }
}

/// Navigation of a text or byte view; false for every other input.
fn navigate(rows: &mut dyn Rows, input: Input) -> bool {
    match input {
        Input::Scroll { x, y } if x.is_finite() && y.is_finite() => rows.scroll_pixels(x, y),
        Input::Lines(lines) => rows.scroll_rows(i64::from(lines)),
        Input::Pages(pages) => rows.scroll_pages(pages),
        Input::Seek(value) => rows.seek(value),
        Input::Difference { backwards } => rows.difference(backwards),
        Input::DifferenceEdge { last } => rows.difference_edge(last),
        _ => return false,
    }
    true
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
    viewport: &mut Viewport,
    sources: &[CapturedFile; 2],
    ssd: [bool; 2],
) -> bool {
    let geometry = viewport.geometry;
    if let Input::Resize(value, revision) = input {
        viewport.geometry = bounded(value);
        viewport.revision = revision;
        match content {
            Content::Images(images) => images.invalidate(),
            Content::Text(pair) => pair.text.reveal_visible_find_on_resize(),
            Content::Bytes(_) => {}
        }
        return true;
    }
    if let Input::FontFamily(family) = &input {
        // Text and its Hex view paint in the same family.
        let mut changed = false;
        for typeface in content.typefaces() {
            changed |= typeface.change_font(family);
        }
        return changed;
    }
    match (content, input) {
        (Content::Text(pair), Input::Bytes(show)) => {
            if show && pair.bytes.is_none() {
                let mut bytes = BinaryComparison::new(sources, ssd, None);
                bytes.typeface = pair.text.typeface.clone();
                pair.bytes = Some(bytes);
            }
            return std::mem::replace(&mut pair.hex, show) != show;
        }
        (Content::Text(pair), Input::FindStep { delta, revision }) => {
            pair.text.advance_find(delta, revision);
        }
        (
            Content::Text(pair),
            Input::Find {
                query,
                backwards,
                revision,
            },
        ) => {
            pair.text.begin_find(query, backwards, revision);
        }
        (Content::Text(pair), input) => {
            let hex = pair.hex;
            return match pair.bytes.as_mut().filter(|_| hex) {
                Some(bytes) => navigate(bytes, input),
                None => navigate(&mut pair.text, input),
            };
        }
        (Content::Bytes(bytes), input) => return navigate(bytes.as_mut(), input),
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
