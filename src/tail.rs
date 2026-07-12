use crate::db::Db;
use crate::settings::TailTheme;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

// Polling cadence and bounds for the follow loop. The loop is deliberately
// bounded (no unbounded `tail -f`): it stops draining on the run's terminal
// `result` event, and otherwise gives up after IDLE_TIMEOUT of no new bytes so
// an interrupted/rate-limited run (which never emits `result`) can't follow
// forever. The idle clock resets whenever new bytes arrive, so an actively
// streaming run keeps going. Once draining stops the UI stays up until `q`.
const POLL_INTERVAL: Duration = Duration::from_millis(400);
const IDLE_TIMEOUT: Duration = Duration::from_secs(360); // ~6 min with no new bytes
const MAX_LINES_PER_POLL: u32 = 100_000;
const MAX_DIR_ENTRIES: u32 = 100_000;

/// Follow the most recent run log for `issue` in a full-screen TUI: the
/// subagent's text replies word-wrap under a `›` bullet, its tool calls appear
/// as truncated `◦` one-liners, and the final report closes the stream.
/// `log_dir` is where the orchestration helper tees the transcripts.
pub fn run(db: &Db, issue: i64, log_dir: &Path, theme: &TailTheme) -> io::Result<()> {
    let Some(path) = newest_log(log_dir, issue)? else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no logs for issue #{issue} in {}", log_dir.display()),
        ));
    };
    let file = File::open(&path)?;
    let mut app = App::new(header(db, issue), file, *theme);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    res
}

/// The newest-by-mtime `issue-<id>-*.jsonl` in `dir`, mirroring the shell
/// helper's `ls -t .../issue-<id>-*.jsonl | head -1`. The `issue-<id>-` prefix
/// is matched exactly so `issue-1-` never matches `issue-12-`.
fn newest_log(dir: &Path, issue: i64) -> io::Result<Option<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no logs directory at {}", dir.display()),
            ));
        }
        Err(e) => return Err(e),
    };

    let prefix = format!("issue-{issue}-");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.take(MAX_DIR_ENTRIES as usize) {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&prefix) || !name.ends_with(".jsonl") {
            continue;
        }
        let mtime = entry.metadata()?.modified()?;
        if best.as_ref().is_none_or(|(t, _)| mtime > *t) {
            best = Some((mtime, entry.path()));
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// `pit · #<id> · <title>` — best-effort; if the issue isn't in the DB (the log
/// can outlive its row) the title is omitted rather than erroring.
fn header(db: &Db, issue: i64) -> String {
    match db.get_issue(&json!({ "id": issue })) {
        Ok(v) => match v.get("title").and_then(Value::as_str) {
            Some(title) => format!("pit · #{issue} · {title}"),
            None => format!("pit · #{issue}"),
        },
        Err(_) => format!("pit · #{issue}"),
    }
}

/// One streamed event worth showing. `Msg` is assistant prose (word-wrapped),
/// `Tool` is a `Name: args` tool call (truncated to one row), `Report` is the
/// terminal result text.
enum Entry {
    Msg(String),
    Tool(String),
    Report(String),
}

struct App {
    header: String,
    entries: Vec<Entry>,
    reader: BufReader<File>,
    // Doubles as the read target and the partial-line accumulator: a file being
    // actively tee'd can leave the last line without its trailing `\n`, so we
    // only parse (and clear) once a line is newline-terminated.
    buf: String,
    scroll: u16,
    // Follow-the-tail toggle: while `stick` the viewport is pinned to the bottom
    // as new entries arrive; any upward scroll releases it, and scrolling back to
    // the bottom re-engages it.
    stick: bool,
    view_h: u16,
    done: bool,
    status: Option<String>,
    last_data: Instant,
    theme: TailTheme,
}

impl App {
    fn new(header: String, file: File, theme: TailTheme) -> Self {
        Self {
            header,
            entries: Vec::new(),
            reader: BufReader::new(file),
            buf: String::new(),
            scroll: 0,
            stick: true,
            view_h: 0,
            done: false,
            status: None,
            last_data: Instant::now(),
            theme,
        }
    }

    /// Drain the currently-available complete lines into `entries`. A partial
    /// (non-newline-terminated) trailing line is left in `buf` for the next poll.
    /// Stops draining once the terminal `result` event or the idle deadline is
    /// reached; either way the UI stays up until the user quits.
    fn poll_file(&mut self) {
        if self.done {
            return;
        }
        let mut got = 0u32;
        for _ in 0..MAX_LINES_PER_POLL {
            match self.reader.read_line(&mut self.buf) {
                Ok(0) => break, // EOF for now — more may be appended before the next poll.
                Ok(_) => {}
                Err(e) => {
                    self.status = Some(format!("read error: {e}"));
                    self.done = true;
                    return;
                }
            }
            if !self.buf.ends_with('\n') {
                break; // Partial line still being written; keep it in `buf`.
            }
            let saw_result = ingest(self.buf.trim_end(), &mut self.entries);
            self.buf.clear();
            got += 1;
            if saw_result {
                self.done = true;
                break;
            }
        }
        if got > 0 {
            self.last_data = Instant::now();
        } else if self.last_data.elapsed() >= IDLE_TIMEOUT {
            self.status = Some("no result event (run interrupted or idle)".into());
            self.done = true;
        }
    }
}

/// Parse one stream-json event line into zero or more `entries`, returning
/// `true` for the terminal `result` event. Assistant `text` blocks become
/// `Msg`, `tool_use` blocks become `Tool`; `thinking` and other block/event
/// types are skipped. Malformed lines are ignored.
fn ingest(line: &str, entries: &mut Vec<Entry>) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    match v.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            let blocks = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array);
            for block in blocks.into_iter().flatten() {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                        if !text.trim().is_empty() {
                            entries.push(Entry::Msg(text.trim_end().to_string()));
                        }
                    }
                    Some("tool_use") => {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                        entries.push(Entry::Tool(format!(
                            "{name}{}",
                            tool_summary(block.get("input"))
                        )));
                    }
                    _ => {}
                }
            }
            false
        }
        Some("result") => {
            let report = v.get("result").and_then(Value::as_str).unwrap_or("");
            if !report.trim().is_empty() {
                entries.push(Entry::Report(report.trim().to_string()));
            }
            true
        }
        _ => false,
    }
}

/// A compact `: <arg>` suffix for a `tool_use` block, picking the most
/// descriptive string field from its `input`. Well-known argument names are
/// tried in priority order so the summary works for any tool without a per-tool
/// table; unknown tools with none of these keys get an empty suffix (just the
/// bare tool name). The value is flattened to a single line; the display-width
/// truncation happens at render time.
fn tool_summary(input: Option<&Value>) -> String {
    const KEYS: &[&str] = &[
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "prompt",
        "description",
    ];
    let Some(obj) = input.and_then(Value::as_object) else {
        return String::new();
    };
    for key in KEYS {
        if let Some(s) = obj.get(*key).and_then(Value::as_str) {
            let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
            if !one_line.is_empty() {
                return format!(": {one_line}");
            }
        }
    }
    String::new()
}

fn event_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        app.poll_file();
        terminal.draw(|f| render(f, app))?;

        // Once following has stopped there is nothing new to drain, so we only
        // wake often enough to stay responsive to keys.
        let timeout = if app.done {
            Duration::from_millis(500)
        } else {
            POLL_INTERVAL
        };
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if handle_key(app, key) => return Ok(()),
                Event::Mouse(m) => handle_mouse(app, m),
                _ => {}
            }
        }
    }
}

/// Dispatch a key press. Returns `true` when the app should quit. Non-press key
/// events (repeat/release) are ignored.
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.kind != KeyEventKind::Press {
        return false;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return true;
    }
    let page = app.view_h.max(1);
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        // Downward scrolls leave `stick` alone — render re-engages it if the move
        // lands at the bottom; upward scrolls always release it.
        KeyCode::Down | KeyCode::Char('j') => app.scroll = app.scroll.saturating_add(1),
        KeyCode::Up | KeyCode::Char('k') => {
            app.scroll = app.scroll.saturating_sub(1);
            app.stick = false;
        }
        KeyCode::PageDown | KeyCode::Char('J') => app.scroll = app.scroll.saturating_add(page),
        KeyCode::PageUp | KeyCode::Char('K') => {
            app.scroll = app.scroll.saturating_sub(page);
            app.stick = false;
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.scroll = 0;
            app.stick = false;
        }
        KeyCode::End | KeyCode::Char('G') => app.stick = true,
        _ => {}
    }
    false
}

fn handle_mouse(app: &mut App, m: MouseEvent) {
    match m.kind {
        MouseEventKind::ScrollDown => app.scroll = app.scroll.saturating_add(3),
        MouseEventKind::ScrollUp => {
            app.scroll = app.scroll.saturating_sub(3);
            app.stick = false;
        }
        _ => {}
    }
}

fn render(f: &mut Frame, app: &mut App) {
    let theme = app.theme; // Copy — avoids borrow conflicts with the mutations below.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    // Header — the whole line carries the header accent.
    let head = truncate(&app.header, chunks[0].width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            head,
            Style::default().fg(theme.header).add_modifier(Modifier::BOLD),
        ))),
        chunks[0],
    );

    // Body — every entry as a themed line, word-wrapped, scrolled.
    let body = chunks[1];
    app.view_h = body.height;
    let lines = build_lines(&app.entries, body.width, &theme);
    let total = wrapped_height(&lines, body.width);
    let max_scroll = total.saturating_sub(body.height);
    if app.stick {
        app.scroll = max_scroll;
    } else {
        if app.scroll > max_scroll {
            app.scroll = max_scroll;
        }
        if app.scroll >= max_scroll {
            app.stick = true; // Scrolled back to the bottom — resume following.
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        body,
    );

    render_footer(f, chunks[2], app);
}

/// Flatten the entry list into styled, prefix-marked lines sized to `width`.
/// Messages keep their hard line breaks (each marked `›`) and word-wrap at
/// render; tool calls are truncated to a single `◦` row so they never wrap.
fn build_lines(entries: &[Entry], width: u16, theme: &TailTheme) -> Vec<Line<'static>> {
    let message = Style::default().fg(theme.message);
    let tool = Style::default().fg(theme.tool);
    let tool_budget = (width as usize).saturating_sub(2); // leave room for "◦ "
    let mut lines: Vec<Line<'static>> = Vec::new();
    for e in entries {
        match e {
            Entry::Msg(t) => push_prose(&mut lines, t, message),
            Entry::Report(t) => {
                lines.push(Line::from(""));
                push_prose(&mut lines, t, message.add_modifier(Modifier::BOLD));
            }
            Entry::Tool(t) => lines.push(Line::from(vec![
                Span::styled("◦ ", tool),
                Span::styled(truncate(t, tool_budget), tool),
            ])),
        }
    }
    lines
}

/// Push one prose entry, marking each hard line with `›` (blank lines stay
/// blank). Soft word-wrap of long lines is left to the Paragraph.
fn push_prose(lines: &mut Vec<Line<'static>>, text: &str, style: Style) {
    for raw in text.split('\n') {
        if raw.trim().is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled("› ", style),
            Span::styled(raw.to_string(), style),
        ]));
    }
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let accent = Style::default().fg(app.theme.header);
    let mut spans = vec![
        Span::styled("q", accent.add_modifier(Modifier::BOLD)),
        Span::styled(" · ", accent),
        Span::styled("quit", accent),
    ];
    if let Some(status) = &app.status {
        spans.push(Span::styled("   ", accent));
        spans.push(Span::styled(
            status.clone(),
            Style::default().fg(app.theme.status),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Count the display rows `lines` occupy when word-wrapped to `width`, by
/// reflowing them through ratatui's own renderer into a scratch buffer — this
/// matches the wrapping the body Paragraph applies, so following/scrolling can
/// reach the true bottom. The scratch buffer height is bounded by an upper
/// estimate. (Mirrors the kanban detail pane's measurement.)
fn wrapped_height(lines: &[Line<'static>], width: u16) -> u16 {
    let width = width.max(1);
    let bound: usize = lines
        .iter()
        .map(|l| {
            let chars: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            chars / width as usize + 1
        })
        .sum::<usize>()
        .clamp(1, 100_000);
    let area = Rect::new(0, 0, width, bound as u16);
    let mut buf = Buffer::empty(area);
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .render(area, &mut buf);
    let mut rows = 1u16;
    for y in 0..area.height {
        if (0..width).any(|x| !buf[(x, y)].symbol().trim().is_empty()) {
            rows = y + 1;
        }
    }
    rows
}

/// Cap `s` at `max` display characters, appending `…` when clipped. Counts by
/// `char` so a multi-byte boundary is never split.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    match max {
        0 => String::new(),
        1 => "…".to_string(),
        _ => {
            let taken: String = s.chars().take(max - 1).collect();
            format!("{taken}…")
        }
    }
}
