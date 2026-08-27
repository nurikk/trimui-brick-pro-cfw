use std::fmt;

use serde::Serialize;

pub const SCENE_WIDTH: u16 = 1024;
pub const SCENE_HEIGHT: u16 = 768;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Text,
    Secret,
    Numeric,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AllowedChars {
    pub letters: bool,
    pub digits: bool,
    pub punctuation: bool,
    pub whitespace: bool,
    pub unicode: bool,
    url_safe_only: bool,
}

impl AllowedChars {
    pub const fn ascii() -> Self {
        Self {
            letters: true,
            digits: true,
            punctuation: true,
            whitespace: true,
            unicode: false,
            url_safe_only: false,
        }
    }

    pub const fn any() -> Self {
        Self {
            letters: true,
            digits: true,
            punctuation: true,
            whitespace: true,
            unicode: true,
            url_safe_only: false,
        }
    }

    pub const fn numeric() -> Self {
        Self {
            letters: false,
            digits: true,
            punctuation: false,
            whitespace: false,
            unicode: false,
            url_safe_only: false,
        }
    }

    pub const fn url_safe() -> Self {
        Self {
            letters: true,
            digits: true,
            punctuation: true,
            whitespace: false,
            unicode: false,
            url_safe_only: true,
        }
    }

    fn accepts(self, character: char) -> bool {
        if character.is_control() || (!self.unicode && !character.is_ascii()) {
            return false;
        }
        if self.url_safe_only {
            return character.is_ascii_alphanumeric()
                || b"-._~:/?#[]@!$&'()*+,;=%".contains(&(character as u8));
        }
        (character.is_alphabetic() && self.letters)
            || (character.is_numeric() && self.digits)
            || (character.is_whitespace() && self.whitespace)
            || (character.is_ascii_punctuation() && self.punctuation)
    }
}

impl fmt::Debug for AllowedChars {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AllowedChars")
            .field("letters", &self.letters)
            .field("digits", &self.digits)
            .field("punctuation", &self.punctuation)
            .field("whitespace", &self.whitespace)
            .field("unicode", &self.unicode)
            .field("url_safe_only", &self.url_safe_only)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationState {
    Valid,
    Invalid,
}

pub struct FieldPolicy {
    kind: FieldKind,
    allowed: AllowedChars,
    max_bytes: usize,
    max_scalars: usize,
    initial_value: String,
    placeholder: String,
    validation: ValidationState,
}

impl FieldPolicy {
    pub fn text(
        initial: &str,
        placeholder: &str,
        max_bytes: usize,
        max_scalars: usize,
        allowed: AllowedChars,
    ) -> Self {
        Self::new(
            FieldKind::Text,
            initial,
            placeholder,
            max_bytes,
            max_scalars,
            allowed,
            ValidationState::Valid,
        )
    }

    pub fn secret(
        initial: &str,
        placeholder: &str,
        max_bytes: usize,
        max_scalars: usize,
        allowed: AllowedChars,
    ) -> Self {
        Self::new(
            FieldKind::Secret,
            initial,
            placeholder,
            max_bytes,
            max_scalars,
            allowed,
            ValidationState::Valid,
        )
    }

    pub fn numeric(initial: &str, placeholder: &str, max_bytes: usize, max_scalars: usize) -> Self {
        Self::new(
            FieldKind::Numeric,
            initial,
            placeholder,
            max_bytes,
            max_scalars,
            AllowedChars::numeric(),
            ValidationState::Valid,
        )
    }

    pub fn with_validation(mut self, validation: ValidationState) -> Self {
        self.validation = validation;
        self
    }

    fn new(
        kind: FieldKind,
        initial: &str,
        placeholder: &str,
        max_bytes: usize,
        max_scalars: usize,
        allowed: AllowedChars,
        validation: ValidationState,
    ) -> Self {
        Self {
            kind,
            allowed,
            max_bytes,
            max_scalars,
            initial_value: initial.to_owned(),
            placeholder: placeholder.to_owned(),
            validation,
        }
    }
}

impl fmt::Debug for FieldPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FieldPolicy")
            .field("kind", &self.kind)
            .field("allowed", &self.allowed)
            .field("max_bytes", &self.max_bytes)
            .field("max_scalars", &self.max_scalars)
            .field(
                "initial_value",
                &if self.kind == FieldKind::Secret {
                    "<redacted>"
                } else {
                    self.initial_value.as_str()
                },
            )
            .field(
                "placeholder",
                &if self.kind == FieldKind::Secret {
                    "<redacted>"
                } else {
                    self.placeholder.as_str()
                },
            )
            .field("validation", &self.validation)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutId {
    LowercaseQwerty,
    UppercaseShift,
    UppercaseCaps,
    Symbols,
    NumericKeypad,
    UrlSafe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    Primary,
    Secondary,
    LeftShoulder,
    RightShoulder,
    Start,
    Menu,
    Select,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WrapMode {
    None,
    Horizontal,
    Vertical,
    Both,
}

impl WrapMode {
    fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum KeyAction {
    Character(char),
    Backspace,
    Delete,
    Clear,
    Space,
    Switch(LayoutId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KeyCell {
    pub id: String,
    pub label: String,
    pub action: KeyAction,
}

impl KeyCell {
    fn new(row: usize, column: usize, label: &str, action: KeyAction) -> Self {
        Self {
            id: format!("key-{row}-{column}"),
            label: label.to_owned(),
            action,
        }
    }

    fn character(row: usize, column: usize, character: char) -> Self {
        Self::new(
            row,
            column,
            &character.to_string(),
            KeyAction::Character(character),
        )
    }
}

pub fn key_grid(layout: LayoutId) -> Vec<Vec<KeyCell>> {
    let rows: &[&str] = match layout {
        LayoutId::LowercaseQwerty => &["qwertyuiop", "asdfghjkl", "zxcvbnm"],
        LayoutId::UppercaseShift | LayoutId::UppercaseCaps => {
            &["QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"]
        }
        LayoutId::Symbols => &["!@#$%^&*()", "-_+=[]{}", ";:'\",.<>?", "/\\|"],
        LayoutId::NumericKeypad => &["123", "456", "789", "0"],
        LayoutId::UrlSafe => &[
            "qwertyuiop",
            "asdfghjkl",
            "zxcvbnm",
            "0123456789",
            "-._~:/?#[]@",
            "!$&'()*+",
            ",;=%",
        ],
    };
    let mut grid = rows
        .iter()
        .enumerate()
        .map(|(row, values)| {
            values
                .chars()
                .enumerate()
                .map(|(column, character)| KeyCell::character(row, column, character))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let row = grid.len();
    let controls = match layout {
        LayoutId::LowercaseQwerty => vec![
            KeyCell::new(row, 0, "SHIFT", KeyAction::Switch(LayoutId::UppercaseShift)),
            KeyCell::new(row, 1, "SYM", KeyAction::Switch(LayoutId::Symbols)),
            KeyCell::new(row, 2, "SPACE", KeyAction::Space),
            KeyCell::new(row, 3, "BACK", KeyAction::Backspace),
            KeyCell::new(row, 4, "DEL", KeyAction::Delete),
            KeyCell::new(row, 5, "CLEAR", KeyAction::Clear),
        ],
        LayoutId::UppercaseShift => vec![
            KeyCell::new(row, 0, "CAPS", KeyAction::Switch(LayoutId::UppercaseCaps)),
            KeyCell::new(row, 1, "SYM", KeyAction::Switch(LayoutId::Symbols)),
            KeyCell::new(row, 2, "SPACE", KeyAction::Space),
            KeyCell::new(row, 3, "BACK", KeyAction::Backspace),
            KeyCell::new(row, 4, "DEL", KeyAction::Delete),
            KeyCell::new(row, 5, "CLEAR", KeyAction::Clear),
        ],
        LayoutId::UppercaseCaps => vec![
            KeyCell::new(row, 0, "abc", KeyAction::Switch(LayoutId::LowercaseQwerty)),
            KeyCell::new(row, 1, "SYM", KeyAction::Switch(LayoutId::Symbols)),
            KeyCell::new(row, 2, "SPACE", KeyAction::Space),
            KeyCell::new(row, 3, "BACK", KeyAction::Backspace),
            KeyCell::new(row, 4, "DEL", KeyAction::Delete),
            KeyCell::new(row, 5, "CLEAR", KeyAction::Clear),
        ],
        LayoutId::Symbols => vec![
            KeyCell::new(row, 0, "abc", KeyAction::Switch(LayoutId::LowercaseQwerty)),
            KeyCell::new(row, 1, "SPACE", KeyAction::Space),
            KeyCell::new(row, 2, "BACK", KeyAction::Backspace),
            KeyCell::new(row, 3, "DEL", KeyAction::Delete),
            KeyCell::new(row, 4, "CLEAR", KeyAction::Clear),
        ],
        LayoutId::NumericKeypad => vec![
            KeyCell::new(row, 0, "BACK", KeyAction::Backspace),
            KeyCell::new(row, 1, "DEL", KeyAction::Delete),
            KeyCell::new(row, 2, "CLEAR", KeyAction::Clear),
        ],
        LayoutId::UrlSafe => vec![
            KeyCell::new(row, 0, "SPACE", KeyAction::Space),
            KeyCell::new(row, 1, "BACK", KeyAction::Backspace),
            KeyCell::new(row, 2, "DEL", KeyAction::Delete),
            KeyCell::new(row, 3, "CLEAR", KeyAction::Clear),
        ],
    };
    grid.push(controls);
    grid
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RejectReason {
    DisallowedCharacter,
    MaxBytes,
    MaxScalars,
    InvalidNumeric,
}

#[derive(Clone, Eq, PartialEq)]
pub enum TypedValue {
    Text(String),
    Secret(String),
    Numeric(u64),
}

impl fmt::Debug for TypedValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(value) => formatter.debug_tuple("Text").field(value).finish(),
            Self::Secret(_) => formatter
                .debug_tuple("Secret")
                .field(&"<redacted>")
                .finish(),
            Self::Numeric(value) => formatter.debug_tuple("Numeric").field(value).finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Navigation,
    CursorMovement,
    Insert,
    Backspace,
    Delete,
    Clear,
    LayoutSwitch,
    Confirm,
    Cancel,
    Reject,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticEvent {
    pub kind: EventKind,
    pub field: FieldKind,
    pub length_bytes: usize,
    pub length_scalars: usize,
    pub cursor_scalar: usize,
    pub layout: LayoutId,
}

#[derive(Clone, Eq, PartialEq)]
pub enum InputResult {
    Ignored,
    Navigated,
    Inserted,
    Rejected(RejectReason),
    LayoutChanged(LayoutId),
    Confirmed(TypedValue),
    Cancelled,
}

impl fmt::Debug for InputResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ignored => formatter.write_str("Ignored"),
            Self::Navigated => formatter.write_str("Navigated"),
            Self::Inserted => formatter.write_str("Inserted"),
            Self::Rejected(reason) => formatter.debug_tuple("Rejected").field(reason).finish(),
            Self::LayoutChanged(layout) => formatter
                .debug_tuple("LayoutChanged")
                .field(layout)
                .finish(),
            Self::Confirmed(value) => formatter.debug_tuple("Confirmed").field(value).finish(),
            Self::Cancelled => formatter.write_str("Cancelled"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TokenRole {
    Background,
    Text,
    Focus,
    Action,
    Error,
    Help,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SceneCell {
    pub key_id: String,
    pub label: String,
    pub role: TokenRole,
    pub focused: bool,
    pub focus_marker: String,
    pub hit_box: [u16; 4],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Scene {
    pub width: u16,
    pub height: u16,
    pub field: FieldKind,
    pub display: String,
    pub display_length_scalars: usize,
    pub cursor_scalar: usize,
    pub placeholder: String,
    pub validation: ValidationState,
    pub validation_label: String,
    pub layout: LayoutId,
    pub selected_key: String,
    pub cells: Vec<SceneCell>,
    pub token_roles: Vec<TokenRole>,
    pub help_strip: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticSnapshot {
    pub field: FieldKind,
    pub layout: LayoutId,
    pub length_bytes: usize,
    pub length_scalars: usize,
    pub cursor_byte: usize,
    pub cursor_scalar: usize,
    pub display: String,
    pub selected_key: String,
    pub validation: ValidationState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyboardError {
    InvalidInitialValue,
}

impl fmt::Display for KeyboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInitialValue => "initial value does not satisfy field policy",
        })
    }
}

impl std::error::Error for KeyboardError {}

pub struct Keyboard {
    policy: FieldPolicy,
    original: String,
    buffer: String,
    cursor_byte: usize,
    layout: LayoutId,
    selected_row: usize,
    selected_column: usize,
    wrap: WrapMode,
    events: Vec<SemanticEvent>,
}

impl Keyboard {
    pub fn new(policy: FieldPolicy) -> Result<Self, KeyboardError> {
        let valid = policy.initial_value.len() <= policy.max_bytes
            && policy.initial_value.chars().count() <= policy.max_scalars
            && policy
                .initial_value
                .chars()
                .all(|character| policy.allowed.accepts(character))
            && (policy.kind != FieldKind::Numeric
                || policy
                    .initial_value
                    .chars()
                    .all(|character| character.is_ascii_digit()));
        if !valid {
            return Err(KeyboardError::InvalidInitialValue);
        }
        let layout = if policy.kind == FieldKind::Numeric {
            LayoutId::NumericKeypad
        } else {
            LayoutId::LowercaseQwerty
        };
        let length = policy.initial_value.len();
        Ok(Self {
            original: policy.initial_value.clone(),
            buffer: policy.initial_value.clone(),
            cursor_byte: length,
            policy,
            layout,
            selected_row: 0,
            selected_column: 0,
            wrap: WrapMode::Both,
            events: Vec::new(),
        })
    }

    pub fn set_wrap_mode(&mut self, wrap: WrapMode) {
        self.wrap = wrap;
        self.normalize_selection();
    }

    pub fn layout(&self) -> LayoutId {
        self.layout
    }

    pub fn set_layout(&mut self, layout: LayoutId) {
        self.layout = layout;
        self.selected_row = 0;
        self.selected_column = 0;
    }

    pub fn selected_key(&self) -> KeyCell {
        key_grid(self.layout)[self.selected_row][self.selected_column].clone()
    }

    pub fn semantic_snapshot(&self) -> SemanticSnapshot {
        SemanticSnapshot {
            field: self.policy.kind,
            layout: self.layout,
            length_bytes: self.buffer.len(),
            length_scalars: self.buffer.chars().count(),
            cursor_byte: self.cursor_byte,
            cursor_scalar: self.buffer[..self.cursor_byte].chars().count(),
            display: self.display(),
            selected_key: self.selected_key().id,
            validation: self.policy.validation,
        }
    }

    pub fn scene(&self) -> Scene {
        let grid = key_grid(self.layout);
        let mut cells = Vec::new();
        let row_step = if grid.len() > 7 { 60 } else { 70 };
        for (row, keys) in grid.iter().enumerate() {
            for (column, key) in keys.iter().enumerate() {
                let focused = row == self.selected_row && column == self.selected_column;
                cells.push(SceneCell {
                    key_id: key.id.clone(),
                    label: key.label.clone(),
                    role: if focused {
                        TokenRole::Focus
                    } else if matches!(&key.action, KeyAction::Character(_) | KeyAction::Space) {
                        TokenRole::Text
                    } else {
                        TokenRole::Action
                    },
                    focused,
                    focus_marker: if focused {
                        ">".to_owned()
                    } else {
                        String::new()
                    },
                    hit_box: [48 + column as u16 * 88, 260 + row as u16 * row_step, 80, 60],
                });
            }
        }
        Scene {
            width: SCENE_WIDTH,
            height: SCENE_HEIGHT,
            field: self.policy.kind,
            display: self.display(),
            display_length_scalars: self.buffer.chars().count(),
            cursor_scalar: self.buffer[..self.cursor_byte].chars().count(),
            placeholder: if self.buffer.is_empty() && self.policy.kind != FieldKind::Secret {
                self.policy.placeholder.clone()
            } else {
                String::new()
            },
            validation: self.policy.validation,
            validation_label: if self.policy.validation == ValidationState::Invalid {
                "Error".to_owned()
            } else {
                "Valid".to_owned()
            },
            layout: self.layout,
            selected_key: self.selected_key().id,
            cells,
            token_roles: vec![
                TokenRole::Background,
                TokenRole::Text,
                TokenRole::Focus,
                TokenRole::Action,
                TokenRole::Error,
                TokenRole::Help,
            ],
            help_strip: vec![
                "D-pad: move".to_owned(),
                "A: select".to_owned(),
                "B: back".to_owned(),
                "L/R: cursor".to_owned(),
                "START: confirm".to_owned(),
                "MENU: cancel".to_owned(),
            ],
        }
    }

    pub fn semantic_scene_snapshot(&self) -> Vec<u8> {
        serde_json::to_vec(&self.scene()).expect("scene is serializable")
    }

    pub fn drain_events(&mut self) -> Vec<SemanticEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn press(&mut self, button: Button) -> InputResult {
        let selected_action = (button == Button::Primary).then_some(self.selected_key().action);
        let result = match button {
            Button::Up => {
                self.move_selection(-1, 0);
                InputResult::Navigated
            }
            Button::Down => {
                self.move_selection(1, 0);
                InputResult::Navigated
            }
            Button::Left => {
                self.move_selection(0, -1);
                InputResult::Navigated
            }
            Button::Right => {
                self.move_selection(0, 1);
                InputResult::Navigated
            }
            Button::LeftShoulder => self.move_cursor_left(),
            Button::RightShoulder => self.move_cursor_right(),
            Button::Primary => self.activate_selected(),
            Button::Secondary | Button::Menu => self.cancel(),
            Button::Start => self.confirm(),
            Button::Select => InputResult::Ignored,
        };
        let event = match (&result, button, selected_action.as_ref()) {
            (InputResult::Navigated, Button::LeftShoulder | Button::RightShoulder, _) => {
                Some(EventKind::CursorMovement)
            }
            (InputResult::Navigated, _, _) => Some(EventKind::Navigation),
            (InputResult::Inserted, Button::Primary, Some(KeyAction::Backspace)) => {
                Some(EventKind::Backspace)
            }
            (InputResult::Inserted, Button::Primary, Some(KeyAction::Delete)) => {
                Some(EventKind::Delete)
            }
            (InputResult::Inserted, Button::Primary, Some(KeyAction::Clear)) => {
                Some(EventKind::Clear)
            }
            (InputResult::Inserted, _, _) => Some(EventKind::Insert),
            (InputResult::Rejected(_), _, _) => Some(EventKind::Reject),
            (InputResult::LayoutChanged(_), _, _) => Some(EventKind::LayoutSwitch),
            (InputResult::Confirmed(_), _, _) => Some(EventKind::Confirm),
            (InputResult::Cancelled, _, _) => Some(EventKind::Cancel),
            _ => None,
        };
        if let Some(event) = event {
            self.record(event);
        }
        result
    }

    fn activate_selected(&mut self) -> InputResult {
        match self.selected_key().action.clone() {
            KeyAction::Character(character) => self.insert(character),
            KeyAction::Space => self.insert(' '),
            KeyAction::Backspace => self.backspace(),
            KeyAction::Delete => self.delete(),
            KeyAction::Clear => {
                if self.buffer.is_empty() {
                    InputResult::Ignored
                } else {
                    self.buffer.clear();
                    self.cursor_byte = 0;
                    InputResult::Inserted
                }
            }
            KeyAction::Switch(layout) => {
                self.layout = layout;
                self.selected_row = 0;
                self.selected_column = 0;
                InputResult::LayoutChanged(layout)
            }
        }
    }

    fn insert(&mut self, character: char) -> InputResult {
        if !self.policy.allowed.accepts(character)
            || (self.policy.kind == FieldKind::Numeric && !character.is_ascii_digit())
        {
            return InputResult::Rejected(RejectReason::DisallowedCharacter);
        }
        if self.buffer.len() + character.len_utf8() > self.policy.max_bytes {
            return InputResult::Rejected(RejectReason::MaxBytes);
        }
        if self.buffer.chars().count() == self.policy.max_scalars {
            return InputResult::Rejected(RejectReason::MaxScalars);
        }
        self.buffer.insert(self.cursor_byte, character);
        self.cursor_byte += character.len_utf8();
        if self.layout == LayoutId::UppercaseShift {
            self.layout = LayoutId::LowercaseQwerty;
            self.selected_row = 0;
            self.selected_column = 0;
        }
        InputResult::Inserted
    }

    fn backspace(&mut self) -> InputResult {
        if self.cursor_byte == 0 {
            return InputResult::Ignored;
        }
        let start = self.buffer[..self.cursor_byte]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.buffer.drain(start..self.cursor_byte);
        self.cursor_byte = start;
        InputResult::Inserted
    }

    fn delete(&mut self) -> InputResult {
        let Some(character) = self.buffer[self.cursor_byte..].chars().next() else {
            return InputResult::Ignored;
        };
        let end = self.cursor_byte + character.len_utf8();
        self.buffer.drain(self.cursor_byte..end);
        InputResult::Inserted
    }

    fn move_cursor_left(&mut self) -> InputResult {
        if self.cursor_byte == 0 {
            return InputResult::Ignored;
        }
        self.cursor_byte = self.buffer[..self.cursor_byte]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        InputResult::Navigated
    }

    fn move_cursor_right(&mut self) -> InputResult {
        let Some(character) = self.buffer[self.cursor_byte..].chars().next() else {
            return InputResult::Ignored;
        };
        self.cursor_byte += character.len_utf8();
        InputResult::Navigated
    }

    fn confirm(&self) -> InputResult {
        if self.policy.kind == FieldKind::Numeric {
            if self.buffer.is_empty() {
                return InputResult::Rejected(RejectReason::InvalidNumeric);
            }
            return self.buffer.parse::<u64>().map(TypedValue::Numeric).map_or(
                InputResult::Rejected(RejectReason::InvalidNumeric),
                InputResult::Confirmed,
            );
        }
        match self.policy.kind {
            FieldKind::Text => InputResult::Confirmed(TypedValue::Text(self.buffer.clone())),
            FieldKind::Secret => InputResult::Confirmed(TypedValue::Secret(self.buffer.clone())),
            FieldKind::Numeric => unreachable!(),
        }
    }

    fn cancel(&mut self) -> InputResult {
        self.buffer.clone_from(&self.original);
        self.cursor_byte = self.buffer.len();
        InputResult::Cancelled
    }

    fn move_selection(&mut self, row_delta: isize, column_delta: isize) {
        let grid = key_grid(self.layout);
        if column_delta != 0 {
            let row = &grid[self.selected_row];
            let next = self.selected_column as isize + column_delta;
            if next >= 0 && (next as usize) < row.len() {
                self.selected_column = next as usize;
            } else if self.wrap.horizontal() {
                self.selected_column = if next < 0 { row.len() - 1 } else { 0 };
            }
            return;
        }
        let next = self.selected_row as isize + row_delta;
        if next >= 0 && (next as usize) < grid.len() {
            self.selected_row = next as usize;
        } else if self.wrap.vertical() {
            self.selected_row = if next < 0 { grid.len() - 1 } else { 0 };
        }
        self.selected_column = self.selected_column.min(grid[self.selected_row].len() - 1);
    }

    fn normalize_selection(&mut self) {
        let grid = key_grid(self.layout);
        self.selected_row = self.selected_row.min(grid.len() - 1);
        self.selected_column = self.selected_column.min(grid[self.selected_row].len() - 1);
    }

    fn record(&mut self, kind: EventKind) {
        self.events.push(SemanticEvent {
            kind,
            field: self.policy.kind,
            length_bytes: self.buffer.len(),
            length_scalars: self.buffer.chars().count(),
            cursor_scalar: self.buffer[..self.cursor_byte].chars().count(),
            layout: self.layout,
        });
    }

    fn display(&self) -> String {
        if self.policy.kind == FieldKind::Secret {
            "*".repeat(self.buffer.chars().count())
        } else if self.buffer.is_empty() {
            String::new()
        } else {
            self.buffer.clone()
        }
    }
}

impl fmt::Debug for Keyboard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Keyboard")
            .field("policy", &self.policy)
            .field("length_bytes", &self.buffer.len())
            .field("length_scalars", &self.buffer.chars().count())
            .field("cursor_byte", &self.cursor_byte)
            .field("layout", &self.layout)
            .field("selected_row", &self.selected_row)
            .field("selected_column", &self.selected_column)
            .field("wrap", &self.wrap)
            .field("events", &self.events)
            .finish()
    }
}
