//! The third-party library list of the About dialog, read from the dependency
//! inventory that ships with the sources so the two cannot drift apart.
use crate::{LicenseRow, MainWindow};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;

const INVENTORY: &str = include_str!("../../../docs/third-party-inventory.csv");

/// Splits one CSV record; quoted fields may hold commas and doubled quotes.
fn fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    out.push(field);
    out
}

/// Name, version and license of every library, sorted and without repeats.
pub(crate) fn parse(inventory: &str) -> Vec<(String, String, String)> {
    let mut rows: Vec<_> = inventory
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = fields(line).into_iter();
            let (name, version) = (columns.next()?, columns.next()?);
            let license = columns.nth(1)?;
            (!name.is_empty()).then_some((name, version, license))
        })
        .collect();
    rows.sort_by_key(|(name, version, _)| (name.to_lowercase(), version.clone()));
    rows.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    rows
}

pub(crate) fn matching(
    rows: &[(String, String, String)],
    query: &str,
) -> Vec<(String, String, String)> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .filter(|(name, _, license)| {
            query.is_empty()
                || name.to_lowercase().contains(&query)
                || license.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

fn show(window: &MainWindow, rows: &[(String, String, String)], total: usize) {
    let model: Vec<LicenseRow> = rows
        .iter()
        .map(|(name, version, license)| LicenseRow {
            name: name.as_str().into(),
            version: version.as_str().into(),
            license: license.as_str().into(),
        })
        .collect();
    window.set_license_rows(ModelRc::from(Rc::new(VecModel::from(model))));
    window.set_license_summary(SharedString::from(if rows.len() == total {
        format!("{total} libraries")
    } else {
        format!("{} of {total} libraries", rows.len())
    }));
}

pub(crate) fn install(window: &MainWindow) {
    let rows = Rc::new(parse(INVENTORY));
    show(window, &rows, rows.len());
    let weak = window.as_weak();
    window.on_license_filter(move |query| {
        if let Some(window) = weak.upgrade() {
            show(&window, &matching(&rows, &query), rows.len());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_inventory_lists_every_library_with_a_license() {
        let rows = parse(INVENTORY);
        assert!(rows.len() > 400, "{} libraries", rows.len());
        assert!(rows.iter().all(|(name, version, license)| {
            !name.is_empty() && !version.is_empty() && !license.is_empty()
        }));
        assert!(rows.iter().any(|(name, _, _)| name == "slint"));
    }

    #[test]
    fn quoted_fields_keep_their_commas_and_filters_match_name_or_license() {
        assert_eq!(fields(r#"a,"b,c","d""e""#), ["a", "b,c", r#"d"e"#]);
        let rows = vec![
            ("Serde".into(), "1".into(), "MIT OR Apache-2.0".into()),
            ("zlib".into(), "2".into(), "Zlib".into()),
        ];
        assert_eq!(matching(&rows, " ser ").len(), 1);
        assert_eq!(matching(&rows, "SERDE").len(), 1);
        assert_eq!(matching(&rows, "zlib").len(), 1);
        assert_eq!(matching(&rows, "mit").len(), 1);
        assert_eq!(matching(&rows, "").len(), 2);
    }
}
