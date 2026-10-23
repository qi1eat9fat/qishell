use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::COUNT;
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::term::{ClipboardType, Config, Osc52, TermMode};
use alacritty_terminal::vte::ansi::{
    Color, CursorShape, CursorStyle, Processor, Rgb, Timeout as _,
};
use futures::future::{Either, select};
use gpui_kit::SharedString;

use crate::connection::{ConnectionPrompt, ConnectionPromptReply, Latency};

use super::directory::ReportedDirectory;
use super::links::{LinkMarker, url_search, visible_links};
use super::notices::{NoticeScanner, ProgramNotice, TerminalNotice};
use super::search::{MatchMarker, SearchDirection, SearchMark, SearchPosition, TerminalSearch};
use super::{
    SharedTerminalTransportFactory, TerminalColors, TerminalLifecycle, TerminalSize,
    TerminalStatus, TerminalTheme, TerminalTransportCommand, TerminalTransportEvent,
};

pub type AlacrittyTerm = Term<TerminalEventProxy>;

#[derive(Clone, Debug)]
pub struct TerminalCell {
    pub character: String,
    pub foreground: Color,
    pub background: Color,
    pub flags: Flags,
    pub selected: bool,
    pub search: SearchMark,
    /// The link the cell belongs to, an index into the snapshot's `links`.
    pub link: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct TerminalSnapshot {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<TerminalCell>,
    pub cursor_row: Option<usize>,
    pub cursor_column: usize,
    pub cursor_shape: CursorShape,
    pub cursor_blinking: bool,
    pub display_offset: usize,
    /// The lines above the screen, which the view can scroll back through.
    pub history_size: usize,
    pub colors: [Option<Rgb>; COUNT],
    /// The addresses of the links on screen, by the cells' `link`.
    pub links: Vec<gpui_kit::SharedString>,
}

impl TerminalSnapshot {
    pub fn visible_text(&self) -> String {
        self.cells
            .chunks(self.columns)
            .map(|row| {
                row.iter()
                    .filter(|cell| !cell.flags.contains(Flags::WIDE_CHAR_SPACER))
                    .map(|cell| cell.character.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub struct TerminalEngine {
    factory: SharedTerminalTransportFactory,
    runtime: TerminalRuntime,
    lifecycle: TerminalLifecycle,
    title: Option<String>,
    /// The connection's latest round trip, while one is running.
    latency: Option<Latency>,
    /// The open find, if any. Rendering reads it through `&self` and still
    /// has to advance the search engine's cache, hence the `RefCell`.
    search: RefCell<Option<TerminalSearch>>,
    /// Finds the web addresses on screen, built once.
    url_search: RefCell<RegexSearch>,
    /// The theme, shared with the parser thread, which answers the colors
    /// programs ask for from it.
    theme: SharedTheme,
    _theme_observer: gpui_kit::Subscription,
    /// The folder the shell last reported on this connection.
    directory: Option<ReportedDirectory>,
    reconnect_on_enter: bool,
    generation: u64,
    size: TerminalSize,
    event_sender: mpsc::Sender<TerminalUiEvent>,
    event_task: Option<gpui_kit::Task<()>>,
}

impl TerminalEngine {
    pub fn new(factory: SharedTerminalTransportFactory, cx: &mut gpui_kit::Context<Self>) -> Self {
        let size = TerminalSize::DEFAULT;
        let generation = 1;
        // A timer drains this standard channel on the UI thread. Parser
        // threads therefore never wake GPUI's local executor directly, and a
        // burst of terminal bytes results in a single render notification.
        let (event_sender, event_receiver) = mpsc::channel();
        let theme = Arc::new(Mutex::new(TerminalColors::current(cx)));
        let runtime = TerminalRuntime::start(
            generation,
            size,
            factory.create(),
            event_sender.clone(),
            theme.clone(),
        );
        let theme_observer = cx.observe_global::<TerminalColors>(|this, cx| {
            *this.theme.lock().unwrap_or_else(|error| error.into_inner()) =
                TerminalColors::current(cx);
        });
        let mut this = Self {
            factory,
            runtime,
            lifecycle: TerminalLifecycle::Starting,
            title: None,
            latency: None,
            search: RefCell::new(None),
            url_search: RefCell::new(url_search()),
            theme,
            _theme_observer: theme_observer,
            directory: None,
            reconnect_on_enter: false,
            generation,
            size,
            event_sender,
            event_task: None,
        };
        this.event_task = Some(Self::listen(event_receiver, cx));
        this
    }

    fn listen(
        events: mpsc::Receiver<TerminalUiEvent>,
        cx: &mut gpui_kit::Context<Self>,
    ) -> gpui_kit::Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let batch: Vec<_> = events.try_iter().collect();
                if !batch.is_empty()
                    && this
                        .update(cx, |this, cx| {
                            for event in batch {
                                if let Some(event) = this.handle_event(event, cx) {
                                    cx.emit(event);
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                {
                    break;
                }
            }
        })
    }

    /// Fold one transport event into the engine's state. Anything the view
    /// must hear about comes back as an event to emit.
    fn handle_event(
        &mut self,
        event: TerminalUiEvent,
        cx: &mut gpui_kit::App,
    ) -> Option<TerminalEvent> {
        if event.generation != self.generation {
            return None;
        }

        match event.kind {
            TerminalUiEventKind::Started => {
                self.lifecycle = TerminalLifecycle::Running;
                return Some(TerminalEvent::Started);
            }
            TerminalUiEventKind::Wakeup => {
                self.runtime.wakeup_pending.store(false, Ordering::Release);
            }
            TerminalUiEventKind::Title(title) => self.title = sanitize_title(&title),
            TerminalUiEventKind::ResetTitle => self.title = None,
            TerminalUiEventKind::Exited { code, signal } => {
                self.append_reconnect_hint();
                self.lifecycle = TerminalLifecycle::Exited { code, signal };
                self.latency = None;
                self.directory = None;
            }
            TerminalUiEventKind::Failed(error) => {
                self.append_reconnect_hint();
                self.lifecycle = TerminalLifecycle::Failed(error);
                self.latency = None;
                self.directory = None;
            }
            // Each prompt reports the folder again: only a change is news.
            TerminalUiEventKind::Directory(directory) => {
                if self.directory.as_ref() != Some(&directory) {
                    self.directory = Some(directory.clone());
                    return Some(TerminalEvent::DirectoryChanged(directory));
                }
            }
            TerminalUiEventKind::Latency(latency) => self.latency = Some(latency),
            TerminalUiEventKind::Prompt(prompt) => {
                return Some(TerminalEvent::PromptRequested(prompt));
            }
            TerminalUiEventKind::ClipboardStore(text) => {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
            }
            TerminalUiEventKind::Notice(notice) => {
                return Some(TerminalEvent::Notice(notice.cleaned()));
            }
            TerminalUiEventKind::Bell => {
                let line = cursor_line_text(&self.runtime.term.lock());
                return Some(TerminalEvent::Notice(TerminalNotice::bell(&line)));
            }
        }
        None
    }

    pub(super) fn enable_reconnect_on_enter(&mut self) {
        self.reconnect_on_enter = true;
    }

    fn append_reconnect_hint(&self) {
        if self.reconnect_on_enter {
            append_message(&self.runtime.term, "按 Enter 重新连接");
        }
    }

    pub fn restart(&mut self, cx: &mut gpui_kit::Context<Self>) {
        self.generation = self.generation.saturating_add(1);
        self.runtime.shutdown();
        self.lifecycle = TerminalLifecycle::Starting;
        self.title = None;
        self.latency = None;
        self.directory = None;
        // The new emulator starts empty; matches in the old one mean nothing.
        self.search.replace(None);
        self.runtime = TerminalRuntime::start(
            self.generation,
            self.size,
            self.factory.create(),
            self.event_sender.clone(),
            self.theme.clone(),
        );
        cx.notify();
    }

    pub fn restart_with_factory(
        &mut self,
        factory: SharedTerminalTransportFactory,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        self.factory = factory;
        self.restart(cx);
    }

    pub fn reply_to_prompt(&self, request_id: u64, reply: ConnectionPromptReply) {
        self.runtime.reply_to_prompt(request_id, reply);
    }

    pub fn shutdown(&mut self, cx: &mut gpui_kit::Context<Self>) {
        self.generation = self.generation.saturating_add(1);
        self.lifecycle = TerminalLifecycle::Closing;
        self.runtime.shutdown();
        self.directory = None;
        cx.notify();
    }

    pub fn stop(&mut self, message: &str, cx: &mut gpui_kit::Context<Self>) {
        self.generation = self.generation.saturating_add(1);
        self.runtime.shutdown();
        self.directory = None;
        append_message(&self.runtime.term, message);
        self.append_reconnect_hint();
        self.lifecycle = TerminalLifecycle::Exited {
            code: 0,
            signal: None,
        };
        cx.notify();
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        if self.lifecycle.accepts_input() {
            self.runtime.write(bytes.into());
        }
    }

    /// Send bytes originating from the user. Unlike protocol writes such as
    /// terminal query responses and focus reports, interactive input returns
    /// to the live screen and retires the previous selection first.
    pub fn send_user_input(&self, bytes: impl Into<Vec<u8>>) {
        if !self.lifecycle.accepts_input() {
            return;
        }

        let mut term = self.runtime.term.lock();
        prepare_term_for_user_input(&mut term);
        drop(term);
        self.runtime.write(bytes.into());
    }

    pub fn prepare_for_user_input(&self) {
        if !self.lifecycle.accepts_input() {
            return;
        }

        let mut term = self.runtime.term.lock();
        prepare_term_for_user_input(&mut term);
    }

    pub fn paste(&self, text: &str) {
        self.send_user_input(encode_paste(text, self.mode()));
    }

    /// Put `command` on the shell's input line, as a paste, and with `run`
    /// press Enter: what a click in 历史命令 and 命令片段 does. With
    /// `replace`, in place of what is typed there. Says why not while the
    /// shell is not running or a full-screen program has the keys.
    pub fn enter_command(
        &self,
        command: &str,
        run: bool,
        replace: bool,
    ) -> Result<(), SharedString> {
        if !self.lifecycle.accepts_input() {
            return Err("终端没有连接".into());
        }
        self.send_user_input(encode_command(command, run, replace, self.mode())?);
        Ok(())
    }

    pub fn resize(&mut self, size: TerminalSize, cx: &mut gpui_kit::Context<Self>) {
        if size == self.size {
            return;
        }
        self.size = size;
        *self
            .runtime
            .size
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = size;
        self.runtime.term.lock().resize(size);
        self.runtime.resize(size);
        cx.notify();
    }

    pub fn scroll(&mut self, lines: i32, cx: &mut gpui_kit::Context<Self>) {
        self.runtime
            .term
            .lock()
            .scroll_display(Scroll::Delta(lines));
        cx.notify();
    }

    /// Scrolls the view without a context, for the scrollbar's drag.
    pub fn scrollback(&self) -> Scrollback {
        Scrollback(self.runtime.term.clone())
    }

    pub fn start_selection(
        &mut self,
        point: Point,
        side: Side,
        selection_type: SelectionType,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        self.runtime.term.lock().selection = Some(Selection::new(selection_type, point, side));
        cx.notify();
    }

    pub fn update_selection(&mut self, point: Point, side: Side, cx: &mut gpui_kit::Context<Self>) {
        if let Some(selection) = self.runtime.term.lock().selection.as_mut() {
            selection.update(point, side);
        }
        cx.notify();
    }

    /// Scroll the view `lines` (into the history when positive), then
    /// move the selection's end to `column` of the view's row `row`: a drag
    /// held past the top or the bottom, or the wheel turned while dragging,
    /// takes in what comes into view.
    pub fn scroll_selection(
        &mut self,
        lines: i32,
        row: usize,
        column: Column,
        side: Side,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        let mut term = self.runtime.term.lock();
        if lines != 0 {
            term.scroll_display(Scroll::Delta(lines));
        }
        let row = row.min(term.screen_lines().saturating_sub(1)) as i32;
        let point = Point::new(Line(row - term.grid().display_offset() as i32), column);
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
        }
        drop(term);
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut gpui_kit::Context<Self>) {
        self.runtime.term.lock().selection = None;
        cx.notify();
    }

    pub fn select_all(&mut self, cx: &mut gpui_kit::Context<Self>) {
        select_all(&mut self.runtime.term.lock());
        cx.notify();
    }

    /// Open the find, or search the open one for `query`, focusing the newest
    /// match at or above the bottom of the viewport.
    pub fn set_search_query(&mut self, query: &str, cx: &mut gpui_kit::Context<Self>) {
        let mut term = self.runtime.term.lock();
        self.search
            .borrow_mut()
            .get_or_insert_with(TerminalSearch::new)
            .set_query(query, &mut term);
        drop(term);
        cx.notify();
    }

    /// Focus the next match up (older output) or down (newer output).
    pub fn step_search(&mut self, direction: SearchDirection, cx: &mut gpui_kit::Context<Self>) {
        let mut term = self.runtime.term.lock();
        if let Some(search) = self.search.borrow_mut().as_mut() {
            search.step(direction, &mut term);
        }
        drop(term);
        cx.notify();
    }

    pub fn clear_search(&mut self, cx: &mut gpui_kit::Context<Self>) {
        if self.search.replace(None).is_some() {
            cx.notify();
        }
    }

    /// Where the focused match is, while the find has a query.
    pub fn search_position(&self) -> Option<SearchPosition> {
        self.search
            .borrow()
            .as_ref()
            .and_then(|search| search.position())
    }

    /// Clear the screen and the scrollback, keeping only the line the cursor
    /// is on (with everything wrapped into it), at the top. This happens in
    /// the emulator alone; the shell is not told, and carries on writing from
    /// the cursor, which stays beside its prompt.
    pub fn clear_keeping_prompt(&mut self, cx: &mut gpui_kit::Context<Self>) {
        let mut term = self.runtime.term.lock();
        if !clear_keeping_cursor_line(&mut term) {
            return;
        }
        if let Some(search) = self.search.borrow_mut().as_mut() {
            search.refresh(&mut term);
        }
        drop(term);
        cx.notify();
    }

    pub fn selection_text(&self) -> Option<String> {
        self.runtime.term.lock().selection_to_string()
    }

    pub fn has_selection(&self) -> bool {
        self.selection_text().is_some_and(|text| !text.is_empty())
    }

    pub fn mode(&self) -> TermMode {
        *self.runtime.term.lock().mode()
    }

    pub fn set_focused(&self, focused: bool) {
        let mut term = self.runtime.term.lock();
        if term.is_focused == focused {
            return;
        }
        term.is_focused = focused;
        if term.mode().contains(TermMode::FOCUS_IN_OUT) {
            drop(term);
            self.runtime
                .write(if focused { b"\x1b[I" } else { b"\x1b[O" }.to_vec());
        }
    }

    pub fn lifecycle(&self) -> &TerminalLifecycle {
        &self.lifecycle
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn latency(&self) -> Option<Latency> {
        self.latency
    }

    /// Counts the transports started: every reconnect is a new connection,
    /// possibly to another machine.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The folder the shell last reported on this connection, if it has.
    pub fn directory(&self) -> Option<&ReportedDirectory> {
        self.directory.as_ref()
    }

    pub fn status(&self) -> TerminalStatus {
        TerminalStatus::new(
            self.lifecycle.clone(),
            self.size.columns(),
            self.size.rows(),
        )
    }

    /// Whether the cursor blinks: the program may say. Cheaper than a
    /// snapshot, which the blink timer would otherwise take twice a second.
    pub fn cursor_blinking(&self) -> bool {
        self.runtime.term.lock().cursor_style().blinking
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        let term = self.runtime.term.lock();
        let content = term.renderable_content();
        let selection = content.selection;
        let display_offset = content.display_offset as i32;
        let cursor_row = content.cursor.point.line.0 + display_offset;
        let cursor_point = content.cursor.point;
        let cursor_shape = content.cursor.shape;
        let columns = term.grid().columns();
        let (matches, focused) = self
            .search
            .borrow_mut()
            .as_mut()
            .map(|search| search.visible(&term))
            .unwrap_or_default();
        let mut marker = MatchMarker::new(&matches, focused.as_ref());
        let mut links = LinkMarker::new(visible_links(&term, &mut self.url_search.borrow_mut()));
        let mut cells: Vec<_> = content
            .display_iter
            .map(|indexed| {
                let mut character = indexed.cell.c.to_string();
                if let Some(zerowidth) = indexed.cell.zerowidth() {
                    character.extend(zerowidth);
                }
                TerminalCell {
                    character,
                    foreground: indexed.cell.fg,
                    background: indexed.cell.bg,
                    flags: indexed.cell.flags,
                    selected: selection.is_some_and(|range| {
                        range.contains_cell(&indexed, cursor_point, cursor_shape)
                    }),
                    search: marker.mark(indexed.point),
                    link: links.mark(indexed.point, indexed.cell.hyperlink()),
                }
            })
            .collect();
        normalize_wide_character_selection(&mut cells, columns);
        TerminalSnapshot {
            columns,
            rows: term.grid().screen_lines(),
            cells,
            cursor_row: (0..term.grid().screen_lines() as i32)
                .contains(&cursor_row)
                .then_some(cursor_row as usize),
            cursor_column: content.cursor.point.column.0,
            cursor_shape: content.cursor.shape,
            cursor_blinking: term.cursor_style().blinking,
            display_offset: content.display_offset,
            history_size: term.grid().history_size(),
            colors: std::array::from_fn(|index| content.colors[index]),
            links: links.into_links(),
        }
    }
}

/// A terminal's view of its history, for the scrollbar, whose drag hands
/// over a position and no context: whoever draws the terminal is told to
/// redraw it. Taken for each frame, as a restart replaces the terminal.
#[derive(Clone)]
pub struct Scrollback(Arc<FairMutex<AlacrittyTerm>>);

impl Scrollback {
    /// Show the history from `lines` below its oldest line.
    pub fn scroll_to(&self, lines: usize) {
        scroll_to(&mut self.0.lock(), lines);
    }
}

/// Lines are counted from the top, which stays put while output arrives
/// below: the view stays where the pointer let it go.
fn scroll_to(term: &mut AlacrittyTerm, lines: usize) {
    let grid = term.grid();
    let offset = grid.history_size().saturating_sub(lines);
    let delta = offset as i32 - grid.display_offset() as i32;
    term.scroll_display(Scroll::Delta(delta));
}

/// The text of the line the cursor is on, or of the nearest line above it
/// on screen that has any: where a program that rings for an answer has
/// its question.
fn cursor_line_text(term: &AlacrittyTerm) -> String {
    let mut line = term.grid().cursor.point.line;
    loop {
        let text: String = term.grid()[line]
            .into_iter()
            .filter(|cell| !cell.flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(|cell| cell.c)
            .collect();
        let text = text.trim();
        if !text.is_empty() || line.0 <= 0 {
            return text.to_string();
        }
        line = Line(line.0 - 1);
    }
}

/// Keep the leading cell and spacer of a full-width glyph visually atomic,
/// for the selection and for find highlights alike.
/// Alacritty stores CJK glyphs in one `WIDE_CHAR` cell followed by a
/// `WIDE_CHAR_SPACER`; painting only one half produces a one-column highlight
/// that cannot line up with the two-column glyph.
fn normalize_wide_character_selection(cells: &mut [TerminalCell], columns: usize) {
    if columns == 0 {
        return;
    }

    for row in cells.chunks_mut(columns) {
        for column in 0..row.len().saturating_sub(1) {
            if row[column].flags.contains(Flags::WIDE_CHAR)
                && row[column + 1].flags.contains(Flags::WIDE_CHAR_SPACER)
            {
                let selected = row[column].selected || row[column + 1].selected;
                row[column].selected = selected;
                row[column + 1].selected = selected;
                let search = row[column].search.max(row[column + 1].search);
                row[column].search = search;
                row[column + 1].search = search;
            }
        }
    }
}

impl gpui_kit::EventEmitter<TerminalEvent> for TerminalEngine {}

impl Drop for TerminalEngine {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}

#[derive(Clone, Debug)]
pub enum TerminalEvent {
    /// The shell is there: connected, for a remote terminal, again after
    /// each reconnection.
    Started,
    PromptRequested(ConnectionPrompt),
    /// Something to show the user while they look elsewhere.
    Notice(TerminalNotice),
    /// The shell reported another folder than the last one.
    DirectoryChanged(ReportedDirectory),
}

struct TerminalRuntime {
    term: Arc<FairMutex<AlacrittyTerm>>,
    commands: mpsc::Sender<TerminalTransportCommand>,
    size: Arc<Mutex<TerminalSize>>,
    wakeup_pending: Arc<AtomicBool>,
}

impl TerminalRuntime {
    fn start(
        generation: u64,
        size: TerminalSize,
        transport: Box<dyn super::TerminalTransport>,
        ui_events: mpsc::Sender<TerminalUiEvent>,
        theme: SharedTheme,
    ) -> Self {
        let (commands, command_receiver) = mpsc::channel();
        let (transport_events, transport_receiver) = async_channel::unbounded();
        let shared_size = Arc::new(Mutex::new(size));
        let wakeup_pending = Arc::new(AtomicBool::new(false));
        let proxy = TerminalEventProxy {
            generation,
            commands: commands.clone(),
            ui_events: ui_events.clone(),
            size: shared_size.clone(),
            wakeup_pending: wakeup_pending.clone(),
            replies: Arc::default(),
            theme,
        };
        let parser_proxy = proxy.clone();
        let config = terminal_config();
        let term = Arc::new(FairMutex::new(AlacrittyTerm::new(config, &size, proxy)));

        let mut parser = Parser {
            term: term.clone(),
            processor: Processor::new(),
            notices: NoticeScanner::default(),
            proxy: parser_proxy,
        };
        thread::Builder::new()
            .name("qishell-terminal-parser".into())
            .spawn(move || {
                loop {
                    // Output inside a synchronized update is held back until
                    // the program ends it. Alacritty's own event loop draws
                    // it anyway once the update's time is up, so a program
                    // that never ends it (killed mid-frame, or its connection
                    // lost) does not freeze the screen; this loop has to too.
                    let deadline = parser.processor.sync_timeout().sync_timeout();
                    let event = match receive(&transport_receiver, deadline) {
                        Received::Event(event) => event,
                        Received::Closed => break,
                        Received::Overdue => {
                            let mut term = parser.term.lock();
                            if end_overdue_sync(&mut parser.processor, &mut term) {
                                parser.proxy.answer(&term);
                            }
                            drop(term);
                            parser.proxy.wakeup();
                            continue;
                        }
                    };
                    match event {
                        TerminalTransportEvent::Started => {
                            parser.proxy.send(TerminalUiEventKind::Started);
                        }
                        TerminalTransportEvent::Output(bytes) => {
                            parser.output(&bytes);
                            parser.proxy.wakeup();
                        }
                        TerminalTransportEvent::Latency(latency) => {
                            parser.proxy.send(TerminalUiEventKind::Latency(latency));
                        }
                        TerminalTransportEvent::Prompt(prompt) => {
                            parser.proxy.send(TerminalUiEventKind::Prompt(prompt));
                        }
                        TerminalTransportEvent::Exited { code, signal } => {
                            let description = signal
                                .as_deref()
                                .map(|signal| {
                                    format!(
                                        "进程已退出：{signal}，退出码 {code}",
                                        signal = signal,
                                        code = code
                                    )
                                })
                                .unwrap_or_else(|| {
                                    format!("进程已退出，退出码 {code}", code = code)
                                });
                            append_message_with_processor(
                                &parser.term,
                                &mut parser.processor,
                                &description,
                            );
                            parser
                                .proxy
                                .send(TerminalUiEventKind::Exited { code, signal });
                            parser.proxy.wakeup();
                            break;
                        }
                        TerminalTransportEvent::Failed(error) => {
                            append_message_with_processor(
                                &parser.term,
                                &mut parser.processor,
                                &format!("终端错误：{error}", error = error),
                            );
                            parser.proxy.send(TerminalUiEventKind::Failed(error));
                            parser.proxy.wakeup();
                        }
                    }
                }
            })
            .expect("terminal parser thread");

        thread::Builder::new()
            .name("qishell-terminal-transport".into())
            .spawn(move || {
                if let Err(error) = transport.run(size, command_receiver, transport_events.clone())
                {
                    let _ = transport_events
                        .send_blocking(TerminalTransportEvent::Failed(error.to_string()));
                }
            })
            .expect("terminal transport thread");

        Self {
            term,
            commands,
            size: shared_size,
            wakeup_pending,
        }
    }

    fn write(&self, bytes: Vec<u8>) {
        let _ = self.commands.send(TerminalTransportCommand::Write(bytes));
    }

    fn resize(&self, size: TerminalSize) {
        let _ = self.commands.send(TerminalTransportCommand::Resize(size));
    }

    fn shutdown(&self) {
        let _ = self.commands.send(TerminalTransportCommand::Shutdown);
    }

    fn reply_to_prompt(&self, request_id: u64, reply: ConnectionPromptReply) {
        let _ = self
            .commands
            .send(TerminalTransportCommand::PromptReply { request_id, reply });
    }
}

fn terminal_config() -> Config {
    Config {
        scrolling_history: 10_000,
        default_cursor_style: CursorStyle {
            shape: CursorShape::Block,
            blinking: true,
        },
        // Programs may copy to the clipboard (vim, tmux), never read it.
        osc52: Osc52::OnlyCopy,
        kitty_keyboard: false,
        ..Config::default()
    }
}

impl Drop for TerminalRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone)]
pub struct TerminalEventProxy {
    generation: u64,
    commands: mpsc::Sender<TerminalTransportCommand>,
    ui_events: mpsc::Sender<TerminalUiEvent>,
    size: Arc<Mutex<TerminalSize>>,
    wakeup_pending: Arc<AtomicBool>,
    /// What the output just parsed asks of the terminal, in the order it
    /// asks, until `answer`.
    replies: Arc<Mutex<Vec<Reply>>>,
    theme: SharedTheme,
}

/// The theme a terminal's parser thread answers color queries from,
/// swapped by the engine when `TerminalColors` changes.
type SharedTheme = Arc<Mutex<&'static TerminalTheme>>;

/// An answer to a query in the output.
enum Reply {
    Bytes(Vec<u8>),
    /// OSC 4, 10, 11 or 12. The answer needs the colors the program set,
    /// which only the one holding the terminal can read.
    Color(usize, Arc<dyn Fn(Rgb) -> String + Send + Sync>),
}

impl EventListener for TerminalEventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(title) => self.send(TerminalUiEventKind::Title(title)),
            Event::ResetTitle => self.send(TerminalUiEventKind::ResetTitle),
            Event::ColorRequest(index, formatter) => self.reply(Reply::Color(index, formatter)),
            Event::PtyWrite(text) => self.reply(Reply::Bytes(text.into_bytes())),
            Event::TextAreaSizeRequest(formatter) => {
                let size = *self.size.lock().unwrap_or_else(|error| error.into_inner());
                let response = formatter(WindowSize {
                    num_lines: size.rows().min(u16::MAX as usize) as u16,
                    num_cols: size.columns().min(u16::MAX as usize) as u16,
                    cell_width: size.cell_width(),
                    cell_height: size.cell_height(),
                });
                self.reply(Reply::Bytes(response.into_bytes()));
            }
            Event::Bell => {
                self.send(TerminalUiEventKind::Bell);
                self.wakeup();
            }
            Event::Wakeup
            | Event::MouseCursorDirty
            | Event::CursorBlinkingChange
            | Event::Exit
            | Event::ChildExit(_) => self.wakeup(),
            // Only the clipboard: a program setting the primary selection
            // on every selection must not overwrite it.
            Event::ClipboardStore(ClipboardType::Clipboard, text) => {
                self.send(TerminalUiEventKind::ClipboardStore(text));
            }
            Event::ClipboardStore(_, _) | Event::ClipboardLoad(_, _) => {}
        }
    }
}

impl TerminalEventProxy {
    fn reply(&self, reply: Reply) {
        self.replies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(reply);
    }

    /// Send the answers to what the output parsed into `term` asked, at
    /// once and in the order asked. Programs read them in that order:
    /// termenv (ollama, Charm's tools) asks for the background by OSC 11
    /// and then for the cursor, which every terminal answers, and takes a
    /// cursor report coming first to mean no OSC 11 answer is coming. The
    /// shell would then read that answer as typed text.
    fn answer(&self, term: &AlacrittyTerm) {
        let replies = std::mem::take(
            &mut *self
                .replies
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        if replies.is_empty() {
            return;
        }
        let theme = *self.theme.lock().unwrap_or_else(|error| error.into_inner());
        let mut bytes = Vec::new();
        for reply in replies {
            match reply {
                Reply::Bytes(reply) => bytes.extend(reply),
                Reply::Color(index, formatter) => {
                    // A color the program set itself, or else the theme's.
                    let color = term.colors()[index].unwrap_or_else(|| theme.query_color(index));
                    bytes.extend(formatter(color).into_bytes());
                }
            }
        }
        let _ = self.commands.send(TerminalTransportCommand::Write(bytes));
    }

    /// Queue an event for the UI thread, stamped with this run's generation.
    fn send(&self, kind: TerminalUiEventKind) {
        let _ = self.ui_events.send(TerminalUiEvent {
            generation: self.generation,
            kind,
        });
    }

    fn wakeup(&self) {
        if !self.wakeup_pending.swap(true, Ordering::AcqRel) {
            self.send(TerminalUiEventKind::Wakeup);
        }
    }
}

struct TerminalUiEvent {
    generation: u64,
    kind: TerminalUiEventKind,
}

enum TerminalUiEventKind {
    Started,
    Wakeup,
    Title(String),
    ResetTitle,
    Exited {
        code: u32,
        signal: Option<String>,
    },
    Failed(String),
    Prompt(ConnectionPrompt),
    Latency(Latency),
    /// A program's copy, by OSC 52.
    ClipboardStore(String),
    /// A notification a program asked for, by OSC 9 or 777.
    Notice(ProgramNotice),
    /// The folder the shell reported, by OSC 7 or 1337.
    Directory(ReportedDirectory),
    Bell,
}

fn append_message(term: &Arc<FairMutex<AlacrittyTerm>>, message: &str) {
    let mut processor = Processor::new();
    append_message_with_processor(term, &mut processor, message);
}

fn append_message_with_processor(
    term: &Arc<FairMutex<AlacrittyTerm>>,
    processor: &mut Processor,
    message: &str,
) {
    let line = format!("\r\n\x1b[2m[{message}]\x1b[0m\r\n");
    let mut term = term.lock();
    // A program that ended mid-frame left its synchronized update open:
    // neither that frame nor this message would show.
    if processor.sync_timeout().pending_timeout() {
        processor.stop_sync(&mut *term);
    }
    processor.advance(&mut *term, line.as_bytes());
}

/// What waiting for the transport came to.
enum Received<T> {
    Event(T),
    /// The deadline is up before anything came: an open synchronized
    /// update's.
    Overdue,
    /// The transport is gone.
    Closed,
}

/// The transport's next event; with a `deadline`, waiting only until then.
fn receive<T>(events: &async_channel::Receiver<T>, deadline: Option<Instant>) -> Received<T> {
    async_io::block_on(async {
        let next = std::pin::pin!(events.recv());
        let received = match deadline {
            None => next.await,
            // What has come already wins over a deadline that has passed.
            Some(deadline) => match select(next, async_io::Timer::at(deadline)).await {
                Either::Left((received, _)) => received,
                Either::Right(_) => return Received::Overdue,
            },
        };
        match received {
            Ok(event) => Received::Event(event),
            Err(_) => Received::Closed,
        }
    })
}

/// What the parser thread keeps: the emulator it feeds, and what reads
/// the output beside it.
struct Parser {
    term: Arc<FairMutex<AlacrittyTerm>>,
    processor: Processor,
    /// Alacritty drops the notifications programs ask for.
    notices: NoticeScanner,
    proxy: TerminalEventProxy,
}

impl Parser {
    /// Output for the emulator, and for what reads it beside: the programs'
    /// notifications and folders.
    fn output(&mut self, bytes: &[u8]) {
        {
            let mut term = self.term.lock();
            // Output that never pauses gives the wait no chance to see the
            // time is up.
            end_overdue_sync(&mut self.processor, &mut term);
            self.processor.advance(&mut *term, bytes);
            self.proxy.answer(&term);
        }
        for notice in self.notices.scan(bytes) {
            self.proxy.send(TerminalUiEventKind::Notice(notice));
        }
        if let Some(directory) = self.notices.take_directory() {
            self.proxy.send(TerminalUiEventKind::Directory(directory));
        }
    }
}

/// Draw what an open synchronized update has held back, if its time is up.
/// Says whether it was.
fn end_overdue_sync(processor: &mut Processor, term: &mut AlacrittyTerm) -> bool {
    let overdue = processor
        .sync_timeout()
        .sync_timeout()
        .is_some_and(|deadline| deadline <= Instant::now());
    if overdue {
        processor.stop_sync(term);
    }
    overdue
}

/// Select the scrollback and the screen, from the first line with text to
/// the last; the blank lines around them would only copy as empty lines.
/// With no text at all, nothing.
fn select_all(term: &mut AlacrittyTerm) {
    let lines = term.topmost_line().0..=term.bottommost_line().0;
    let has_text = |line: &i32| !term.grid()[Line(*line)].is_clear();
    let first = lines.clone().find(has_text);
    let last = lines.rev().find(has_text);
    term.selection = first.zip(last).map(|(first, last)| {
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(Line(first), Column(0)),
            Side::Left,
        );
        selection.update(Point::new(Line(last), term.last_column()), Side::Right);
        selection
    });
}

fn sanitize_title(title: &str) -> Option<String> {
    let title: String = title
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect();
    (!title.trim().is_empty()).then_some(title)
}

fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{}\x1b[201~", normalized.replace('\x1b', "")).into_bytes()
    } else {
        normalized.into_bytes()
    }
}

/// What `TerminalEngine::enter_command` types: with `replace` Ctrl-E and
/// Ctrl-U first, to the line's end and everything before it away (the
/// emacs keys of bash, zsh and fish alike), then the command as a paste,
/// then Enter to run it.
///
/// A full-screen program would take those keys for its own. A command of
/// several lines can only be put on the line in a bracketed paste: without
/// one, its first newline would run it.
fn encode_command(
    command: &str,
    run: bool,
    replace: bool,
    mode: TermMode,
) -> Result<Vec<u8>, SharedString> {
    if mode.contains(TermMode::ALT_SCREEN) {
        return Err("终端里正在运行全屏程序，先退出它".into());
    }
    if !run && command.contains('\n') && !mode.contains(TermMode::BRACKETED_PASTE) {
        return Err("这是一条多行命令，在这个终端里只能直接执行".into());
    }
    let mut bytes = if replace {
        b"\x05\x15".to_vec()
    } else {
        Vec::new()
    };
    bytes.extend(encode_paste(command, mode));
    if run {
        bytes.push(b'\r');
    }
    Ok(bytes)
}

/// See `TerminalEngine::clear_keeping_prompt`. Returns whether anything was
/// cleared.
fn clear_keeping_cursor_line(term: &mut AlacrittyTerm) -> bool {
    if term.mode().contains(TermMode::ALT_SCREEN) {
        return false;
    }
    term.scroll_display(Scroll::Bottom);
    term.selection = None;
    let cursor = term.grid().cursor.point;
    // The prompt may have wrapped onto the cursor's row; keep it whole. A part
    // already pushed into the scrollback goes with the scrollback.
    let start = term.line_search_left(cursor).line.max(Line(0));
    let lines = start.0 as usize;
    if lines > 0 {
        let screen = Line(0)..Line(term.screen_lines() as i32);
        let grid = term.grid_mut();
        grid.scroll_up(&screen, lines);
        grid.cursor.point.line -= lines;
    }
    term.grid_mut().clear_history();
    true
}

fn prepare_term_for_user_input(term: &mut AlacrittyTerm) {
    term.scroll_display(Scroll::Bottom);
    term.selection = None;
}
