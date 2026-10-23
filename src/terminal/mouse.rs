//! Mouse reporting: what a program that asked for the mouse (vim with
//! `mouse=a`, htop, tmux) is told of a click, a drag or the wheel, in the
//! encodings xterm defines.

use alacritty_terminal::term::TermMode;

/// The button a report names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportButton {
    Left,
    Middle,
    WheelUp,
    WheelDown,
    /// Motion with no button held, which only 1003 asks for.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseReport {
    pub button: ReportButton,
    pub kind: ReportKind,
    /// The cell, counted from the top left of the screen from 0.
    pub column: usize,
    pub row: usize,
    pub alt: bool,
    pub control: bool,
}

/// Whether the program asked to hear of `kind`: 1000 reports presses,
/// releases and the wheel, 1002 motion while a button is held as well, 1003
/// all motion.
pub fn wants(mode: TermMode, kind: ReportKind, button_held: bool) -> bool {
    match kind {
        ReportKind::Press | ReportKind::Release => mode.intersects(TermMode::MOUSE_MODE),
        ReportKind::Motion => {
            mode.contains(TermMode::MOUSE_MOTION)
                || (button_held && mode.contains(TermMode::MOUSE_DRAG))
        }
    }
}

/// The bytes that tell the program of `report`. `None` where the encoding
/// in use cannot say it: the default one stops at column and row 223.
pub fn encode(report: MouseReport, mode: TermMode) -> Option<Vec<u8>> {
    let button = match report.button {
        ReportButton::Left => 0,
        ReportButton::Middle => 1,
        ReportButton::None => 3,
        ReportButton::WheelUp => 64,
        ReportButton::WheelDown => 65,
    };
    let modifiers = 8 * u32::from(report.alt) + 16 * u32::from(report.control);
    let motion = if report.kind == ReportKind::Motion {
        32
    } else {
        0
    };
    let column = report.column as u32 + 1;
    let row = report.row as u32 + 1;
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if report.kind == ReportKind::Release {
            'm'
        } else {
            'M'
        };
        let code = button + modifiers + motion;
        return Some(format!("\x1b[<{code};{column};{row}{end}").into_bytes());
    }
    // The older encodings cannot tell which button went up.
    let code = if report.kind == ReportKind::Release {
        3 + modifiers
    } else {
        button + modifiers + motion
    };
    let mut bytes = b"\x1b[M".to_vec();
    bytes.push(32 + code as u8);
    for value in [column, row] {
        let value = 32 + value;
        if mode.contains(TermMode::UTF8_MOUSE) {
            // Two UTF-8 bytes at most, as in xterm.
            let character = char::from_u32(value).filter(|_| value < 0x800)?;
            let mut buffer = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
        } else {
            bytes.push(u8::try_from(value).ok()?);
        }
    }
    Some(bytes)
}
