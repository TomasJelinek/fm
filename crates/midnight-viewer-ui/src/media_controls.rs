//! Value-only transport between the viewer chrome and an admitted media session.
use crate::{MediaControlsState, ViewerWindow};
use midnight_core::{
    MediaAction, MediaControl, MediaControlId, MediaPhase, MediaSessionId, MediaSnapshot,
};
use slint::{ComponentHandle, SharedString, Weak};
use std::{cell::RefCell, rc::Rc, time::Duration};

#[cfg(test)]
#[path = "media_controls_tests.rs"]
mod tests;

pub(super) fn is_active(window: &ViewerWindow) -> bool {
    !window.get_media_controls().session.is_empty()
}

enum Input {
    Toggle,
    Seek(f32),
}

#[derive(Clone, Copy)]
struct Pending {
    playing: bool,
    seek: Option<Duration>,
}

struct Active {
    session: MediaSessionId,
    accepted: MediaControlId,
    minimum_frame_control: MediaControlId,
    latest: Option<MediaSnapshot>,
    pending: Option<Pending>,
    refused: bool,
}

impl Active {
    fn playback_allowed(&self) -> bool {
        self.latest.as_ref().is_some_and(|snapshot| {
            !matches!(snapshot.phase, MediaPhase::Opening | MediaPhase::Failed(_))
        })
    }

    fn duration(&self) -> Option<Duration> {
        self.latest
            .as_ref()?
            .duration
            .filter(|duration| !duration.is_zero())
    }

    fn seek_duration(&self) -> Option<Duration> {
        let snapshot = self.latest.as_ref()?;
        if self.playback_allowed() && snapshot.seekable {
            self.duration()
        } else {
            None
        }
    }

    /// The video plays sound, so its volume and mute can be changed.
    fn audio(&self) -> bool {
        self.playback_allowed() && self.latest.as_ref().is_some_and(|snapshot| snapshot.audio)
    }

    fn playing(&self) -> bool {
        self.pending.map_or_else(
            || {
                self.latest
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.playing)
            },
            |pending| pending.playing,
        )
    }

    fn plan(&self, session: &str, input: Input) -> Option<MediaControl> {
        if session.parse::<u64>().ok()? != self.session.0 || !self.playback_allowed() {
            return None;
        }
        let action = match input {
            Input::Toggle => MediaAction::SetPlaying(!self.playing()),
            Input::Seek(fraction) => {
                MediaAction::Seek(seek_position(self.seek_duration()?, fraction)?)
            }
        };
        Some(MediaControl {
            session: self.session,
            id: MediaControlId(self.accepted.0.checked_add(1)?),
            action,
        })
    }

    fn submitted(&mut self, control: MediaControl, accepted: bool) {
        if control.session != self.session || self.accepted.0.checked_add(1) != Some(control.id.0) {
            return;
        }
        self.refused = !accepted;
        if accepted {
            // Ordinary play/pause leaves an already decoded preview valid. A
            // replay changes content only when Ended was observed with no
            // outstanding intent; pending seeks already own their own floor.
            let replay = matches!(control.action, MediaAction::SetPlaying(true))
                && self.pending.is_none()
                && self
                    .latest
                    .as_ref()
                    .is_some_and(|state| state.phase == MediaPhase::Ended);
            if matches!(control.action, MediaAction::Seek(_)) || replay {
                self.minimum_frame_control = control.id;
            }
            let mut pending = self.pending.unwrap_or(Pending {
                playing: self.playing(),
                seek: None,
            });
            match control.action {
                MediaAction::SetPlaying(playing) => pending.playing = playing,
                MediaAction::Seek(position) => pending.seek = Some(position),
            }
            self.pending = Some(pending);
            self.accepted = control.id;
        }
    }

    fn apply(&mut self, snapshot: MediaSnapshot) -> bool {
        if snapshot.session != self.session
            || snapshot.control != self.accepted
            || self
                .latest
                .as_ref()
                .is_some_and(|latest| snapshot.revision <= latest.revision)
        {
            return false;
        }
        self.pending = None;
        self.latest = Some(snapshot);
        true
    }

    fn presentation(&self) -> MediaControlsState {
        let phase = self.latest.as_ref().map(|snapshot| &snapshot.phase);
        let playing = self.playing();
        let status = match (self.pending, phase) {
            (Some(Pending { seek: Some(_), .. }), _) | (_, Some(MediaPhase::Seeking)) => "Seeking…",
            (Some(_), _) if playing => "Starting playback…",
            (Some(_), _) => "Pausing…",
            (_, None | Some(MediaPhase::Opening)) => "Opening video…",
            (_, Some(MediaPhase::Buffering)) => "Buffering…",
            (_, Some(MediaPhase::Ended)) => "Playback ended",
            _ => "",
        };
        let error = match phase {
            Some(MediaPhase::Failed(message)) if !message.trim().is_empty() => message.as_str(),
            Some(MediaPhase::Failed(_)) => "Video playback failed.",
            _ if self.refused => "Playback command could not be queued. Try again.",
            _ => "",
        };
        let mut position = self
            .pending
            .and_then(|pending| pending.seek)
            .unwrap_or_else(|| {
                self.latest
                    .as_ref()
                    .map_or(Duration::ZERO, |snapshot| snapshot.position)
            });
        let duration = self.duration();
        if let Some(duration) = duration {
            position = position.min(duration);
        }
        let time = if matches!(phase, None | Some(MediaPhase::Opening)) {
            String::new()
        } else if let Some(duration) = duration {
            format!("{} / {}", timestamp(position), timestamp(duration))
        } else {
            timestamp(position)
        };
        MediaControlsState {
            session: self.session.0.to_string().into(),
            audio: self.audio(),
            playing,
            can_play_pause: self.playback_allowed(),
            can_seek: self.seek_duration().is_some(),
            position: duration.map_or(0.0, |duration| {
                (position.as_secs_f64() / duration.as_secs_f64()) as f32
            }),
            time: time.into(),
            status: status.into(),
            error: midnight_core::visible_name(error).as_ref().into(),
            ..MediaControlsState::default()
        }
    }
}

fn seek_position(duration: Duration, fraction: f32) -> Option<Duration> {
    if !(0.0..=1.0).contains(&fraction) {
        return None;
    }
    if fraction == 1.0 {
        return Some(duration);
    }
    Duration::try_from_secs_f64(duration.as_secs_f64() * f64::from(fraction))
        .ok()
        .map(|position| position.min(duration))
}

fn timestamp(value: Duration) -> String {
    let seconds = value.as_secs();
    let hours = seconds / 3600;
    let minutes = seconds / 60;
    if hours != 0 {
        format!("{hours}:{:02}:{:02}", minutes % 60, seconds % 60)
    } else {
        format!("{minutes}:{:02}", seconds % 60)
    }
}

/// The window's sound level. It outlives each video, so the next file the window shows
/// plays at the same level; it is not saved with the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Volume {
    percent: u8,
    muted: bool,
}

impl Default for Volume {
    fn default() -> Self {
        Self {
            percent: 100,
            muted: false,
        }
    }
}

/// A change of the window's sound level from its controls.
enum Sound {
    ToggleMute,
    /// The slider's fraction of full volume. Moving the slider also unmutes.
    Level(f32),
}

impl Volume {
    fn changed(self, sound: Sound) -> Option<Self> {
        let next = match sound {
            Sound::ToggleMute => Self {
                muted: !self.muted,
                ..self
            },
            Sound::Level(fraction) => {
                if !(0.0..=1.0).contains(&fraction) {
                    return None;
                }
                Self {
                    percent: (fraction * 100.0).round() as u8,
                    muted: false,
                }
            }
        };
        (next != self).then_some(next)
    }
}

#[derive(Default)]
struct State {
    highest: Option<MediaSessionId>,
    active: Option<Active>,
    volume: Volume,
}

impl State {
    fn presentation(&self) -> MediaControlsState {
        let mut state = self
            .active
            .as_ref()
            .map_or_else(MediaControlsState::default, Active::presentation);
        state.volume = f32::from(self.volume.percent) / 100.0;
        state.muted = self.volume.muted;
        state
    }

    /// Change the level for the active video with sound. `apply` stores it in the
    /// window's audio output and says whether it took it.
    fn sound(
        &mut self,
        session: Option<&str>,
        sound: Sound,
        apply: &dyn Fn(u8, bool) -> bool,
    ) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        if session.is_some_and(|session| session.parse::<u64>().ok() != Some(active.session.0))
            || !active.audio()
        {
            return false;
        }
        let Some(next) = self.volume.changed(sound) else {
            return false;
        };
        if !apply(next.percent, next.muted) {
            return false;
        }
        self.volume = next;
        true
    }
}

/// Stores a sound level in the window's audio output: percent and muted.
pub type SetVolume = Rc<dyn Fn(u8, bool) -> bool>;

/// The viewer owns one binding for its lifetime. Worker delivery is asynchronous
/// and the sender reports admission without delivering a snapshot inline.
pub struct MediaControls {
    window: Weak<ViewerWindow>,
    state: Rc<RefCell<State>>,
    sender: Rc<dyn Fn(MediaControl) -> bool>,
    set_volume: SetVolume,
}

impl MediaControls {
    pub fn install(
        window: &ViewerWindow,
        send: Rc<dyn Fn(MediaControl) -> bool>,
        set_volume: SetVolume,
    ) -> Self {
        let binding = Self {
            window: window.as_weak(),
            state: Rc::new(RefCell::new(State::default())),
            sender: send,
            set_volume,
        };
        let weak = binding.window.clone();
        let state = Rc::downgrade(&binding.state);
        let sender = Rc::downgrade(&binding.sender);
        let dispatch = Rc::new(move |session: SharedString, input| {
            let (Some(window), Some(state), Some(sender)) =
                (weak.upgrade(), state.upgrade(), sender.upgrade())
            else {
                return;
            };
            let Some(control) = state
                .borrow()
                .active
                .as_ref()
                .and_then(|active| active.plan(session.as_str(), input))
            else {
                return;
            };
            let accepted = sender(control);
            let presentation = {
                let mut state = state.borrow_mut();
                if let Some(active) = &mut state.active {
                    active.submitted(control, accepted);
                }
                state.presentation()
            };
            window.set_media_controls(presentation);
        });
        let toggle = dispatch.clone();
        window.on_media_toggle(move |session| toggle(session, Input::Toggle));
        window.on_media_seek(move |session, value| dispatch(session, Input::Seek(value)));
        let sound = {
            let weak = binding.window.clone();
            let state = Rc::downgrade(&binding.state);
            let set_volume = Rc::downgrade(&binding.set_volume);
            Rc::new(move |session: SharedString, sound| {
                if let (Some(window), Some(state), Some(set_volume)) =
                    (weak.upgrade(), state.upgrade(), set_volume.upgrade())
                    && state
                        .borrow_mut()
                        .sound(Some(session.as_str()), sound, &*set_volume)
                {
                    window.set_media_controls(state.borrow().presentation());
                }
            })
        };
        let mute = sound.clone();
        window.on_media_mute(move |session| mute(session, Sound::ToggleMute));
        window.on_media_volume(move |session, value| sound(session, Sound::Level(value)));
        binding.refresh();
        binding
    }

    /// Mute or unmute the active video from the keyboard (`viewer.mute`). False when no
    /// video with sound is shown.
    pub(super) fn toggle_mute(&self) -> bool {
        let changed = self
            .state
            .borrow_mut()
            .sound(None, Sound::ToggleMute, &*self.set_volume);
        if changed {
            self.refresh();
        }
        changed
    }

    /// Begin only from a trusted viewer open transition.
    pub fn begin(&self, session: MediaSessionId) -> bool {
        let mut state = self.state.borrow_mut();
        if session.0 == 0 || state.highest.is_some_and(|highest| session <= highest) {
            return false;
        }
        state.highest = Some(session);
        state.active = Some(Active {
            session,
            accepted: MediaControlId::default(),
            minimum_frame_control: MediaControlId::default(),
            latest: None,
            pending: None,
            refused: false,
        });
        drop(state);
        self.refresh();
        true
    }

    pub fn apply(&self, snapshot: MediaSnapshot) -> bool {
        let mut state = self.state.borrow_mut();
        let accepted = state
            .active
            .as_mut()
            .is_some_and(|active| active.apply(snapshot));
        drop(state);
        if accepted {
            self.refresh();
        }
        accepted
    }

    pub(super) fn accepts_frame(
        &self,
        session: MediaSessionId,
        control: Option<MediaControlId>,
    ) -> bool {
        self.state.borrow().active.as_ref().is_some_and(|active| {
            active.session == session
                && control.is_some_and(|control| {
                    active.minimum_frame_control.0 <= control.0 && control.0 <= active.accepted.0
                })
        })
    }

    /// Retire from the accepted file or session transition. Worker cleanup has
    /// its own lifetime and can still finish after the controls are hidden.
    pub fn retire(&self, session: MediaSessionId) -> bool {
        let mut state = self.state.borrow_mut();
        if state
            .active
            .as_ref()
            .is_none_or(|active| active.session != session)
        {
            return false;
        }
        state.active = None;
        drop(state);
        self.refresh();
        true
    }

    pub fn retire_active(&self) {
        let session = self
            .state
            .borrow()
            .active
            .as_ref()
            .map(|active| active.session);
        if let Some(session) = session {
            self.retire(session);
        }
    }

    fn refresh(&self) {
        if let Some(window) = self.window.upgrade() {
            let presentation = self.state.borrow().presentation();
            window.set_media_controls(presentation);
        }
    }
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        self.retire_active();
    }
}
