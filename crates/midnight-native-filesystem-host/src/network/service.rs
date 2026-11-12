use super::*;
use midnight_core::{Cancellation, NetworkFailure, NetworkFailureKind, NetworkNeighborhood};

impl NetworkProvider {
    /// Preserve discovery failure categories when loading or calling a provider.
    pub fn classify_failure(failure: Failure) -> NetworkFailure {
        error(failure)
    }
    fn root_inner(&self, cancelled: Cancel<'_>) -> Result<NetworkAddress, Failure> {
        self.client.require(abi::CAP_VOLUMES)?;
        let Response::Volumes(mut roots) = self.client.exchange(Request::Volumes, cancelled)?
        else {
            return Err(invalid("Missing Network root"));
        };
        check_cancel(cancelled)?;
        if roots.len() != 1 {
            return Err(invalid("Network provider must expose one root"));
        }
        let root = roots.remove(0);
        self.decode(root.root, root.display)
    }
    fn parse_inner(&self, text: &str, cancelled: Cancel<'_>) -> Result<NetworkAddress, Failure> {
        self.client.require(abi::CAP_RESOLVE_TEXT)?;
        let (_lease, _, node) =
            self.resolved(Request::ResolveText { text: text.into() }, cancelled)?;
        self.decode(node.address, node.display)
    }
    fn parent_inner(
        &self,
        address: &NetworkAddress,
        cancelled: Cancel<'_>,
    ) -> Result<Option<NetworkAddress>, Failure> {
        let (_lease, mount, node) = self.resolve(address, cancelled)?;
        let Response::Parent(parent) = self.client.exchange(
            Request::Parent {
                mount,
                node: node.id,
            },
            cancelled,
        )?
        else {
            return Err(invalid("Missing Network parent"));
        };
        check_cancel(cancelled)?;
        parent
            .map(|node| {
                validate_node(&node)?;
                self.decode(node.address, node.display)
            })
            .transpose()
    }
    fn refresh_inner(
        &self,
        address: &NetworkAddress,
        cancelled: Cancel<'_>,
    ) -> Result<(), Failure> {
        self.client.require(abi::CAP_REFRESH)?;
        let (_lease, mount, node) = self.resolve(address, cancelled)?;
        self.client.exchange(
            Request::Refresh {
                mount,
                node: node.id,
            },
            cancelled,
        )?;
        check_cancel(cancelled)?;
        Ok(())
    }
}

impl NetworkNeighborhood for NetworkProvider {
    fn root(&self, cancel: &Cancellation) -> Result<NetworkAddress, NetworkFailure> {
        finish(self.root_inner(&|| cancel.is_cancelled()), cancel)
    }
    fn parse(&self, text: &str, cancel: &Cancellation) -> Result<NetworkAddress, NetworkFailure> {
        finish(self.parse_inner(text, &|| cancel.is_cancelled()), cancel)
    }
    fn parent(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
    ) -> Result<Option<NetworkAddress>, NetworkFailure> {
        finish(
            self.parent_inner(address, &|| cancel.is_cancelled()),
            cancel,
        )
    }
    fn list(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(NetworkNode) -> bool,
    ) -> Result<(), NetworkFailure> {
        finish(
            self.list_inner(address, &|| cancel.is_cancelled(), emit),
            cancel,
        )
    }
    fn activate(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
    ) -> Result<NetworkDestination, NetworkFailure> {
        finish(
            self.activate_inner(address, &|| cancel.is_cancelled()),
            cancel,
        )
    }
    fn refresh(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
    ) -> Result<(), NetworkFailure> {
        finish(
            self.refresh_inner(address, &|| cancel.is_cancelled()),
            cancel,
        )
    }
}

fn finish<T>(result: Result<T, Failure>, cancel: &Cancellation) -> Result<T, NetworkFailure> {
    check_cancel(&|| cancel.is_cancelled()).map_err(error)?;
    result.map_err(error)
}

fn error(failure: Failure) -> NetworkFailure {
    use midnight_filesystem_plugin_api::ErrorCode;
    let kind = match failure.code {
        ErrorCode::Invalid => NetworkFailureKind::Invalid,
        ErrorCode::Unsupported => NetworkFailureKind::Unsupported,
        ErrorCode::NotFound => NetworkFailureKind::NotFound,
        ErrorCode::PermissionDenied => NetworkFailureKind::PermissionDenied,
        ErrorCode::SourceChanged => NetworkFailureKind::SourceChanged,
        ErrorCode::Cancelled => NetworkFailureKind::Cancelled,
        ErrorCode::ResourceLimit => NetworkFailureKind::ResourceLimit,
        ErrorCode::Io => NetworkFailureKind::Io,
        ErrorCode::StaleReference => NetworkFailureKind::StaleReference,
        ErrorCode::Busy => NetworkFailureKind::Busy,
        ErrorCode::Unavailable => NetworkFailureKind::Unavailable,
        ErrorCode::Offline => NetworkFailureKind::Offline,
        ErrorCode::AuthenticationRequired => NetworkFailureKind::AuthenticationRequired,
        ErrorCode::MountRequired => NetworkFailureKind::MountRequired,
        ErrorCode::TimedOut => NetworkFailureKind::TimedOut,
    };
    NetworkFailure {
        kind,
        message: failure.message,
        os_code: failure.os_code,
    }
}
