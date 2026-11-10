//! Portable application shortcuts. Native editing and IME handling stay with the UI.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

mod catalog;
pub use catalog::shortcut_catalog;

pub const MAX_SHORTCUT_OVERRIDES: usize = 256;
pub const MAX_SHORTCUT_KEYS: usize = 4;
pub const MAX_SHORTCUT_CHORD_BYTES: usize = 128;
pub const MAX_SHORTCUT_ACTION_BYTES: usize = 96;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ShortcutChord {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    /// Canonical named key, ASCII uppercase character, or unchanged Unicode scalar.
    /// Digit0 through Digit9 identify physical keys, not layout-dependent text.
    pub key: String,
}

impl ShortcutChord {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty()
            || text.len() > MAX_SHORTCUT_CHORD_BYTES
            || text.chars().any(char::is_control)
        {
            return Err(
                "A shortcut must contain 1 to 128 bytes without control characters.".into(),
            );
        }
        let mut chord = Self::default();
        let mut parts = text.split('+').peekable();
        while let Some(part) = parts.next() {
            let part = part.trim();
            if parts.peek().is_none() {
                chord.key = canonical_key(part)?;
                break;
            }
            let flag = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut chord.control,
                "alt" => &mut chord.alt,
                "shift" => &mut chord.shift,
                "meta" | "super" | "win" => &mut chord.meta,
                _ => return Err(format!("Unknown shortcut modifier: {part}")),
            };
            if std::mem::replace(flag, true) {
                return Err(format!("Repeated shortcut modifier: {part}"));
            }
        }
        Ok(chord)
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::with_capacity(5);
        for (enabled, name) in [
            (self.control, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.meta, "Meta"),
        ] {
            if enabled {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("+")
    }

    fn search_clipboard_key(&self) -> bool {
        if self.alt {
            return false;
        }
        let clipboard_letter = !self.shift
            && (self.control ^ self.meta)
            && matches!(self.key.as_str(), "C" | "X" | "V");
        let insert_alias = self.key == "Insert" && !self.meta && (self.control ^ self.shift);
        clipboard_letter || insert_alias
    }
}

fn canonical_key(key: &str) -> Result<String, String> {
    let lower = key.to_ascii_lowercase();
    let named = match lower.as_str() {
        "enter" | "return" => "Enter",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "space" => "Space",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "insert" | "ins" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" | "pgup" => "PageUp",
        "pagedown" | "pgdown" | "pgdn" => "PageDown",
        "up" | "uparrow" | "arrowup" | "↑" => "Up",
        "down" | "downarrow" | "arrowdown" | "↓" => "Down",
        "left" | "leftarrow" | "arrowleft" | "←" => "Left",
        "right" | "rightarrow" | "arrowright" | "→" => "Right",
        "plus" => "Plus",
        "minus" | "-" => "Minus",
        "equal" | "equals" | "=" => "Equal",
        "backslash" | "\\" => "Backslash",
        "slash" | "/" => "Slash",
        "bracketleft" | "[" => "BracketLeft",
        "bracketright" | "]" => "BracketRight",
        "comma" | "," => "Comma",
        "period" | "." => "Period",
        "semicolon" | ";" => "Semicolon",
        "quote" | "'" => "Quote",
        "backquote" | "`" => "Backquote",
        _ => "",
    };
    if !named.is_empty() {
        return Ok(named.into());
    }
    if let Some(number) = lower
        .strip_prefix('f')
        .and_then(|digits| digits.parse::<u8>().ok())
        && (1..=24).contains(&number)
        && lower == format!("f{number}")
    {
        return Ok(format!("F{number}"));
    }
    if let Some(digit) = lower.strip_prefix("digit")
        && digit.len() == 1
        && digit.as_bytes()[0].is_ascii_digit()
    {
        return Ok(format!("Digit{digit}"));
    }
    let mut characters = key.chars();
    if let Some(character) = characters.next()
        && characters.next().is_none()
        && !character.is_whitespace()
        && !character.is_control()
    {
        if character.is_ascii_digit() {
            return Ok(format!("Digit{character}"));
        }
        return Ok(character.to_ascii_uppercase().to_string());
    }
    Err(format!(
        "Unknown shortcut key: {key}. Use a named key or one character."
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutBinding {
    pub action: String,
    /// Replaces every default alias. An empty list disables the action.
    pub keys: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutContext {
    Files,
    Search,
    Command,
    Other,
    Volume,
    Viewer,
    ViewerSearch,
}

const CONTEXTS: [ShortcutContext; 7] = [
    ShortcutContext::Files,
    ShortcutContext::Search,
    ShortcutContext::Command,
    ShortcutContext::Other,
    ShortcutContext::Volume,
    ShortcutContext::Viewer,
    ShortcutContext::ViewerSearch,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutScope {
    FilesIdle,
    FilesSearch,
    FilesAny,
    FilesOrCommand,
    Command,
    Global,
    Volume,
    ViewerContent,
    ViewerAny,
}

impl ShortcutScope {
    pub fn includes(self, context: ShortcutContext) -> bool {
        use ShortcutContext::*;
        match self {
            Self::FilesIdle => context == Files,
            Self::FilesSearch => context == Search,
            Self::FilesAny => matches!(context, Files | Search),
            Self::FilesOrCommand => matches!(context, Files | Search | Command),
            Self::Command => context == Command,
            Self::Global => !matches!(context, Viewer | ViewerSearch),
            Self::Volume => context == Volume,
            Self::ViewerContent => context == Viewer,
            Self::ViewerAny => matches!(context, Viewer | ViewerSearch),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::FilesIdle => "File list outside filename search",
            Self::FilesSearch => "Filename search",
            Self::FilesAny => "File list and filename search",
            Self::FilesOrCommand => "File list, filename search and command editor",
            Self::Command => "Command editor",
            Self::Global => "Application window",
            Self::Volume => "Volume choices popup",
            Self::ViewerContent => "Viewer content",
            Self::ViewerAny => "Viewer window and search",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutDefinition {
    pub id: String,
    pub label: String,
    pub defaults: Vec<String>,
    pub scope: ShortcutScope,
}

impl ShortcutDefinition {
    /// Text-sensitive aliases retain their narrower context even when reassigned.
    /// Clipboard keys edit search text; Backspace and plain/Shift digits stay native.
    pub fn scope_for(&self, chord: &ShortcutChord) -> ShortcutScope {
        let plain = !chord.control && !chord.alt && !chord.meta;
        if (self.scope == ShortcutScope::FilesAny && chord.search_clipboard_key())
            || (self.id == "panel.parent" && plain && !chord.shift && chord.key == "Backspace")
            || (self.id.starts_with("bookmark.open.") && plain && chord.key.starts_with("Digit"))
        {
            ShortcutScope::FilesIdle
        } else {
            self.scope
        }
    }

    fn validate_key(&self, chord: &ShortcutChord) -> Result<(), String> {
        if chord.control && !chord.alt && !chord.meta && chord.key == "Space" {
            return Err(format!(
                "{} is reserved for the input method (IME).",
                chord.display()
            ));
        }
        let scope = self.scope_for(chord);
        if scope == ShortcutScope::FilesSearch && chord.search_clipboard_key() {
            return Err(format!(
                "{} is reserved for clipboard editing in filename search.",
                chord.display()
            ));
        }
        if scope == ShortcutScope::Volume
            && !chord.control
            && !chord.alt
            && !chord.meta
            && matches!(
                chord.key.as_str(),
                "Enter"
                    | "Escape"
                    | "Tab"
                    | "Up"
                    | "Down"
                    | "Left"
                    | "Right"
                    | "Home"
                    | "End"
                    | "PageUp"
                    | "PageDown"
            )
        {
            return Err(format!(
                "{} is reserved for native popup navigation.",
                chord.display()
            ));
        }
        if !scope.includes(ShortcutContext::Command)
            && !scope.includes(ShortcutContext::Other)
            && !scope.includes(ShortcutContext::ViewerSearch)
        {
            return Ok(());
        }
        let key = chord.key.as_str();
        let editor_chord = !chord.alt
            && (chord.control || chord.meta)
            && matches!(
                key,
                "A" | "C"
                    | "X"
                    | "V"
                    | "Z"
                    | "Y"
                    | "Insert"
                    | "Backspace"
                    | "Delete"
                    | "Left"
                    | "Right"
                    | "Up"
                    | "Down"
                    | "Home"
                    | "End"
                    | "PageUp"
                    | "PageDown"
            );
        let plain = !chord.control && !chord.alt && !chord.meta;
        let printable = key.chars().count() == 1
            || key.starts_with("Digit")
            || matches!(
                key,
                "Space"
                    | "Plus"
                    | "Minus"
                    | "Equal"
                    | "Backslash"
                    | "Slash"
                    | "BracketLeft"
                    | "BracketRight"
                    | "Comma"
                    | "Period"
                    | "Semicolon"
                    | "Quote"
                    | "Backquote"
            );
        let editing_key = matches!(
            key,
            "Backspace"
                | "Delete"
                | "Insert"
                | "Left"
                | "Right"
                | "Up"
                | "Down"
                | "Home"
                | "End"
                | "PageUp"
                | "PageDown"
                | "Tab"
        );
        let viewer_escape = scope == ShortcutScope::ViewerAny && key == "Escape";
        let widget_submit =
            scope != ShortcutScope::Command && !viewer_escape && matches!(key, "Enter" | "Escape");
        if editor_chord || (plain && (printable || editing_key || widget_submit)) {
            Err(format!(
                "{} is reserved for native text editing in {}.",
                chord.display(),
                scope.label()
            ))
        } else {
            Ok(())
        }
    }
}

/// Compiled once when preferences change; resolving a chord performs no allocation.
#[derive(Clone, Debug)]
pub struct ShortcutMap {
    definitions: Arc<[ShortcutDefinition]>,
    resolved: HashMap<ShortcutChord, [Option<usize>; 7]>,
    bindings: HashMap<String, Vec<ShortcutChord>>,
}

impl ShortcutMap {
    pub fn new(overrides: &[ShortcutBinding]) -> Result<Self, String> {
        Self::with_user_commands(overrides, &[])
    }

    /// Include only explicitly configured command IDs; arbitrary dynamic actions remain invalid.
    pub fn with_user_commands(
        overrides: &[ShortcutBinding],
        commands: &[crate::UserCommand],
    ) -> Result<Self, String> {
        crate::validate_user_commands(commands).map_err(str::to_owned)?;
        if overrides.len() > MAX_SHORTCUT_OVERRIDES {
            return Err("At most 256 shortcut overrides can be saved.".into());
        }
        let mut definitions = shortcut_catalog().to_vec();
        definitions.extend(commands.iter().map(|command| ShortcutDefinition {
            id: command.id.action(),
            label: command.name.clone(),
            defaults: Vec::new(),
            scope: ShortcutScope::FilesIdle,
        }));
        let definitions: Arc<[ShortcutDefinition]> = definitions.into();
        let mut replacements = HashMap::new();
        let mut explicit_keys: HashMap<ShortcutChord, [bool; 7]> = HashMap::new();
        for binding in overrides {
            if binding.action.len() > MAX_SHORTCUT_ACTION_BYTES {
                return Err("A shortcut action identity exceeds 96 bytes.".into());
            }
            let Some(definition) = definitions
                .iter()
                .find(|definition| definition.id == binding.action)
            else {
                return Err(format!("Unknown shortcut action: {}", binding.action));
            };
            if binding.keys.len() > MAX_SHORTCUT_KEYS {
                return Err(format!(
                    "At most four keys can be assigned to {}.",
                    binding.action
                ));
            }
            if replacements
                .insert(binding.action.as_str(), &binding.keys)
                .is_some()
            {
                return Err(format!("Repeated shortcut action: {}", binding.action));
            }
            for text in &binding.keys {
                let chord = ShortcutChord::parse(text)?;
                let scope = definition.scope_for(&chord);
                let assigned = explicit_keys.entry(chord).or_default();
                for (position, context) in CONTEXTS.into_iter().enumerate() {
                    assigned[position] |= scope.includes(context);
                }
            }
        }
        let mut result = Self {
            definitions: definitions.clone(),
            resolved: HashMap::new(),
            bindings: HashMap::new(),
        };
        for (index, definition) in definitions.iter().enumerate() {
            let keys = replacements
                .get(definition.id.as_str())
                .copied()
                .unwrap_or(&definition.defaults);
            let mut seen = HashSet::new();
            let mut chords = Vec::with_capacity(keys.len());
            for text in keys {
                let chord = ShortcutChord::parse(text)?;
                definition.validate_key(&chord)?;
                let scope = definition.scope_for(&chord);
                // These defaults were added after configurable shortcuts. Keep
                // previously saved assignments valid, including their hints,
                // without weakening conflicts between other defaults or overrides.
                let added_default = matches!(
                    definition.id.as_str(),
                    "files.copy-to"
                        | "files.move-to"
                        | "files.cut"
                        | "files.find"
                        | "files.checksums"
                        | "files.compare"
                        | "selection.files"
                        | "selection.invert-all"
                        | "files.make-list"
                        | "viewer.document-next"
                        | "viewer.document-previous"
                        | "compare.next-difference"
                        | "compare.previous-difference"
                        | "compare.first-difference"
                        | "compare.last-difference"
                ) || (definition.id == "selection.all"
                    && chord.control
                    && chord.shift
                    && !chord.alt
                    && !chord.meta
                    && chord.key == "A")
                    || (definition.id.starts_with("bookmark.open.")
                        && !chord.control
                        && !chord.alt
                        && !chord.shift
                        && !chord.meta
                        && chord.key.starts_with("Digit"));
                if !replacements.contains_key(definition.id.as_str())
                    && added_default
                    && explicit_keys.get(&chord).is_some_and(|assigned| {
                        CONTEXTS.iter().enumerate().any(|(position, context)| {
                            assigned[position] && scope.includes(*context)
                        })
                    })
                {
                    continue;
                }
                if !seen.insert(chord.clone()) {
                    return Err(format!(
                        "Repeated shortcut {} for {}.",
                        chord.display(),
                        definition.label
                    ));
                }
                for (position, context) in CONTEXTS.into_iter().enumerate() {
                    if !scope.includes(context) {
                        continue;
                    }
                    if let Some(previous) =
                        result.resolved.entry(chord.clone()).or_default()[position].replace(index)
                    {
                        return Err(format!(
                            "{} conflicts between {} and {} in {:?}.",
                            chord.display(),
                            definitions[previous].label,
                            definition.label,
                            context
                        ));
                    }
                }
                chords.push(chord);
            }
            result.bindings.insert(definition.id.clone(), chords);
        }
        Ok(result)
    }

    pub fn resolve(
        &self,
        chord: &ShortcutChord,
        context: ShortcutContext,
    ) -> Option<&ShortcutDefinition> {
        let position = match context {
            ShortcutContext::Files => 0,
            ShortcutContext::Search => 1,
            ShortcutContext::Command => 2,
            ShortcutContext::Other => 3,
            ShortcutContext::Volume => 4,
            ShortcutContext::Viewer => 5,
            ShortcutContext::ViewerSearch => 6,
        };
        self.resolved.get(chord)?[position].map(|index| &self.definitions[index])
    }

    pub fn bindings(&self, action: &str) -> Option<&[ShortcutChord]> {
        self.bindings.get(action).map(Vec::as_slice)
    }

    /// Exact catalog used by this compiled map, including configured command IDs.
    pub fn definitions(&self) -> &[ShortcutDefinition] {
        &self.definitions
    }
}
