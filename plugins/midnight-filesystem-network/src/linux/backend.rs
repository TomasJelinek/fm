use super::{Linux, uri};
use crate::{
    adapter::{Adapter, Context, NodeKind, Record, Target},
    api::{ActivationTarget, Address, AddressEncoding, ErrorCode, Failure},
};
pub(super) struct Entry {
    pub uri: Vec<u8>,
    pub display: String,
}
pub(super) trait Backend {
    type Cursor;
    fn begin(&self, uri: &[u8], context: &Context<'_>) -> Result<Self::Cursor, Failure>;
    fn next(
        &self,
        cursor: &mut Self::Cursor,
        context: &Context<'_>,
    ) -> Result<Option<Entry>, Failure>;
    fn activate(&self, target: &Target, context: &Context<'_>) -> Result<Option<Vec<u8>>, Failure>;
}
pub(super) fn begin<B: Backend>(
    backend: &B,
    target: &Target,
    context: &Context<'_>,
) -> Result<B::Cursor, Failure> {
    context.check()?;
    Linux::validate(target)?;
    if target.kind == NodeKind::Share {
        return Err(Failure::new(
            ErrorCode::MountRequired,
            "Activate this share to open its native location.",
        ));
    }
    backend.begin(&target.key, context)
}
pub(super) fn next<B: Backend>(
    backend: &B,
    cursor: &mut B::Cursor,
    context: &Context<'_>,
) -> Result<Option<Record>, Failure> {
    // Bound work even when an external provider sends a stream of unsupported rows.
    for _ in 0..128 {
        context.check()?;
        let Some(entry) = backend.next(cursor, context)? else {
            return Ok(None);
        };
        let kind = match uri::classify(&entry.uri) {
            Ok(kind) => kind,
            Err(error) if error.code == ErrorCode::Invalid => continue,
            Err(error) => return Err(error),
        };
        if entry.display.len() > uri::MAX_URI_BYTES {
            return Err(Failure::new(
                ErrorCode::ResourceLimit,
                "Network caption exceeds the admission budget.",
            ));
        }
        let display: String = entry.display.chars().filter(|c| !c.is_control()).collect();
        let display = if display.is_empty() {
            String::from_utf8_lossy(&entry.uri).into_owned()
        } else {
            display
        };
        context.check()?;
        return Ok(Some(Record {
            target: Target {
                kind,
                key: entry.uri,
            },
            display,
        }));
    }
    Err(Failure::new(
        ErrorCode::ResourceLimit,
        "Too many unsupported network discovery records.",
    ))
}
pub(super) fn activate<B: Backend>(
    backend: &B,
    target: &Target,
    context: &Context<'_>,
) -> Result<ActivationTarget, Failure> {
    context.check()?;
    Linux::validate(target)?;
    let native = backend.activate(target, context)?;
    context.check()?;
    if target.kind != NodeKind::Share {
        return Ok(ActivationTarget::WithinProvider(Address {
            encoding: AddressEncoding::Opaque,
            bytes: target.key.clone(),
        }));
    }
    let bytes = native.ok_or_else(|| {
        Failure::new(
            ErrorCode::Unsupported,
            "The mounted share has no native path. A working GVfs FUSE bridge is required.",
        )
    })?;
    if bytes.first() != Some(&b'/') || bytes.contains(&0) {
        return Err(Failure::new(
            ErrorCode::Invalid,
            "GIO returned an invalid native location.",
        ));
    }
    if bytes.len() > uri::MAX_URI_BYTES {
        return Err(Failure::new(
            ErrorCode::ResourceLimit,
            "Native location exceeds the admission budget.",
        ));
    }
    Ok(ActivationTarget::Native(Address {
        encoding: AddressEncoding::UnixBytes,
        bytes,
    }))
}
