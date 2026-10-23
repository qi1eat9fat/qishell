//! 终端：alacritty 引擎、字节流传输与网格视图。

mod directory;
mod engine;
mod font;
mod links;
mod model;
mod mouse;
mod notices;
mod scrollbar;
mod search;
mod terminal_view;
mod theme;
mod transport;

pub use directory::ReportedDirectory;
pub use engine::{TerminalCell, TerminalEngine, TerminalEvent, TerminalSnapshot};
pub use font::{
    DEFAULT_FONT_SIZE, DEFAULT_LINE_HEIGHT, FONT_SIZE_RANGE, LINE_HEIGHT_RANGE, TerminalFont,
    TerminalFontPreview, is_font_installed, monospace_font_families,
};
pub use model::{RemoteTerminalId, TerminalLifecycle, TerminalSize, TerminalStatus};
pub use notices::TerminalNotice;
pub use search::{SearchDirection, SearchMark, SearchPosition};
pub use terminal_view::TerminalView;
pub(crate) use terminal_view::terminal_key_bindings;
pub use theme::{TerminalChrome, TerminalColors, TerminalTheme};
pub use transport::{
    FixedRemoteTerminalTransportProvider, RemoteTerminalTransportProvider,
    SharedRemoteTerminalTransportProvider, SharedTerminalTransportFactory, TerminalTransport,
    TerminalTransportCommand, TerminalTransportEvent, TerminalTransportFactory,
};
