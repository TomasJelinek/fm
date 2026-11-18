use crate::file_choice::{self, Picker, resolve};
use crate::{
    Command, ExternalCompareLauncher, FileChoice, IdentityVerdict, Input, Mailbox, NativeCapture,
    Output, OwnedFileComparatorPolicy, check_cancel,
    external_compare::{self, Attempt},
    identity::{IdentityTurn, WholeInput},
    images::Images,
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
    fn identity_turn(&mut self, sources: &[CapturedFile; 2]) -> Option<WholeInput> {
        match self {
            Self::Text(text) => text
                .identity_turn()
                .map(|equal| WholeInput::new(sources, Some(equal))),
            Self::Images(_) => Some(WholeInput::new(sources, None)),
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
}

impl InputPoll {
    fn after_turn(turn: IdentityTurn, identity_ready: bool) -> Self {
        if identity_ready && turn == IdentityTurn::Advanced {
            Self::NonBlocking
        } else {
            Self::Wait(Duration::from_millis(10))
        }
    }

    fn receive<T>(self, receiver: &Receiver<T>) -> Result<T, RecvTimeoutError> {
        match self {
            Self::NonBlocking => receiver.try_recv().map_err(|error| match error {
                TryRecvError::Empty => RecvTimeoutError::Timeout,
                TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
            }),
            Self::Wait(delay) => receiver.recv_timeout(delay),
        }
    }
}

pub(crate) struct Capture<'a> {
    pub factory: &'a NativeCapture,
    pub cancel: &'a AtomicBool,
}

pub(crate) struct Context<'a> {
    pub receiver: &'a Receiver<Command>,
    pub output: &'a Mailbox,
    pub capture: Capture<'a>,
    pub policy: Option<Box<dyn OwnedFileComparatorPolicy>>,
    pub launcher: Option<Arc<dyn ExternalCompareLauncher>>,
    pub credit: Arc<dyn ExternalCompareSessionCredit>,
    pub cancellation: Cancellation,
    pub discover_images: crate::ImageDiscovery,
}

impl Capture<'_> {
    fn open(&self, path: &std::path::Path) -> Result<CapturedFile, String> {
        check_cancel(self.cancel)?;
        let result = (self.factory)(path, self.cancel);
        check_cancel(self.cancel)?;
        result
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
                            let result = routing::plan(
                                target,
                                &paths,
                                &launch.disabled_plugins,
                                context.discover_images,
                            )
                            .and_then(|plan| {
                                compare(
                                    &paths,
                                    &appearance,
                                    &mut viewport,
                                    generation,
                                    &context,
                                    plan,
                                )
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
        match context.receiver.recv_timeout(Duration::from_millis(30)) {
            Ok(Command {
                input: Input::Close,
                ..
            })
            | Err(RecvTimeoutError::Disconnected) => return Ok(None),
            Ok(Command {
                input: Input::Resize(value, revision),
                ..
            }) => {
                viewport.geometry = value;
                viewport.revision = revision;
            }
            #[cfg(test)]
            Ok(Command {
                input: Input::ChooserBarrier(entered),
                ..
            }) => {
                let _ = entered.try_send(std::thread::current().id());
            }
            Ok(command)
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
    #[cfg(test)]
    let first_frame_started = std::time::Instant::now();
    #[cfg(test)]
    crate::trace_first_frame("capture-start", first_frame_started);
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
    let sources = [left, right];
    #[cfg(test)]
    crate::trace_first_frame("capture-complete", first_frame_started);
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
                cancel,
            )?))
        }
        BuiltInPlan::Text => {
            output.put(
                generation,
                Output::Status("Reading and comparing text…".into()),
            );
            #[cfg(test)]
            crate::trace_first_frame("text-read-start", first_frame_started);
            let left = crate::text::read(&sources[0], cancel)?;
            #[cfg(test)]
            crate::trace_first_frame("left-text-read-complete", first_frame_started);
            let right = crate::text::read(&sources[1], cancel)?;
            #[cfg(test)]
            crate::trace_first_frame("text-reads-complete", first_frame_started);
            let mut text = TextComparison::new(&left.text, &right.text, cancel)?;
            text.encodings = Some([left.encoding, right.encoding]);
            #[cfg(test)]
            crate::trace_first_frame("text-prepare-complete", first_frame_started);
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
        let requested = output.appearance();
        dirty |= requested.revision != painted_appearance;
        if dirty && output.is_current(generation) {
            #[cfg(test)]
            if first_frame {
                crate::trace_first_frame("first-frame-revalidate", first_frame_started);
            }
            for source in &sources {
                source
                    .revalidate()
                    .map_err(|error| format!("Compared file changed: {error}"))?;
            }
            let mut frame = match &mut content {
                Content::Text(text) => {
                    text.paint(viewport.geometry, appearance, requested.palette, cancel)?
                }
                Content::Images(images) => images.paint(viewport.geometry, cancel)?,
            };
            #[cfg(test)]
            if first_frame {
                crate::trace_first_frame("first-frame-painted", first_frame_started);
            }
            frame.geometry_revision = viewport.revision;
            frame.comparison_generation = generation;
            frame.identity = identity;
            content.describe_identity(identity, &mut frame.status);
            check_cancel(cancel)?;
            for source in &sources {
                source
                    .revalidate()
                    .map_err(|error| format!("Compared file changed: {error}"))?;
            }
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
                #[cfg(test)]
                crate::trace_first_frame("first-frame-published", first_frame_started);
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
        let input_poll = InputPoll::after_turn(last_identity_turn, identity_ready);
        match input_poll.receive(receiver) {
            Ok(Command {
                input: Input::Close,
                ..
            })
            | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
            Ok(command)
                if output.is_current(command.generation)
                    && matches!(command.input, Input::Compare(_) | Input::Browse { .. }) =>
            {
                return Ok(Some(command));
            }
            Ok(Command {
                input,
                generation: command_generation,
            }) if command_generation == generation => {
                dirty |= apply(
                    input,
                    &mut content,
                    &mut viewport.geometry,
                    &mut viewport.revision,
                )
            }
            _ => {}
        }
        for command in receiver.try_iter().take(31) {
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
                whole_input = content.identity_turn(&sources);
            }
            last_identity_turn = if let Some(whole_input) = whole_input.as_mut() {
                whole_input.step(&sources, cancel, || output.is_current(generation))?
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

fn apply(
    input: Input,
    content: &mut Content,
    geometry: &mut ViewerGeometry,
    geometry_revision: &mut u64,
) -> bool {
    if let Input::Resize(value, revision) = input {
        *geometry = bounded(value);
        *geometry_revision = revision;
        if let Content::Images(images) = content {
            images.invalidate();
        }
        return true;
    }
    match (content, input) {
        (Content::Text(text), Input::FontFamily(family)) => return text.change_font(&family),
        (Content::Text(text), Input::Scroll { x, y }) if x.is_finite() && y.is_finite() => {
            text.scroll_pixels(y);
            text.horizontal = (text.horizontal - x).clamp(0.0, 262144.0);
        }
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
