//! What a terminal has to tell the user while they look elsewhere: a
//! notification a program asks for (OSC 9 as iTerm2 has it, OSC 777 as urxvt
//! and VTE have it), or the bell.
//!
//! Alacritty drops both OSC sequences, so the parser thread runs the output
//! through `NoticeScanner` too. The same pass picks up the folder a shell
//! reports (OSC 7 and 1337, see `directory`), which Alacritty drops as well.

use alacritty_terminal::vte::{Parser, Perform};

use super::directory::{ReportedDirectory, reported_directory};

/// The longest title and body a notification shows, in characters.
const TITLE_LIMIT: usize = 80;
const BODY_LIMIT: usize = 240;

/// Something a terminal asks the user to look at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalNotice {
    /// A program asked for a notification. Its title, when it gave one.
    Program { title: Option<String>, body: String },
    /// The bell rang. The line the cursor is on, which a program waiting
    /// for an answer usually has its question on.
    Bell { line: String },
}

impl TerminalNotice {
    /// The bell, with the line the cursor is on.
    pub fn bell(line: &str) -> Self {
        TerminalNotice::Bell {
            line: clean_text(line, BODY_LIMIT),
        }
    }
}

/// A notification a program asked for, as it asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramNotice {
    pub title: Option<String>,
    pub body: String,
}

impl ProgramNotice {
    /// The notice to show: control characters out, long text cut.
    pub fn cleaned(self) -> TerminalNotice {
        TerminalNotice::Program {
            title: self
                .title
                .map(|title| clean_text(&title, TITLE_LIMIT))
                .filter(|title| !title.is_empty()),
            body: clean_text(&self.body, BODY_LIMIT),
        }
    }
}

/// Finds the notifications programs ask for in terminal output, however the
/// output is split into chunks.
pub struct NoticeScanner {
    parser: Parser,
    found: Vec<ProgramNotice>,
    directory: Option<ReportedDirectory>,
}

impl Default for NoticeScanner {
    fn default() -> Self {
        Self {
            parser: Parser::new(),
            found: Vec::new(),
            directory: None,
        }
    }
}

impl NoticeScanner {
    /// The notifications that end in `bytes`.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<ProgramNotice> {
        let mut found = Found {
            notices: &mut self.found,
            directory: &mut self.directory,
        };
        self.parser.advance(&mut found, bytes);
        std::mem::take(&mut self.found)
    }

    /// The last folder reported since the last time it was asked: only the
    /// latest one matters.
    pub fn take_directory(&mut self) -> Option<ReportedDirectory> {
        self.directory.take()
    }
}

struct Found<'a> {
    notices: &'a mut Vec<ProgramNotice>,
    directory: &'a mut Option<ReportedDirectory>,
}

impl Perform for Found<'_> {
    fn osc_dispatch(&mut self, params: &[&[u8]], _: bool) {
        if let Some(notice) = program_notice(params) {
            self.notices.push(notice);
        } else if let Some(directory) = reported_directory(params) {
            *self.directory = Some(directory);
        }
    }
}

/// The notification an OSC sequence asks for, if it is one:
/// - `9;body`, unless the body is a ConEmu command (`9;4;50` is a progress
///   bar, `9;9;/tmp` the working directory);
/// - `777;notify;title;body`.
fn program_notice(params: &[&[u8]]) -> Option<ProgramNotice> {
    // The text after the first parameters, `;` and all: a body may hold one.
    let rest = |from: usize| -> String {
        params
            .get(from..)
            .unwrap_or_default()
            .iter()
            .map(|param| String::from_utf8_lossy(param))
            .collect::<Vec<_>>()
            .join(";")
    };
    let notice = match *params.first()? {
        b"9" => {
            let body = rest(1);
            let conemu = params.len() > 1
                && !params[1].is_empty()
                && params[1].iter().all(u8::is_ascii_digit);
            ProgramNotice {
                title: None,
                body: (!conemu).then_some(body)?,
            }
        }
        b"777" if params.get(1) == Some(&&b"notify"[..]) => ProgramNotice {
            title: Some(String::from_utf8_lossy(params.get(2)?).into_owned()),
            body: rest(3),
        },
        _ => return None,
    };
    (!notice.body.trim().is_empty()).then_some(notice)
}

/// `text` on one line, as a notification shows it: control characters and
/// runs of white space become one space, and what is longer than `max`
/// characters is cut with an ellipsis.
pub fn clean_text(text: &str, max: usize) -> String {
    let words: Vec<&str> = text
        .split(|character: char| character.is_whitespace() || character.is_control())
        .filter(|word| !word.is_empty())
        .collect();
    let line = words.join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let mut cut: String = line.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}
