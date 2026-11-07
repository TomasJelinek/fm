pub(crate) fn resolve() -> Result<std::path::PathBuf, crate::Error> {
    Ok(std::env::current_exe()?)
}
