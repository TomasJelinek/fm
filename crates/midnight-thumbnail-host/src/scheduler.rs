//! Bounded visible scheduling. Hover gets priority while selected videos retain fair progress.
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

/// Delays before the retries of an item whose work failed in a way that may pass (a file
/// changed after its capture, a busy file, a decoder that timed out or crashed, a cache
/// write that failed). After the last one, and at once after a failure that will not
/// pass (content no decoder accepts, a thumbnailer that cannot start), the item is not
/// tried again while it stays in view: its tile keeps the file's icon. A new view
/// generation, or the item leaving the view, forgets this.
#[cfg(not(test))]
pub const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(8)];
/// The unit tests wait for real retries: the same backoff at a tenth of the time.
#[cfg(test)]
pub const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(200), Duration::from_millis(800)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub token: u64,
    pub edge: u32,
    pub video: bool,
    pub selected: bool,
    pub hovered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Work {
    pub generation: u64,
    pub token: u64,
    pub edge: u32,
    pub frame: u8,
}

#[derive(Default)]
pub struct Scheduler {
    generation: u64,
    items: Vec<Item>,
    completed: HashMap<u64, [u32; midnight_thumbnail_api::MAX_ANIMATION_FRAMES as usize]>,
    /// Items waiting for a retry after a failure, until their deadline.
    failed_until: HashMap<(u64, u32), Instant>,
    /// Retries already scheduled per item (`RETRY_DELAYS`).
    retries: HashMap<(u64, u32), usize>,
    /// Items not tried again while they stay in view.
    given_up: HashSet<(u64, u32)>,
    active: Vec<Work>,
    cursor: usize,
    hover_turn: bool,
}

impl Scheduler {
    /// Supply exactly the visible items; never an unbounded directory.
    pub fn set_view(&mut self, generation: u64, items: Vec<Item>) -> bool {
        if items.len() > midnight_core::MAX_THUMBNAIL_VIEW_TARGETS
            || items
                .iter()
                .any(|item| item.edge == 0 || item.edge > midnight_thumbnail_api::MAX_EDGE)
        {
            return false;
        }
        if generation != self.generation {
            self.completed.clear();
            self.failed_until.clear();
            self.retries.clear();
            self.given_up.clear();
            self.cursor = 0;
            self.hover_turn = true;
        }
        self.generation = generation;
        let visible = |token: u64| items.iter().any(|item| item.token == token);
        self.completed.retain(|token, _| visible(*token));
        self.failed_until.retain(|(token, _), _| visible(*token));
        self.retries.retain(|(token, _), _| visible(*token));
        self.given_up.retain(|(token, _)| visible(*token));
        self.items = items;
        // Stale active jobs still consume capacity until the process really exits.
        true
    }

    fn pending(&self, item: Item) -> Option<Work> {
        let key = (item.token, item.edge);
        if self.failed_until.contains_key(&key)
            || self.given_up.contains(&key)
            || self.active.iter().any(|work| work.token == item.token)
        {
            return None;
        }
        let edges = self.completed.get(&item.token).copied().unwrap_or_default();
        let count = if item.video && (item.selected || item.hovered) {
            midnight_thumbnail_api::MAX_ANIMATION_FRAMES
        } else {
            1
        };
        // A cycle's later frames match its first: when frame 0 came at a larger bucket (a
        // larger cached image, an earlier larger view), they are decoded at that bucket too,
        // so a cycle never mixes a sharp first frame with smaller, enlarged ones.
        (0..count).find_map(|frame| {
            let edge = if frame == 0 {
                item.edge
            } else {
                item.edge.max(edges[0])
            };
            (edges[usize::from(frame)] < edge).then_some(Work {
                generation: self.generation,
                token: item.token,
                edge,
                frame,
            })
        })
    }

    /// Failures are remembered per visible item and its requested edge, whatever bucket
    /// one of its cycle frames was decoded at.
    fn failure_key(&self, work: Work) -> (u64, u32) {
        let edge = self
            .items
            .iter()
            .find(|item| item.token == work.token)
            .map_or(work.edge, |item| item.edge);
        (work.token, edge)
    }

    pub fn next_work(&mut self) -> Option<Work> {
        self.next_work_at(Instant::now())
    }

    /// The earliest moment a visible item leaves its failure backoff. An idle backend
    /// needs no wakeup before then unless a request or completion arrives.
    pub(crate) fn next_retry_at(&self) -> Option<Instant> {
        self.failed_until.values().min().copied()
    }

    fn next_work_at(&mut self, now: Instant) -> Option<Work> {
        self.failed_until.retain(|_, deadline| *deadline > now);
        if self.active.len() >= 2 || self.items.is_empty() {
            return None;
        }
        let hover = self
            .items
            .iter()
            .filter(|item| item.hovered)
            .find_map(|item| self.pending(*item));
        let selected = (0..self.items.len()).find_map(|offset| {
            let index = (self.cursor + offset) % self.items.len();
            self.pending(self.items[index]).map(|work| (index, work))
        });
        let work = if self.hover_turn && hover.is_some() {
            self.hover_turn = false;
            hover?
        } else if let Some((index, work)) = selected {
            self.cursor = (index + 1) % self.items.len();
            self.hover_turn = true;
            work
        } else {
            hover?
        };
        self.active.push(work);
        Some(work)
    }

    /// Completion releases real worker capacity; stale/offscreen work never reaches presentation.
    /// A failure counts as one that may pass ([`Self::fail`]).
    pub fn complete(&mut self, work: Work, success: bool) -> bool {
        self.complete_with_coverage(work, success.then_some(work.edge))
    }

    /// A failed job releases its worker capacity. A failure that may pass (`transient`)
    /// is retried after the next of [`RETRY_DELAYS`]; one that will not pass, and one
    /// whose retries are spent, leaves the item untried while it stays in view.
    pub fn fail(&mut self, work: Work, transient: bool) -> bool {
        self.finish_at(work, Err(transient), Instant::now())
    }

    /// Retry an upload rejected by presentation after its available capacity changes.
    pub fn retry_delivery(&mut self, work: Work, retained_edge: u32) -> bool {
        if work.generation != self.generation
            || work.frame >= midnight_thumbnail_api::MAX_ANIMATION_FRAMES
            || work.edge == 0
            || work.edge > midnight_thumbnail_api::MAX_EDGE
            || retained_edge >= work.edge
            || !self.items.iter().any(|item| item.token == work.token)
        {
            return false;
        }
        let Some(edges) = self.completed.get_mut(&work.token) else {
            return false;
        };
        let completed = &mut edges[usize::from(work.frame)];
        if *completed != work.edge {
            return false;
        }
        *completed = retained_edge;
        true
    }

    pub(crate) fn complete_with_coverage(&mut self, work: Work, coverage: Option<u32>) -> bool {
        self.finish_at(work, coverage.ok_or(true), Instant::now())
    }

    /// `outcome` is the covered edge, or whether the failure may pass.
    fn finish_at(&mut self, work: Work, outcome: Result<u32, bool>, now: Instant) -> bool {
        let Some(index) = self.active.iter().position(|active| *active == work) else {
            return false;
        };
        self.active.swap_remove(index);
        if work.generation != self.generation
            || !self.items.iter().any(|item| item.token == work.token)
        {
            return false;
        }
        let edge = match outcome {
            Ok(edge) => edge,
            Err(transient) => {
                let key = self.failure_key(work);
                let retries = self.retries.entry(key).or_default();
                match RETRY_DELAYS.get(*retries).filter(|_| transient) {
                    Some(delay) => {
                        *retries += 1;
                        self.failed_until.insert(key, now + *delay);
                    }
                    None => {
                        self.given_up.insert(key);
                    }
                }
                return true;
            }
        };
        let completed = &mut self.completed.entry(work.token).or_default()[usize::from(work.frame)];
        *completed = (*completed).max(edge);
        self.items
            .iter()
            .any(|item| item.token == work.token && item.edge <= edge)
    }

    /// Cancellation caused by a changed viewport is not a decoder failure or a negative cache hit.
    pub fn abandon(&mut self, work: Work) {
        if let Some(index) = self.active.iter().position(|active| *active == work) {
            self.active.swap_remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(token: u64) -> Item {
        Item {
            token,
            edge: 96,
            video: true,
            selected: true,
            hovered: false,
        }
    }

    #[test]
    fn a_decoder_failure_retries_after_expiry_and_keeps_completed_frames() {
        let mut scheduler = Scheduler::default();
        scheduler.set_view(1, vec![video(1)]);
        let first = scheduler.next_work().unwrap();
        scheduler.complete(first, true);
        let failed = scheduler.next_work().unwrap();
        scheduler.complete(failed, false);
        assert!(scheduler.next_work().is_none());
        let retry_at = scheduler.next_retry_at().unwrap();
        assert!(retry_at <= Instant::now() + RETRY_DELAYS[0]);
        let retry = scheduler
            .next_work_at(Instant::now() + RETRY_DELAYS[0] + Duration::from_millis(10))
            .unwrap();
        assert_eq!(retry.frame, 1);
        assert!(
            scheduler.next_retry_at().is_none(),
            "expired backoff is retired"
        );
    }

    /// WIN-11: a transient failure is retried a bounded number of times, then the item
    /// rests with its icon while in view; a new generation or a return to view retries.
    #[test]
    fn transient_failures_retry_a_bounded_number_of_times() {
        let mut scheduler = Scheduler::default();
        scheduler.set_view(1, vec![video(1)]);
        let mut now = Instant::now();
        for delay in RETRY_DELAYS {
            let work = scheduler.next_work_at(now).expect("tried or retried");
            assert!(scheduler.finish_at(work, Err(true), now));
            assert!(scheduler.next_work_at(now).is_none(), "waits {delay:?}");
            now += delay;
        }
        let last = scheduler.next_work_at(now).expect("the last retry");
        assert!(scheduler.finish_at(last, Err(true), now));
        assert!(scheduler.next_retry_at().is_none());
        assert!(
            scheduler
                .next_work_at(now + Duration::from_secs(3600))
                .is_none(),
            "retries are spent while the item stays in view"
        );
        scheduler.set_view(1, Vec::new());
        scheduler.set_view(1, vec![video(1)]);
        assert!(
            scheduler.next_work_at(now).is_some(),
            "an item that comes back into view is tried again"
        );
    }

    /// A first frame served from a larger cached bucket takes the rest of its cycle along:
    /// a cycle never shows a 384 px first frame beside 192 px frames enlarged to its tile.
    #[test]
    fn cycle_frames_follow_a_larger_first_frame_and_back_off_per_item() {
        let mut scheduler = Scheduler::default();
        let item = Item {
            edge: 192,
            ..video(1)
        };
        scheduler.set_view(1, vec![item]);
        let first = scheduler.next_work().unwrap();
        assert_eq!((first.frame, first.edge), (0, 192));
        assert!(scheduler.complete_with_coverage(first, Some(384)));
        let second = scheduler.next_work().unwrap();
        assert_eq!((second.frame, second.edge), (1, 384));
        // A passing failure of that larger decode backs the item off as any failure does.
        let now = Instant::now();
        assert!(scheduler.finish_at(second, Err(true), now));
        assert!(scheduler.next_work_at(now).is_none(), "no immediate retry");
        let retry = scheduler.next_work_at(now + RETRY_DELAYS[0]).unwrap();
        assert_eq!((retry.frame, retry.edge), (1, 384));
        // Without a larger first frame the cycle keeps the item's own edge.
        let mut plain = Scheduler::default();
        plain.set_view(1, vec![item]);
        let first = plain.next_work().unwrap();
        assert!(plain.complete(first, true));
        assert_eq!(plain.next_work().unwrap().edge, 192);
    }

    #[test]
    fn a_permanent_failure_is_not_retried_while_in_view() {
        let mut scheduler = Scheduler::default();
        scheduler.set_view(1, vec![video(1), video(2)]);
        let first = scheduler.next_work().unwrap();
        assert!(scheduler.fail(first, false));
        assert!(scheduler.next_retry_at().is_none());
        let second = scheduler.next_work().unwrap();
        assert_ne!(second.token, first.token, "other items still run");
        assert!(scheduler.complete(second, true));
        assert!(
            scheduler
                .next_work_at(Instant::now() + Duration::from_secs(3600))
                .is_none_or(|work| work.token != first.token),
            "a permanent failure is never retried in this view"
        );
        scheduler.set_view(2, vec![video(1)]);
        assert_eq!(scheduler.next_work().unwrap().token, first.token);
    }
}
