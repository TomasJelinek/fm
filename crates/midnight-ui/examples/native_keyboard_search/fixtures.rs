//! In-memory diagnostic configurations. Dummy commands are never executed.
use midnight_core::{
    PreferenceTheme, UiDensity, UserCommand, UserCommandId, UserCommandInvocation,
    UserCommandLaunch, UserPreferences,
};
use std::{path::Path, sync::Arc};

pub(super) fn load(path: &Path) -> Result<(String, UserPreferences, bool), String> {
    let name = std::env::args()
        .find_map(|argument| argument.strip_prefix("--fixture=").map(str::to_owned))
        .unwrap_or_else(|| "standard".into());
    let mut preferences = UserPreferences {
        theme: PreferenceTheme::Dracula,
        font_family: "Segoe UI".into(),
        desktop_font_size: 13,
        ..Default::default()
    };
    match name.as_str() {
        "standard" => {}
        "dynamic" | "compact" | "large" => {
            preferences.user_commands = Arc::new(
                (0..128)
                    .map(|index| UserCommand {
                        id: UserCommandId::new(format!("native-{index:03}")).expect("fixture ID"),
                        name: format!(
                            "Diagnostic {index:03} 文档 e\u{301} — {}",
                            "long label ".repeat(20)
                        ),
                        enabled: false,
                        toolbar_label: None,
                        icon: None,
                        working_directory: path.to_string_lossy().into_owned(),
                        launch: UserCommandLaunch::Single,
                        invocation: UserCommandInvocation::Program {
                            executable: path.join("never-executed.exe"),
                            arguments: Vec::new(),
                        },
                    })
                    .collect(),
            );
        }
        _ => return Err("Use --fixture=standard|dynamic|compact|large".into()),
    }
    if name == "large" {
        preferences.theme = PreferenceTheme::Light;
        preferences.font_family = "Arial".into();
        preferences.density = UiDensity::Touch;
        preferences.touch_font_size = 32;
    }
    preferences.validate().map_err(|error| error.to_string())?;
    let compact = name == "compact";
    Ok((name, preferences, compact))
}
