//! Read-only checkpoint validation precedes cancellation; retirement owns no paths.
use super::*;

impl Staging {
    pub(crate) fn cancellation_checkpoint(
        &self,
        operation: &Message,
        message: &Message,
    ) -> io::Result<bool> {
        let Message::Read { request, entry, .. } = operation else {
            return Ok(false);
        };
        if message.request() != *request {
            return Err(invalid());
        }
        let pending = |stage: u64| {
            self.pending
                .as_ref()
                .filter(|(id, value)| {
                    *id == stage
                        && value.request == *request
                        && value.purpose == StagePurpose::Member { entry: *entry }
                })
                .map(|(_, value)| value)
                .ok_or_else(invalid)
        };
        match message {
            Message::StageBegin {
                purpose,
                expected_length,
                max_length,
                ..
            } => {
                if *purpose != (StagePurpose::Member { entry: *entry })
                    || self.pending.is_some()
                    || self.sealed.len() >= 2
                    || self
                        .sealed
                        .values()
                        .any(|value| matches!(value.purpose, StagePurpose::Member { .. }))
                    || expected_length.is_some_and(|length| length > *max_length)
                    || self.next == u64::MAX
                {
                    return Err(invalid());
                }
            }
            Message::StageAppend {
                stage,
                offset,
                bytes,
                ..
            } => {
                let pending = pending(*stage)?;
                let end = offset.checked_add(bytes.len() as u64).ok_or_else(invalid)?;
                if bytes.is_empty()
                    || bytes.len() > MAX_READ as usize
                    || *offset != pending.spool.length()
                    || end > pending.maximum
                    || end > i64::MAX as u64
                    || pending.expected.is_some_and(|length| end > length)
                {
                    return Err(invalid());
                }
            }
            Message::StageProgress {
                stage, completed, ..
            } => {
                if *completed <= pending(*stage)?.completed {
                    return Err(invalid());
                }
            }
            Message::StageSeal { stage, length, .. } => {
                let pending = pending(*stage)?;
                if pending.spool.length() != *length
                    || pending.expected.is_some_and(|expected| expected != *length)
                {
                    return Err(invalid());
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    pub(crate) fn abandon_member(&mut self, request: u64) -> io::Result<()> {
        if self.pending.as_ref().is_some_and(|(_, pending)| {
            pending.request != request || !matches!(pending.purpose, StagePurpose::Member { .. })
        }) || self.sealed.values().any(|sealed| {
            matches!(sealed.purpose, StagePurpose::Member { .. }) && sealed.request != request
        }) {
            return Err(invalid());
        }
        // Spool owns delete-on-close/anonymous storage. Drop on this backend thread,
        // after any admitted native I/O, without touching verified scratch/cache leases.
        self.pending.take();
        self.sealed
            .retain(|_, sealed| !matches!(sealed.purpose, StagePurpose::Member { .. }));
        Ok(())
    }
}
