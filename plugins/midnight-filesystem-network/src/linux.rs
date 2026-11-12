use crate::{
    adapter::*,
    api::{self, ErrorCode, Failure},
};
mod backend;
#[allow(unsafe_code)]
mod gio;
mod uri;
pub struct Linux;
impl Adapter for Linux {
    type Cursor = gio::Cursor;
    fn root() -> Record {
        Record {
            target: Target {
                kind: NodeKind::Root,
                key: b"network:///".to_vec(),
            },
            display: "Network".into(),
        }
    }
    fn validate(target: &Target) -> Result<(), Failure> {
        if uri::classify(&target.key)? == target.kind {
            Ok(())
        } else {
            Err(Failure::new(
                ErrorCode::Invalid,
                "Network location kind does not match its address.",
            ))
        }
    }
    fn manual(input: &str) -> Result<Record, Failure> {
        let (key, kind) = uri::manual(input)?;
        Ok(Record {
            target: Target { kind, key },
            display: input.into(),
        })
    }
    fn begin(target: &Target, context: &Context<'_>) -> Result<Self::Cursor, Failure> {
        backend::begin(&gio::Gio, target, context)
    }
    fn next(cursor: &mut Self::Cursor, context: &Context<'_>) -> Result<Option<Record>, Failure> {
        backend::next(&gio::Gio, cursor, context)
    }
    fn activate(target: &Target, context: &Context<'_>) -> Result<api::ActivationTarget, Failure> {
        backend::activate(&gio::Gio, target, context)
    }
}
#[cfg(test)]
mod tests;
