//! Constructor rejection still owns its stage and must surface cleanup failures.
use super::*;

struct OwnedStage {
    data: Cursor<Vec<u8>>,
    cleaned: Arc<Mutex<bool>>,
    refuse: bool,
}
impl Read for OwnedStage {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        self.data.read(data)
    }
}
impl Write for OwnedStage {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.data.write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for OwnedStage {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.data.seek(pos)
    }
}
impl StagedTransferFile for OwnedStage {
    fn sync(&mut self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn len(&self) -> Result<u64, FsError> {
        Ok(self.data.get_ref().len() as u64)
    }
    fn publish_no_replace(
        &mut self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        panic!("rejected constructor cannot publish")
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        if self.refuse {
            return Err(FsError::from_io(
                "discard owned stage",
                Path::new("destination/.me-owned"),
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "owned stage removal refused",
                ),
            ));
        }
        *self.cleaned.lock().unwrap() = true;
        self.data.get_mut().clear();
        Ok(())
    }
}

#[test]
fn rejected_constructor_cleans_owned_stage_or_reports_its_exact_failure() {
    for mode in [
        TransferVerification::FullSha256,
        TransferVerification::SizeAndSamples,
    ] {
        for invalid_kind in [true, false] {
            for refuse in [false, true] {
                let (copy, _) = copy(vec![42; 17], false);
                let mut entry = copy.source_entry().clone();
                if invalid_kind {
                    entry.item.kind = EntryKind::Directory;
                } else {
                    entry.item.metadata.size = None;
                }
                let source = Box::new(Source {
                    entry,
                    data: Cursor::new(vec![42; 17]),
                    changed: Arc::default(),
                });
                let cleaned = Arc::new(Mutex::new(false));
                let result = VerifiedFileCopy::new_with_verification(
                    source,
                    Box::new(OwnedStage {
                        data: Cursor::new(b"partial".to_vec()),
                        cleaned: cleaned.clone(),
                        refuse,
                    }),
                    mode,
                );
                let Err(error) = result else {
                    panic!("invalid source accepted")
                };
                if refuse {
                    assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
                    assert_eq!(error.path, Path::new("destination/.me-owned"));
                    assert!(error.message.contains("regular source file"));
                    assert!(!*cleaned.lock().unwrap());
                } else {
                    assert!(
                        *cleaned.lock().unwrap(),
                        "constructor only dropped its stage without explicit cleanup"
                    );
                }
            }
        }
    }
}
