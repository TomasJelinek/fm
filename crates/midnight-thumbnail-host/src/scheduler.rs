//! Bounded visible scheduling. Hover gets priority while selected videos retain fair progress.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

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
    failed_until: HashMap<(u64, u32), Instant>,
    active: Vec<Work>,
    cursor: usize,
    hover_turn: bool,
}

impl Scheduler {
    /// Supply exactly the visible items; never an unbounded directory.
    pub fn set_view(&mut self, generation: u64, items: Vec<Item>) -> bool {
        if items.len() > 256
            || items
                .iter()
                .any(|item| item.edge == 0 || item.edge > midnight_thumbnail_api::MAX_EDGE)
        {
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
            .retain(|(token, _), _| items.iter().any(|item| item.token == *token));
        self.items = items;
        // Stale active jobs still consume capacity until the process really exits.
        true
    }

    fn pending(&self, item: Item) -> Option<Work> {
        if self.failed_until.contains_key(&(item.token, item.edge))
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
        (0..count)
            .find(|frame| edges[usize::from(*frame)] < item.edge)
            .map(|frame| Work {
                generation: self.generation,
                token: item.token,
                edge: item.edge,
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
        self.complete_with_coverage(work, success.then_some(work.edge))
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
        let Some(index) = self.active.iter().position(|active| *active == work) else {
            return false;
        };
        self.active.swap_remove(index);
        if work.generation != self.generation
            || !self.items.iter().any(|item| item.token == work.token)
        {
            return false;
        }
        if let Some(edge) = coverage {
            let completed =
                &mut self.completed.entry(work.token).or_default()[usize::from(work.frame)];
            *completed = (*completed).max(edge);
            self.items
                .iter()
                .any(|item| item.token == work.token && item.edge <= edge)
        } else {
            self.failed_until.insert(
                (work.token, work.edge),
                Instant::now() + Duration::from_secs(30),
            );
            true
        }
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
                edge: 96,
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
