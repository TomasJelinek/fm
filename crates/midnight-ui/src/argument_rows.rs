//! Literal argument mutations shared by the two settings forms.
use slint::{Model, ModelRc, SharedString, VecModel};

pub(crate) fn sync(model: ModelRc<SharedString>, values: &[String]) -> ModelRc<SharedString> {
    if let Some(rows) = model.as_any().downcast_ref::<VecModel<SharedString>>() {
        while rows.row_count() > values.len() {
            rows.remove(rows.row_count() - 1);
        }
        for (index, value) in values.iter().enumerate() {
            if index == rows.row_count() {
                rows.push(value.as_str().into());
            } else if rows.row_data(index).as_deref() != Some(value.as_str()) {
                rows.set_row_data(index, value.as_str().into());
            }
        }
        model
    } else {
        ModelRc::new(VecModel::from(
            values.iter().map(|v| v.as_str().into()).collect::<Vec<_>>(),
        ))
    }
}

pub(crate) fn change(values: &mut Vec<String>, operation: i32, row: i32, text: &str) -> bool {
    let index = usize::try_from(row).ok();
    match operation {
        0 => {
            let Some(value) = index.and_then(|i| values.get_mut(i)) else {
                return false;
            };
            if value == text {
                return false;
            }
            *value = text.into();
        }
        1 if values.len() < 64 => values.push(text.into()),
        2 => {
            let Some(index) = index.filter(|i| *i < values.len()) else {
                return false;
            };
            values.remove(index);
        }
        -1 | 3 => {
            let Some(index) = index.filter(|i| *i < values.len()) else {
                return false;
            };
            let next = if operation == -1 {
                index.checked_sub(1)
            } else {
                index.checked_add(1)
            };
            let Some(next) = next.filter(|i| *i < values.len()) else {
                return false;
            };
            values.swap(index, next);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_keep_empty_unicode_and_whitespace_as_distinct_arguments() {
        let mut values = vec!["one two".into(), "".into(), "文档".into()];
        assert!(change(&mut values, 0, 1, " \"x\" "));
        assert!(change(&mut values, -1, 2, ""));
        assert_eq!(values, ["one two", "文档", " \"x\" "]);
        assert!(change(&mut values, 1, 0, ""));
        assert_eq!(values.last().unwrap(), "");
        let before = values.clone();
        assert!(!change(&mut values, 2, -1, ""));
        assert!(!change(&mut values, 3, 3, ""));
        assert_eq!(values, before);
    }
    #[test]
    fn insertion_budget_does_not_truncate_existing_arguments() {
        let mut values = vec![String::new(); 64];
        assert!(!change(&mut values, 1, 0, "extra"));
        assert_eq!(values.len(), 64);
    }
}
