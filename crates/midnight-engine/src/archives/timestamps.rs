//! Format provider dates on the listing worker; local ZIP times have no UTC offset.
use midnight_core::{ArchiveTimestamp, Entry, LocaleService, LocalizedDateTime};

pub(super) fn localize(entry: &mut Entry, locale: Option<&dyn LocaleService>) {
    if let Some(time) = entry.virtual_node.as_ref().and_then(|node| node.modified)
        && time.is_valid()
        && let ArchiveTimestamp::Local {
            year,
            month,
            day,
            hour,
            minute,
            second,
        } = time
    {
        entry.metadata.localized_modified = locale
            .and_then(|locale| locale.format_archive_local_time(time))
            .or_else(|| {
                Some(LocalizedDateTime {
                    date: format!("{year:04}-{month:02}-{day:02}"),
                    time: format!("{hour:02}:{minute:02}:{second:02}"),
                })
            });
    } else {
        crate::localize_metadata(&mut entry.metadata, locale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::*;

    struct Locale;
    impl LocaleService for Locale {
        fn number_locale(&self) -> Result<NumberLocale, FsError> {
            Ok(NumberLocale::default())
        }
        fn format_archive_local_time(&self, value: ArchiveTimestamp) -> Option<LocalizedDateTime> {
            assert!(matches!(
                value,
                ArchiveTimestamp::Local {
                    year: 2024,
                    hour: 12,
                    ..
                }
            ));
            Some(LocalizedDateTime {
                date: "native date".into(),
                time: "native time".into(),
            })
        }
    }

    #[test]
    fn local_calendar_is_shown_without_inventing_a_utc_timestamp() {
        let mut entry = Entry::from_archive(ArchiveNode {
            id: VirtualEntryId { mount: 1, entry: 2 },
            parent: 1,
            raw_name: b"folder".as_slice().into(),
            display_name: "folder".into(),
            kind: EntryKind::Directory,
            size: None,
            readable: false,
            modified: Some(ArchiveTimestamp::Local {
                year: 2024,
                month: 2,
                day: 29,
                hour: 12,
                minute: 34,
                second: 56,
            }),
        });
        localize(&mut entry, None);
        assert_eq!(
            entry.metadata.localized_modified,
            Some(LocalizedDateTime {
                date: "2024-02-29".into(),
                time: "12:34:56".into(),
            })
        );
        assert!(entry.metadata.modified.is_none());
        localize(&mut entry, Some(&Locale));
        assert_eq!(
            entry.metadata.localized_modified.as_ref().unwrap().date,
            "native date"
        );
        assert_eq!(
            entry.metadata.localized_modified.as_ref().unwrap().time,
            "native time"
        );
        entry.virtual_node.as_mut().unwrap().modified = None;
        entry.metadata = EntryMetadata::default();
        localize(&mut entry, None);
        assert!(entry.metadata.localized_modified.is_none());
    }
}
