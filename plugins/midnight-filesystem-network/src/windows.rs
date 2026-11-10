//! Unicode Windows network discovery; filesystem handoff requires explicit activation.
use crate::{
    adapter::{Adapter, Context, NodeKind, Record, Target},
    api::{ActivationTarget, Address, AddressEncoding, ErrorCode, Failure},
};
#[allow(unsafe_code)]
mod mpr;
mod path;
mod resource;
pub struct Windows;
impl Adapter for Windows {
    type Cursor = mpr::Cursor;
    fn root() -> Record {
        Record {
            target: Target {
                kind: NodeKind::Root,
                key: b"WNET-ROOT\x01".to_vec(),
            },
            display: "Network".into(),
        }
    }
    fn validate(target: &Target) -> Result<(), Failure> {
        if target.kind == NodeKind::Root {
            return if *target == Self::root().target {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        let resource = resource::Resource::decode(&target.key)?;
        if resource.classify()? == Some(target.kind) {
            Ok(())
        } else {
            Err(invalid())
        }
    }
    fn manual(input: &str) -> Result<Record, Failure> {
        use windows_sys::Win32::NetworkManagement::WNet::*;
        let (remote, kind) = path::manual(input)?;
        let resource = resource::Resource {
            scope: RESOURCE_GLOBALNET,
            kind: RESOURCETYPE_DISK,
            display_type: if kind == NodeKind::Server {
                resource::DISPLAY_SERVER
            } else {
                resource::DISPLAY_SHARE
            },
            usage: if kind == NodeKind::Server {
                RESOURCEUSAGE_CONTAINER
            } else {
                RESOURCEUSAGE_CONNECTABLE
            },
            remote: Some(remote),
            ..Default::default()
        };
        resource.record()?.ok_or_else(unsupported)
    }
    fn begin(target: &Target, context: &Context<'_>) -> Result<Self::Cursor, Failure> {
        mpr::begin(target, context)
    }
    fn next(cursor: &mut Self::Cursor, context: &Context<'_>) -> Result<Option<Record>, Failure> {
        cursor.next(context)
    }
    fn activate(target: &Target, context: &Context<'_>) -> Result<ActivationTarget, Failure> {
        context.check()?;
        Self::validate(target)?;
        let result = if target.kind == NodeKind::Share {
            let resource = resource::Resource::decode(&target.key)?;
            ActivationTarget::Native(Address {
                encoding: AddressEncoding::WindowsUtf16Le,
                bytes: path::native(resource.remote.as_deref().ok_or_else(invalid)?)?,
            })
        } else {
            ActivationTarget::WithinProvider(Address {
                encoding: AddressEncoding::Opaque,
                bytes: target.key.clone(),
            })
        };
        context.check()?;
        Ok(result)
    }
}
fn unsupported() -> Failure {
    Failure::new(ErrorCode::Unsupported, "Unsupported network target.")
}
fn invalid() -> Failure {
    Failure::new(ErrorCode::Invalid, "Invalid network resource identity.")
}
fn limit() -> Failure {
    Failure::new(
        ErrorCode::ResourceLimit,
        "Network resource budget exceeded.",
    )
}
#[cfg(test)]
mod tests;
