use crate::FsError;

/// Optional native network-drive dialogs, dispatched on a backend worker.
/// Credentials and connection changes stay in the operating system dialog.
pub trait NetworkService: Send + Sync {
    /// Opens the system connect or disconnect dialog. Some native disconnect
    /// dialogs are asynchronous, so success does not assert that mappings changed.
    fn manage_connections(&self, disconnect: bool) -> Result<(), FsError>;
}
