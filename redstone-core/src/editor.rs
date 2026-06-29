// redstone-core/src/editor.rs
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Pure data: input buffer, cursor, history. No I/O.
#[derive(Debug, Clone)]
pub struct InputState {
    pub input: String,
    pub cursor: usize,
    history: Vec<String>,
    history_pos: Option<usize>,
    staging: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputAction {
    None,
    Quit,
    Clear,
    Submit(String),
}

impl InputState {
    pub fn new() -> Self {
        Self {
            input: String::with_capacity(64),
            cursor: 0,
            history: Vec::new(),
            history_pos: None,
            staging: String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    pub fn handle_key(&mut self, key: KeyEvent) -> InputAction {
        if key.kind != KeyEventKind::Press {
            return InputAction::None;
        }

        if key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::CONTROL {
            return InputAction::Quit;
        }

        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            if self.input.is_empty() {
                return InputAction::Quit;
            }
            self.input.clear();
            self.cursor = 0;
            self.history_pos = None;
            self.staging.clear();
            return InputAction::Clear;
        }

        match key.code {
            KeyCode::Enter => {
                if self.input.is_empty() {
                    return InputAction::None;
                }
                let line = std::mem::take(&mut self.input);
                self.cursor = 0;
                self.history.push(line.clone());
                self.history_pos = None;
                self.staging.clear();
                InputAction::Submit(line)
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let prev = self.input[..self.cursor].chars().next_back().unwrap();
                    self.input.drain(self.cursor - prev.len_utf8()..self.cursor);
                    self.cursor -= prev.len_utf8();
                }
                InputAction::None
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    let ch = self.input[self.cursor..].chars().next().unwrap();
                    self.input.drain(self.cursor..self.cursor + ch.len_utf8());
                }
                InputAction::None
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    let prev = self.input[..self.cursor].chars().next_back().unwrap();
                    self.cursor -= prev.len_utf8();
                }
                InputAction::None
            }
            KeyCode::Right => {
                if self.cursor < self.input.len() {
                    let next = self.input[self.cursor..].chars().next().unwrap();
                    self.cursor += next.len_utf8();
                }
                InputAction::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                InputAction::None
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                InputAction::None
            }
            KeyCode::Up => {
                if self.history.is_empty() {
                    return InputAction::None;
                }
                if self.history_pos.is_none() {
                    self.staging = self.input.clone();
                }
                let pos = self.history_pos.unwrap_or(self.history.len());
                if pos > 0 {
                    self.history_pos = Some(pos - 1);
                    self.input = self.history[pos - 1].clone();
                    self.cursor = self.input.len();
                }
                InputAction::None
            }
            KeyCode::Down => {
                if let Some(pos) = self.history_pos {
                    if pos + 1 < self.history.len() {
                        self.history_pos = Some(pos + 1);
                        self.input = self.history[pos + 1].clone();
                    } else {
                        self.history_pos = None;
                        self.input = std::mem::take(&mut self.staging);
                    }
                    self.cursor = self.input.len();
                }
                InputAction::None
            }
            KeyCode::Tab => {
                self.input.insert(self.cursor, '\t');
                self.cursor += '\t'.len_utf8();
                InputAction::None
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                InputAction::None
            }
            _ => InputAction::None,
        }
    }

    pub fn input_visual_width(&self) -> (usize, usize) {
        use unicode_width::UnicodeWidthStr;
        let total = self.input.width();
        let prefix = self.input[..self.cursor].width();
        (prefix, total)
    }
}
