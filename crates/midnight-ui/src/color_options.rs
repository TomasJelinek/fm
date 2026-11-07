//! Bounded editor input; syntax compilation is delegated to the backend.
use crate::{ColorOptionsView, FilenameColorDraft, MainWindow, Presenter};
use midnight_core::*;
use slint::{Color, ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub(crate) struct Editor {
    drafts: Vec<FilenameColorDraft>,
    selected: usize,
    pub(crate) dirty: bool,
}

fn from_rule(rule: &FilenameColorRule) -> FilenameColorDraft {
    FilenameColorDraft {
        enabled: rule.enabled,
        mode: i32::from(rule.mode == FilterMode::Regex),
        expression: rule.expression.as_str().into(),
        case_sensitive: rule.case_sensitive,
        target: match rule.target {
            FilenameColorTarget::Files => 0,
            FilenameColorTarget::Folders => 1,
            FilenameColorTarget::All => 2,
        },
        foreground: rule
            .style
            .foreground
            .map(RgbColor::hex)
            .unwrap_or_default()
            .into(),
        background: rule
            .style
            .background
            .map(RgbColor::hex)
            .unwrap_or_default()
            .into(),
    }
}

fn parse_color(text: &str) -> Result<Option<RgbColor>, &'static str> {
    if text.trim().is_empty() {
        Ok(None)
    } else {
        RgbColor::parse(text.trim()).map(Some)
    }
}

fn to_rule(draft: &FilenameColorDraft) -> Result<FilenameColorRule, &'static str> {
    Ok(FilenameColorRule {
        enabled: draft.enabled,
        mode: match draft.mode {
            0 => FilterMode::Masks,
            1 => FilterMode::Regex,
            _ => return Err("Unknown rule syntax."),
        },
        expression: draft.expression.to_string(),
        case_sensitive: draft.case_sensitive,
        target: match draft.target {
            0 => FilenameColorTarget::Files,
            1 => FilenameColorTarget::Folders,
            2 => FilenameColorTarget::All,
            _ => return Err("Unknown rule target."),
        },
        style: FilenameStyle {
            foreground: parse_color(&draft.foreground)?,
            background: parse_color(&draft.background)?,
        },
    })
}

pub(crate) fn color(value: RgbColor) -> Color {
    Color::from_rgb_u8(value.0, value.1, value.2)
}

fn sync_editor(presenter: &mut Presenter) {
    let editor = &mut presenter.colors.editor;
    if !editor.dirty {
        editor.drafts = presenter
            .preferences
            .values
            .filename_colors
            .iter()
            .map(from_rule)
            .collect();
        editor.selected = editor.selected.min(editor.drafts.len().saturating_sub(1));
    }
}

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let editor = &presenter.colors.editor;
    let saved: Vec<_> = presenter
        .preferences
        .values
        .filename_colors
        .iter()
        .map(from_rule)
        .collect();
    let drafts = if editor.dirty { &editor.drafts } else { &saved };
    let view = window.global::<ColorOptionsView>();
    let labels: Vec<slint::SharedString> = drafts
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let mut expression: String = rule.expression.chars().take(80).collect();
            if expression.len() < rule.expression.len() {
                expression.push('…');
            }
            format!(
                "{}. {}{}",
                index + 1,
                if rule.enabled { "" } else { "[off] " },
                expression
            )
            .into()
        })
        .collect();
    let previous = view.get_labels();
    if previous.row_count() != labels.len()
        || previous.iter().zip(&labels).any(|(old, new)| old != *new)
    {
        view.set_labels(ModelRc::new(VecModel::from(labels)));
    }
    view.set_selected(if drafts.is_empty() {
        -1
    } else {
        editor.selected.min(drafts.len() - 1) as i32
    });
    let draft = drafts
        .get(editor.selected.min(drafts.len().saturating_sub(1)))
        .cloned()
        .unwrap_or_default();
    if view.get_draft() != draft {
        view.set_draft(draft.clone());
    }
    let foreground = parse_color(&draft.foreground).ok().flatten();
    let background = parse_color(&draft.background).ok().flatten();
    view.set_preview_foreground_set(foreground.is_some());
    view.set_preview_background_set(background.is_some());
    view.set_preview_foreground(color(foreground.unwrap_or_default()));
    view.set_preview_background(color(background.unwrap_or_default()));
    view.set_busy(presenter.colors.validation.is_some());
    view.set_error(presenter.colors.error.as_str().into());
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    crate::filename_colors::install(window);
    let weak = window.as_weak();
    let state = presenter.clone();
    window.global::<ColorOptionsView>().on_select(move |index| {
        let Some(window) = weak.upgrade() else { return };
        let mut presenter = state.borrow_mut();
        sync_editor(&mut presenter);
        if let Ok(index) = usize::try_from(index)
            && index < presenter.colors.editor.drafts.len()
        {
            presenter.colors.editor.selected = index;
        }
        refresh(&window, &presenter);
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    window.global::<ColorOptionsView>().on_edit(move |draft| {
        let Some(window) = weak.upgrade() else { return };
        let mut presenter = state.borrow_mut();
        sync_editor(&mut presenter);
        if presenter.colors.validation.is_some() {
            return;
        }
        if draft.expression.len() > MAX_FILTER_BYTES
            || draft.foreground.len() > 64
            || draft.background.len() > 64
        {
            presenter.colors.error =
                "Expression limit: 4096 bytes. Colors use #RRGGBB or an empty value.".into();
        } else {
            let editor = &mut presenter.colors.editor;
            if let Some(previous) = editor.drafts.get_mut(editor.selected) {
                *previous = draft;
                editor.dirty = true;
            }
            presenter.colors.error.clear();
        }
        refresh(&window, &presenter);
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    window
        .global::<ColorOptionsView>()
        .on_action(move |action| {
            let Some(window) = weak.upgrade() else { return };
            let mut presenter = state.borrow_mut();
            sync_editor(&mut presenter);
            if presenter.colors.validation.is_some() {
                return;
            }
            let editor = &mut presenter.colors.editor;
            match action {
                0 if editor.drafts.len() < MAX_FILENAME_COLOR_RULES => {
                    editor.drafts.push(from_rule(&FilenameColorRule::default()));
                    editor.selected = editor.drafts.len() - 1;
                    editor.dirty = true;
                }
                1 if editor.selected < editor.drafts.len() => {
                    editor.drafts.remove(editor.selected);
                    editor.selected = editor.selected.min(editor.drafts.len().saturating_sub(1));
                    editor.dirty = true;
                }
                2..=3 => {
                    let other = if action == 2 {
                        editor.selected.checked_sub(1)
                    } else {
                        editor.selected.checked_add(1)
                    };
                    if let Some(other) = other
                        && other < editor.drafts.len()
                    {
                        editor.drafts.swap(editor.selected, other);
                        editor.selected = other;
                        editor.dirty = true;
                    }
                }
                4 => {
                    match editor
                        .drafts
                        .iter()
                        .map(to_rule)
                        .collect::<Result<Vec<_>, _>>()
                    {
                        Ok(rules) => presenter.validate_colors(rules, send.as_ref()),
                        Err(error) => presenter.colors.error = error.into(),
                    }
                }
                5 => {
                    editor.dirty = false;
                    presenter.colors.error.clear();
                }
                _ => {}
            }
            refresh(&window, &presenter);
        });
    sync_editor(&mut presenter.borrow_mut());
    refresh(window, &presenter.borrow());
}
