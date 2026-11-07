//! Bounded visible scheduling. Hover gets priority while selected videos retain fair progress.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub token: u64,
    pub video: bool,
    pub selected: bool,
    pub hovered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Work {
    pub generation: u64,
    pub token: u64,
    pub frame: u8,
}

#[derive(Default)]
pub struct Scheduler {
    generation: u64,
    items: Vec<Item>,
    completed: HashMap<u64, u8>,
    failed_until: HashMap<u64, Instant>,
    active: Vec<Work>,
    cursor: usize,
    hover_turn: bool,
}

impl Scheduler {
    /// Supply exactly the visible items; never an unbounded directory.
    pub fn set_view(&mut self, generation: u64, items: Vec<Item>) -> bool {
        if items.len() > 256 {
            return false;
        }
        if generation != self.generation {
            self.completed.clear();
            self.failed_until.clear();
            self.cursor = 0;
            self.hover_turn = true;
        }
        self.generation = generation;
        self.completed
            .retain(|token, _| items.iter().any(|item| item.token == *token));
        self.failed_until
            .retain(|token, _| items.iter().any(|item| item.token == *token));
        self.items = items;
        // Stale active jobs still consume capacity until the process really exits.
        true
    }

    fn pending(&self, item: Item) -> Option<Work> {
        if self.failed_until.contains_key(&item.token)
            || self.active.iter().any(|work| work.token == item.token)
        {
            return None;
        }
        let mask = self.completed.get(&item.token).copied().unwrap_or(0);
        let count = if item.video && (item.selected || item.hovered) {
            midnight_thumbnail_api::MAX_ANIMATION_FRAMES
        } else {
            1
        };
        (0..count)
            .find(|frame| mask & (1 << frame) == 0)
            .map(|frame| Work {
                generation: self.generation,
                token: item.token,
                frame,
            })
    }

    pub fn next_work(&mut self) -> Option<Work> {
        self.next_work_at(Instant::now())
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
    pub fn complete(&mut self, work: Work, success: bool) -> bool {
        let Some(index) = self.active.iter().position(|active| *active == work) else {
            return false;
        };
        self.active.swap_remove(index);
        if work.generation != self.generation
            || !self.items.iter().any(|item| item.token == work.token)
        {
            return false;
        }
        if success {
            *self.completed.entry(work.token).or_default() |= 1 << work.frame;
        } else {
            self.failed_until
                .insert(work.token, Instant::now() + Duration::from_secs(30));
        }
        true
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
    #[test]
    fn a_decoder_failure_retries_after_expiry_and_keeps_completed_frames() {
        let mut scheduler = Scheduler::default();
        scheduler.set_view(
            1,
            vec![Item {
                token: 1,
                video: true,
                selected: true,
                hovered: false,
            }],
        );
        let first = scheduler.next_work().unwrap();
        scheduler.complete(first, true);
        let failed = scheduler.next_work().unwrap();
        scheduler.complete(failed, false);
        assert!(scheduler.next_work().is_none());
        let retry = scheduler
            .next_work_at(Instant::now() + Duration::from_secs(31))
            .unwrap();
        assert_eq!(retry.frame, 1);
    }
}
