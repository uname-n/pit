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
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
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
/// subagent's text replies render as markdown under a `›` bullet (headers,
/// emphasis, inline/fenced code, lists), its tool calls appear as truncated `◦`
/// one-liners, and the final report closes the stream. `log_dir` is where the
/// orchestration helper tees the transcripts.
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
    // True only while the tail of the stream is a `thinking_tokens` heartbeat —
    // i.e. the subagent is mid-think and hasn't yet emitted the resulting
    // content. Cleared the moment any real event (assistant/user/result) lands.
    thinking: bool,
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
            thinking: false,
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
            match ingest(self.buf.trim_end(), &mut self.entries) {
                Ingest::Result => {
                    self.thinking = false;
                    self.done = true;
                    self.buf.clear();
                    got += 1;
                    break;
                }
                // A `thinking_tokens` heartbeat lights the indicator; any other
                // event means the think (if any) has resolved into content.
                Ingest::Thinking => self.thinking = true,
                Ingest::Other => self.thinking = false,
            }
            self.buf.clear();
            got += 1;
        }
        if got > 0 {
            self.last_data = Instant::now();
        } else if self.last_data.elapsed() >= IDLE_TIMEOUT {
            self.status = Some("no result event (run interrupted or idle)".into());
            self.done = true;
        }
    }
}

/// What one stream-json line signals to the follow loop: the terminal `result`
/// event, a live `thinking_tokens` heartbeat (the run is mid-think), or anything
/// else (content shown or ignored).
enum Ingest {
    Result,
    Thinking,
    Other,
}

/// Parse one stream-json event line into zero or more `entries`. Assistant
/// `text` blocks become `Msg`, `tool_use` blocks become `Tool`; assistant
/// `thinking` blocks and other block types are skipped. A `system` /
/// `thinking_tokens` event returns [`Ingest::Thinking`] (it carries no entry);
/// the terminal `result` returns [`Ingest::Result`]. Malformed lines are
/// ignored and count as [`Ingest::Other`].
fn ingest(line: &str, entries: &mut Vec<Entry>) -> Ingest {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Ingest::Other;
    };
    match v.get("type").and_then(Value::as_str) {
        Some("system")
            if v.get("subtype").and_then(Value::as_str) == Some("thinking_tokens") =>
        {
            Ingest::Thinking
        }
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
            Ingest::Other
        }
        Some("result") => {
            let report = v.get("result").and_then(Value::as_str).unwrap_or("");
            if !report.trim().is_empty() {
                entries.push(Entry::Report(report.trim().to_string()));
            }
            Ingest::Result
        }
        _ => Ingest::Other,
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
    // header · pad · body · pad · footer — the blank pad rows breathe a little
    // space between the header title and the body, and the body and quit line.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
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
    let body = chunks[2];
    app.view_h = body.height;
    let mut lines = build_lines(&app.entries, body.width, &theme);
    // Ephemeral activity indicator: while `thinking_tokens` heartbeats are
    // streaming (see `App::thinking`) the subagent is mid-think, so pin a dim
    // `• thinking…` line to the tail. It's not an `Entry`, so it never persists —
    // it vanishes the moment the next real event lands and `thinking` clears.
    if app.thinking && !app.done {
        lines.push(Line::from(Span::styled(
            "• thinking…",
            Style::default().fg(theme.status),
        )));
    }
    // Lines are pre-wrapped (prose in `push_prose`, tools truncated), so each is
    // exactly one visual row — the total is just the count.
    let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
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
    f.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), body);

    render_footer(f, chunks[4], app);
}

/// Flatten the entry list into styled, prefix-marked lines pre-wrapped to
/// `width` — one `Line` per visual row. A message is marked `›` on its first
/// row and hanging-indented on the rest; tool calls are truncated to a single
/// `◦` row so they never wrap.
fn build_lines(entries: &[Entry], width: u16, theme: &TailTheme) -> Vec<Line<'static>> {
    let message = Style::default().fg(theme.message);
    let tool = Style::default().fg(theme.tool);
    let result = Style::default().fg(theme.result);
    let tool_budget = (width as usize).saturating_sub(2); // leave room for "◦ "
    let mut lines: Vec<Line<'static>> = Vec::new();
    for e in entries {
        match e {
            Entry::Msg(t) => push_markdown(&mut lines, t, message, theme, width),
            Entry::Report(t) => {
                lines.push(Line::from(""));
                let base = result.add_modifier(Modifier::BOLD);
                push_markdown(&mut lines, t, base, theme, width);
            }
            Entry::Tool(t) => lines.push(Line::from(vec![
                Span::styled("◦ ", tool),
                Span::styled(truncate(t, tool_budget), tool),
            ])),
        }
    }
    lines
}

/// Push one prose entry rendered as markdown, keeping the tail's hanging-indent
/// gutter: the first non-blank visual row of the message carries the `›` marker
/// and every continuation row — a later hard line, a soft-wrap, or a list-item
/// hang — aligns beneath it. Supports ATX headers, `**bold**` / `*italic*`,
/// inline `` `code` ``, fenced code blocks (verbatim), blockquotes, and `-`/`*`
/// bullets. Everything is word-wrapped to `width` here (not by the Paragraph) so
/// the indent survives the wrap; `base` is the entry's default text style.
fn push_markdown(lines: &mut Vec<Line<'static>>, text: &str, base: Style, theme: &TailTheme, width: u16) {
    let inner = (width as usize).saturating_sub(2); // room for the "› "/"  " gutter
    let code = Style::default().fg(theme.tool);
    let mut marked = false;
    let mut in_fence = false;
    for raw in text.split('\n') {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence; // swallow the fence marker line itself
            continue;
        }
        if in_fence {
            // Code is verbatim (never reflowed) — hard-split only if over-wide.
            for row in char_wrap(raw, inner) {
                emit_block(lines, &mut marked, base, Vec::new(), vec![vec![Span::styled(row, code)]]);
            }
            continue;
        }
        if raw.trim().is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        if let Some((level, rest)) = header_split(trimmed) {
            let hstyle = header_style(level, theme);
            let rows = wrap_styled(&[Span::styled(rest.to_string(), hstyle)], inner, hstyle);
            emit_block(lines, &mut marked, base, Vec::new(), rows);
        } else if let Some(rest) = trimmed.strip_prefix("> ") {
            let quote = Style::default().fg(theme.status).add_modifier(Modifier::ITALIC);
            let lead = vec![Span::styled("│ ", Style::default().fg(theme.status))];
            let rows = wrap_styled(&inline(rest, quote, code), inner.saturating_sub(2), quote);
            emit_block(lines, &mut marked, base, lead, rows);
        } else if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            let indent = raw.len() - trimmed.len(); // nested-bullet leading spaces
            let mut lead = Vec::new();
            if indent > 0 {
                lead.push(Span::raw(" ".repeat(indent)));
            }
            lead.push(Span::styled("• ", Style::default().fg(theme.tool)));
            let rows = wrap_styled(&inline(rest, base, code), inner.saturating_sub(indent + 2), base);
            emit_block(lines, &mut marked, base, lead, rows);
        } else {
            let rows = wrap_styled(&inline(raw, base, code), inner, base);
            emit_block(lines, &mut marked, base, Vec::new(), rows);
        }
    }
}

/// Emit `rows` (already wrapped to fit) under the message gutter. The first
/// visual row of the whole message gets the `›` marker (tracked via `marked`),
/// the rest get a two-space indent. `lead` is a per-block marker (bullet `•`,
/// quote `│`) shown on the block's first row; continuation rows pad by its width
/// so wrapped text hangs under the content rather than the marker.
fn emit_block(
    lines: &mut Vec<Line<'static>>,
    marked: &mut bool,
    gutter: Style,
    lead: Vec<Span<'static>>,
    rows: Vec<Vec<Span<'static>>>,
) {
    let lead_w: usize = lead.iter().map(|s| s.content.chars().count()).sum();
    for (r, row) in rows.into_iter().enumerate() {
        let marker = if *marked {
            "  "
        } else {
            *marked = true;
            "› "
        };
        let mut spans = vec![Span::styled(marker, gutter)];
        if r == 0 {
            spans.extend(lead.iter().cloned());
        } else if lead_w > 0 {
            spans.push(Span::raw(" ".repeat(lead_w)));
        }
        spans.extend(row);
        lines.push(Line::from(spans));
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

/// Word-wrap a run of styled spans into rows of at most `width` display columns,
/// breaking on whitespace and hard-splitting any single word wider than `width`.
/// Per-character styles are preserved across the wrap; the space inserted
/// between two words is styled with `base`. Whitespace runs collapse to a single
/// space. Never returns an empty vec (an all-blank input yields one empty row).
fn wrap_styled(spans: &[Span], width: usize, base: Style) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    // Split the styled char stream into words (maximal non-space runs), each
    // carrying its per-char style so emphasis survives the wrap.
    let mut words: Vec<Vec<(char, Style)>> = Vec::new();
    let mut word: Vec<(char, Style)> = Vec::new();
    for s in spans {
        for ch in s.content.chars() {
            if ch.is_whitespace() {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            } else {
                word.push((ch, s.style));
            }
        }
    }
    if !word.is_empty() {
        words.push(word);
    }

    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    for word in words {
        if word.len() > width {
            // Oversized word: flush, then hard-split across rows.
            if !cur.is_empty() {
                rows.push(std::mem::take(&mut cur));
            }
            for &pair in &word {
                if cur.len() == width {
                    rows.push(std::mem::take(&mut cur));
                }
                cur.push(pair);
            }
            continue;
        }
        let sep = usize::from(!cur.is_empty());
        if cur.len() + sep + word.len() > width {
            rows.push(std::mem::take(&mut cur));
        } else if sep == 1 {
            cur.push((' ', base));
        }
        cur.extend_from_slice(&word);
    }
    rows.push(cur); // always ≥ 1 row, so the result is never empty
    rows.iter().map(|r| coalesce(r)).collect()
}

/// Merge `(char, style)` pairs into the minimal run of styled spans.
fn coalesce(chars: &[(char, Style)]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur: Option<Style> = None;
    for &(ch, st) in chars {
        match cur {
            Some(s) if s == st => buf.push(ch),
            _ => {
                if let Some(s) = cur.take() {
                    spans.push(Span::styled(std::mem::take(&mut buf), s));
                }
                buf.push(ch);
                cur = Some(st);
            }
        }
    }
    if let Some(s) = cur {
        spans.push(Span::styled(buf, s));
    }
    spans
}

/// Hard-split `s` into rows of at most `width` chars, breaking mid-word (used
/// for fenced code, which must render verbatim rather than reflow). Always
/// returns at least one row so a blank code line stays blank.
fn char_wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut n = 0usize;
    for ch in s.chars() {
        if n == width {
            rows.push(std::mem::take(&mut cur));
            n = 0;
        }
        cur.push(ch);
        n += 1;
    }
    rows.push(cur);
    rows
}

/// Parse inline markdown — `**bold**`, `*italic*`/`_italic_`, and `` `code` `` —
/// into styled spans layered over `base`; inline code takes the `code` style.
/// Unterminated markers are treated as literal text.
fn inline(s: &str, base: Style, code: Style) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut italic = false;
    let flush = |buf: &mut String, bold: bool, italic: bool, out: &mut Vec<Span<'static>>| {
        if buf.is_empty() {
            return;
        }
        let mut st = base;
        if bold {
            st = st.add_modifier(Modifier::BOLD);
        }
        if italic {
            st = st.add_modifier(Modifier::ITALIC);
        }
        out.push(Span::styled(std::mem::take(buf), st));
    };
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            flush(&mut buf, bold, italic, &mut out);
            let mut j = i + 1;
            let mut cd = String::new();
            while j < chars.len() && chars[j] != '`' {
                cd.push(chars[j]);
                j += 1;
            }
            if j < chars.len() {
                out.push(Span::styled(cd, code));
                i = j + 1;
                continue;
            }
            buf.push('`'); // no closing backtick — keep the literal
            i += 1;
            continue;
        }
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            flush(&mut buf, bold, italic, &mut out);
            bold = !bold;
            i += 2;
            continue;
        }
        if (c == '*' || c == '_') && chars.get(i + 1) != Some(&c) {
            flush(&mut buf, bold, italic, &mut out);
            italic = !italic;
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, bold, italic, &mut out);
    out
}

/// Split an ATX header (`#`..`######` then a space) into its level and text.
fn header_split(s: &str) -> Option<(usize, &str)> {
    let bytes = s.as_bytes();
    let mut level = 0;
    while level < 6 && bytes.get(level) == Some(&b'#') {
        level += 1;
    }
    if level == 0 || bytes.get(level) != Some(&b' ') {
        return None;
    }
    Some((level, &s[level + 1..]))
}

/// The style for a level-`level` header: the theme accent, bold, and underlined
/// from level 4 down so the deeper (rarer) headers still read as distinct.
fn header_style(level: usize, theme: &TailTheme) -> Style {
    let mut st = Style::default()
        .fg(theme.header)
        .add_modifier(Modifier::BOLD);
    if level >= 4 {
        st = st.add_modifier(Modifier::UNDERLINED);
    }
    st
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    // Distinct colors per role so a span's origin is unambiguous in assertions.
    fn theme() -> TailTheme {
        TailTheme {
            header: Color::Rgb(1, 0, 0),
            message: Color::Rgb(0, 1, 0),
            tool: Color::Rgb(0, 0, 1),
            status: Color::Rgb(1, 1, 0),
            result: Color::Rgb(0, 1, 1),
        }
    }

    // The visible text of a rendered line (all span contents concatenated).
    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn render_msg(md: &str, width: u16) -> Vec<Line<'static>> {
        build_lines(&[Entry::Msg(md.to_string())], width, &theme())
    }

    #[test]
    fn header_marker_stripped_and_themed() {
        let lines = render_msg("## Overview", 40);
        assert_eq!(text(&lines[0]), "› Overview");
        // The header text carries the header accent + bold; the gutter does not.
        let hdr = &lines[0].spans[1];
        assert_eq!(hdr.content, "Overview");
        assert_eq!(hdr.style.fg, Some(theme().header));
        assert!(hdr.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn only_first_row_gets_the_gutter_marker() {
        let lines = render_msg("line one\nline two", 40);
        assert_eq!(text(&lines[0]), "› line one");
        assert_eq!(text(&lines[1]), "  line two");
    }

    #[test]
    fn long_line_wraps_with_hanging_indent() {
        // width 12 → inner 10; "alpha beta gamma" packs "alpha beta" then "gamma".
        let lines = render_msg("alpha beta gamma", 12);
        assert_eq!(text(&lines[0]), "› alpha beta");
        assert_eq!(text(&lines[1]), "  gamma");
    }

    #[test]
    fn inline_bold_and_code_are_styled_without_markers() {
        let lines = render_msg("a **b** `c`", 40);
        // Markers are gone; the plain text is "a b c".
        assert_eq!(text(&lines[0]), "› a b c");
        let bold = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "b")
            .expect("bold span");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let code = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "c")
            .expect("code span");
        assert_eq!(code.style.fg, Some(theme().tool));
    }

    #[test]
    fn bullets_get_marker_and_hang_under_text() {
        // width 14 → content budget 10 → "one two three" wraps after "two".
        let lines = render_msg("- one two three", 14);
        assert_eq!(text(&lines[0]), "› • one two");
        // Continuation hangs under the text, past the "• " lead (gutter 2 + 2).
        assert_eq!(text(&lines[1]), "    three");
    }

    #[test]
    fn fenced_code_is_verbatim_and_fence_hidden() {
        let lines = render_msg("```\nlet x = 1;\n```", 40);
        assert_eq!(lines.len(), 1); // just the code line — no fence rows
        assert_eq!(text(&lines[0]), "› let x = 1;");
        assert_eq!(lines[0].spans[1].style.fg, Some(theme().tool));
    }

    #[test]
    fn blank_lines_survive_between_blocks() {
        let lines = render_msg("a\n\nb", 40);
        assert_eq!(text(&lines[0]), "› a");
        assert_eq!(text(&lines[1]), "");
        assert_eq!(text(&lines[2]), "  b");
    }
}
