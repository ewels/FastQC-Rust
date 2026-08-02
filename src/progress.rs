//! Live terminal progress reporting.
//!
//! Replaces the line-per-5% `Approx N% complete for <file>` output inherited
//! from Java FastQC with a rich, in-place display:
//!
//! ```text
//! FastQC-Rust v1.0.2-dev0
//!
//!   sample_1.fastq.gz  ⠹ ━━━━━━━━━━━━━━━━━━━━╸━━━━━━━  72%  2.1M reads     4s
//!   sample_2.fastq.gz  ✔ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 100%  3.0M reads     6s
//!
//!   ┌───────────────────────────────────┬────────────────┬────────────────┐
//!   │ Measure                           │ sample_1.fast… │ sample_2.fast… │
//!   ├───────────────────────────────────┼────────────────┼────────────────┤
//!   │ File type                         │ Conventional … │ Conventional … │
//!   │ ...                               │ ...            │ ...            │
//!   └───────────────────────────────────┴────────────────┴────────────────┘
//! ```
//!
//! The display adapts to the size of the run:
//!
//! * **1-10 files** get a progress bar each, in the order given on the command
//!   line, showing that file's own progress.
//! * **More than 10 files** collapse to a single bar counting completed files,
//!   because a screenful of bars is worse than no bars at all.
//! * **1-4 files** additionally get a live statistics table underneath, whose
//!   columns are the files and whose rows are the Basic Statistics measures
//!   from the top of the HTML report. Cells start as `-` and fill in as the
//!   analysis runs; the final values are exactly those in the report, because
//!   both are rendered from the same counters (see
//!   [`crate::modules::basic_stats::BasicStatsCounters::rows`]).
//!
//! # Animation and colour
//!
//! These are two independent switches, each auto-detected and each overridable
//! through the environment — there are no command-line flags for them.
//! `--quiet` beats both and says nothing but errors.
//!
//! **Animation** (`FASTQC_PROGRESS=auto|always|never`) — the default `auto`
//! draws the display only for an interactive stderr. When stderr is a pipe, a
//! log file or a workflow engine's capture, or when `TERM` says the terminal
//! cannot handle a redrawn region (`dumb`, or unset on Unix), a redrawn region
//! would be noise or outright corruption, so the reporter degrades to one plain
//! line per file at start and finish. `always` draws it regardless — for
//! recording a demo, or feeding a consumer that re-renders the stream — by
//! handing indicatif the terminal directly instead of its self-hiding stderr
//! draw target, sizing itself from `COLUMNS`/`LINES` since there is no terminal
//! to measure. `never` always takes the plain path.
//!
//! **Colour** follows [`console::colors_enabled_stderr`], which implements the
//! usual conventions with no help from us: `NO_COLOR` disables colour, the
//! [clicolors spec](https://bixense.com/clicolors/) `CLICOLOR=0` disables it and
//! `CLICOLOR_FORCE=1` forces it on even for a pipe, and `TERM=dumb` disables it.
//! indicatif's template styling reads the same function, so one signal covers
//! this module's styling and the bars alike.
//!
//! Because the two are independent: colour off still draws the bars, just
//! without escape codes; and the plain fallback still colours its lines when
//! colour is forced on, which is what a CI log viewer wants — it renders escape
//! sequences happily while not being a terminal.
//!
//! # Log lines
//!
//! Anything printed while the display is up has to go through
//! [`log_line`] (or [`ProgressReporter::error`]), which hands it to indicatif to
//! print *above* the redrawn region. The line then scrolls up as ordinary
//! output and the display stays pinned below it, so nothing is overwritten and
//! nothing overwrites the bars. Writing to stderr directly would land in the
//! middle of the display and be erased by the next frame.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use console::{style, truncate_str, Term};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle, TermLike};

use crate::modules::basic_stats::{BasicStatsCounters, LiveStats};

/// Environment variable selecting the progress display, matching the
/// `FASTQC_`-prefixed convention already used for the gzip backend.
pub const PROGRESS_ENV: &str = "FASTQC_PROGRESS";

/// A tri-state switch for behaviour that is normally auto-detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum When {
    /// Decide from the environment (default).
    #[default]
    Auto,
    /// Force on, whatever the environment looks like.
    Always,
    /// Force off.
    Never,
}

impl When {
    /// Parse a `FASTQC_PROGRESS`-style value. Unrecognised values fall back to
    /// `Auto` rather than failing a run over a display preference.
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "always" | "force" | "1" | "yes" | "true" | "on" => When::Always,
            "never" | "none" | "0" | "no" | "false" | "off" => When::Never,
            _ => When::Auto,
        }
    }

    fn from_env(name: &str) -> Self {
        std::env::var(name)
            .map(|v| Self::parse(&v))
            .unwrap_or_default()
    }

    /// The forced value, or `None` when the environment should decide.
    fn forced(self) -> Option<bool> {
        match self {
            When::Auto => None,
            When::Always => Some(true),
            When::Never => Some(false),
        }
    }
}

/// Above this many files, per-file bars are replaced by a single bar counting
/// completed files.
const MAX_FILE_BARS: usize = 10;

/// Above this many files, the live statistics table is not shown: the columns
/// would be too narrow to be worth reading.
const MAX_TABLE_COLUMNS: usize = 4;

/// How often the live statistics table is re-rendered.
const TABLE_REFRESH: Duration = Duration::from_millis(150);

/// Redraw rate for a forced display, matching indicatif's own default for its
/// stderr draw target.
const DRAW_RATE_HZ: u8 = 20;

/// Spinner animation period for the running bars.
const SPINNER_TICK: Duration = Duration::from_millis(90);

/// Fallback terminal size when it cannot be measured and the environment does
/// not say (`--progress always` over a pipe, for instance).
const DEFAULT_TERM_WIDTH: usize = 100;
const DEFAULT_TERM_HEIGHT: u16 = 24;

/// Longest a file name may be before it is truncated in a bar label.
const MAX_NAME_WIDTH: usize = 30;

/// Progress is tracked in permille rather than percent so the bars move
/// smoothly rather than in 1% steps.
const SCALE: u64 = 1000;

/// The display currently drawing to stderr, if there is one.
///
/// Code deep in the analysis (a module noticing bad data, a reader hitting an
/// odd record) has no handle on the reporter, but its warnings must not be
/// written straight to stderr while a redrawn region is on screen — they would
/// land in the middle of the bars and be overwritten by the next frame. Routing
/// them through here makes them scroll away above the display instead.
static ACTIVE_DISPLAY: Mutex<Option<MultiProgress>> = Mutex::new(None);

/// Emit a line that scrolls above the progress display, wherever it is called
/// from. Falls back to plain stderr when no display is active.
///
/// The line is printed exactly once and never redrawn, so it behaves like an
/// ordinary log line: the display stays pinned below it.
pub fn log_line(message: &str) {
    let active = ACTIVE_DISPLAY.lock().unwrap_or_else(|e| e.into_inner());
    match active.as_ref() {
        Some(multi) => print_above(multi, message),
        None => eprintln!("{}", message),
    }
}

/// Print `message` above the redrawn region.
///
/// `suspend` rather than `println`: `println` clears the display using a line
/// count captured before it takes the lock, so a bar update landing from
/// another thread in between leaves a stale copy of the whole display stranded
/// on screen. `suspend` holds indicatif's lock across clear, print and redraw,
/// which is exactly the atomicity a concurrent run needs.
fn print_above(multi: &MultiProgress, message: &str) {
    // Erase the whole region, write the line as ordinary scrollback, and let
    // the next update redraw the display beneath it.
    //
    // Not `println` or `suspend`: both erase using the line count from the last
    // frame indicatif drew, and with bars being updated from several threads
    // that count can lag what is actually on screen, which strands a stale copy
    // of the display above the live one. `clear` resets the count to zero, so
    // there is no bookkeeping left to be wrong.
    let _ = multi.clear();
    eprintln!("{}", message);
}

/// The terminal progress display for a whole run.
///
/// Cheap to share across the rayon workers: every method is `&self` and
/// no-ops when the display is disabled.
pub struct ProgressReporter {
    mode: Mode,
}

enum Mode {
    /// `--quiet`: say nothing at all.
    Silent,
    /// No redrawn display: one plain line per file at start and finish. Still
    /// colour-aware, because a CI log viewer renders escape sequences happily
    /// even though it is not a terminal.
    Plain { colors: bool },
    /// A terminal: live bars, and optionally a live statistics table.
    Live(Box<Live>),
}

struct Live {
    multi: MultiProgress,
    /// Pinned header and spacer at the top of the redrawn region.
    header: Vec<ProgressBar>,
    bars: Bars,
    table: Option<Arc<Table>>,
    /// Blank line kept at the bottom of the redrawn region.
    trailer: ProgressBar,
    ticker: Mutex<Option<JoinHandle<()>>>,
    stop: Arc<AtomicBool>,
    colors: bool,
    /// Progress bar style applied to a file once it has finished cleanly.
    done_style: ProgressStyle,
    failed_style: ProgressStyle,
}

enum Bars {
    /// One bar per file, indexed by file group.
    PerFile(Vec<ProgressBar>),
    /// A single bar counting completed files.
    Aggregate(ProgressBar),
}

/// Which of the three display modes a run should use.
///
/// Split out from [`ProgressReporter::new`] so the decision can be tested
/// without a terminal or environment fiddling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeChoice {
    Silent,
    Plain,
    Live,
}

/// Decide how to report progress.
///
/// `--quiet` wins over everything: it is the stronger statement, so
/// `--quiet --progress always` is still silent. Otherwise `--progress`
/// decides if it was given explicitly, and only `auto` consults the
/// environment.
///
/// Auto-detection needs a terminal the display can redraw in place: stderr must
/// be a tty *and* `TERM` must describe a terminal that can do more than accept
/// plain text. `dumb` (and, on Unix, an unset `TERM`) fails that second test —
/// indicatif hides its bars in exactly that case, so without the check a run
/// would print a banner and then go completely silent.
fn choose_mode(
    quiet: bool,
    progress: When,
    stderr_is_terminal: bool,
    dumb_terminal: bool,
) -> ModeChoice {
    if quiet {
        return ModeChoice::Silent;
    }
    match progress.forced() {
        Some(true) => ModeChoice::Live,
        Some(false) => ModeChoice::Plain,
        None if stderr_is_terminal && !dumb_terminal => ModeChoice::Live,
        None => ModeChoice::Plain,
    }
}

impl ProgressReporter {
    /// Build a reporter for a run over `names` (the file group display names,
    /// in command-line order).
    pub fn new(names: &[String], quiet: bool) -> Self {
        let progress = When::from_env(PROGRESS_ENV);
        let forced_display = progress == When::Always;
        let choice = choose_mode(
            quiet,
            progress,
            std::io::stderr().is_terminal(),
            console::is_dumb(),
        );
        let mode = match choice {
            ModeChoice::Silent => Mode::Silent,
            ModeChoice::Plain => Mode::Plain {
                colors: console::colors_enabled_stderr(),
            },
            ModeChoice::Live => Mode::Live(Box::new(Live::new(names, forced_display))),
        };
        ProgressReporter { mode }
    }

    /// A reporter that displays nothing. Used by tests and by callers of the
    /// library API that drive the analysis themselves.
    pub fn hidden() -> Self {
        ProgressReporter { mode: Mode::Silent }
    }

    /// A handle scoped to one file group, for the code that actually runs the
    /// analysis.
    pub fn file(&self, index: usize) -> FileProgress<'_> {
        FileProgress {
            reporter: self,
            index,
        }
    }

    /// The live statistics sink for a file group, if the table is being shown.
    /// Attached to the file's BasicStats module so it can publish partial
    /// results as it works.
    pub fn live_stats(&self, index: usize) -> Option<Arc<LiveStats>> {
        match &self.mode {
            Mode::Live(live) => live
                .table
                .as_ref()
                .and_then(|t| t.columns.get(index))
                .map(|c| Arc::clone(&c.live)),
            _ => None,
        }
    }

    /// Print an error line above the display. Shown even under `--quiet`,
    /// matching the previous behaviour of the runner.
    pub fn error(&self, message: &str) {
        match &self.mode {
            Mode::Live(live) => print_above(&live.multi, &paint_error(live.colors, message)),
            Mode::Plain { colors } => eprintln!("{}", paint_error(*colors, message)),
            // Silent still reports errors, but has no colour context to use.
            Mode::Silent => eprintln!("{}", message),
        }
    }

    /// Tear the display down once every file is done, leaving the final state
    /// on screen.
    pub fn finish(&self) {
        if let Mode::Live(live) = &self.mode {
            live.finish();
        }
    }

    fn on_start(&self, index: usize, name: &str) {
        match &self.mode {
            Mode::Silent => {}
            Mode::Plain { colors } => {
                eprintln!("Started analysis of {}", paint(*colors, name, |s| s.bold()))
            }
            Mode::Live(live) => live.start(index),
        }
    }

    fn on_progress(&self, index: usize, fraction: f64, reads: u64) {
        if let Mode::Live(live) = &self.mode {
            live.progress(index, fraction, reads);
        }
    }

    fn on_stage(&self, index: usize, stage: &str) {
        if let Mode::Live(live) = &self.mode {
            live.stage(index, stage);
        }
    }

    fn on_finish(&self, index: usize, name: &str, reads: u64) {
        match &self.mode {
            Mode::Silent => {}
            Mode::Plain { colors } => eprintln!(
                "Analysis complete for {}",
                paint(*colors, name, |s| s.bold())
            ),
            Mode::Live(live) => live.finish_file(index, reads),
        }
    }

    fn on_fail(&self, index: usize) {
        if let Mode::Live(live) = &self.mode {
            live.fail_file(index);
        }
    }
}

/// A progress handle for a single file group.
#[derive(Clone, Copy)]
pub struct FileProgress<'a> {
    reporter: &'a ProgressReporter,
    index: usize,
}

impl FileProgress<'_> {
    /// The live statistics sink for this file, if the table is being shown.
    pub fn live_stats(&self) -> Option<Arc<LiveStats>> {
        self.reporter.live_stats(self.index)
    }

    /// Announce that analysis of this file has begun.
    pub fn start(&self, name: &str) {
        self.reporter.on_start(self.index, name);
    }

    /// Report how far through the file the reader is. `percent` is the value
    /// from `SequenceFile::percent_complete`; `reads` is the number of records
    /// handed to the modules so far.
    pub fn update(&self, percent: f64, reads: u64) {
        self.reporter
            .on_progress(self.index, percent / 100.0, reads);
    }

    /// Note that the file has been read and the run has moved on to another
    /// phase (rendering charts, writing the report).
    pub fn stage(&self, stage: &str) {
        self.reporter.on_stage(self.index, stage);
    }

    /// Mark the file finished.
    pub fn finish(&self, name: &str, reads: u64) {
        self.reporter.on_finish(self.index, name, reads);
    }

    /// Mark the file failed.
    pub fn fail(&self) {
        self.reporter.on_fail(self.index);
    }
}

impl Live {
    /// `forced` is set by `--progress always`, meaning the display must be
    /// drawn even though the environment says it should not be.
    fn new(names: &[String], forced: bool) -> Self {
        // Honours NO_COLOR, CLICOLOR/CLICOLOR_FORCE and TERM=dumb, plus any
        // `--color` override already applied. indicatif resolves the
        // `.cyan`/`.green` parts of its templates against the same function, so
        // this single flag covers the entire display.
        let colors = console::colors_enabled_stderr();
        let term = ForcedTerm::new();
        let term_width = term.width() as usize;

        // The default stderr draw target hides itself when stderr is not an
        // interactive terminal. `--progress always` asks for the display
        // anyway, so bypass that check by handing indicatif the terminal
        // directly: `term_like` performs no detection of its own. It also
        // applies no rate limiting unless one is given, which would redraw the
        // whole display on every single position update, so pass the same
        // refresh rate `ProgressDrawTarget::stderr` uses.
        let multi = if forced {
            MultiProgress::with_draw_target(ProgressDrawTarget::term_like_with_hz(
                Box::new(term),
                DRAW_RATE_HZ,
            ))
        } else {
            MultiProgress::new()
        };

        // The name and version are a pinned header at the top of the redrawn
        // region rather than a line printed once before it. That keeps them
        // directly above the bars for the whole run: anything printed with
        // `println` scrolls past *above* the header instead of separating it
        // from the display.
        let banner = static_line(&multi);
        banner.set_message(format!(
            "{} {}",
            paint(colors, "FastQC-Rust", |s| s.cyan().bold()),
            paint(colors, &format!("v{}", crate::RUST_VERSION), |s| s.dim()),
        ));
        let spacer = static_line(&multi);
        spacer.set_message(" ");
        let header = vec![banner, spacer];

        let label_width = names
            .iter()
            .map(|n| display_width(n))
            .max()
            .unwrap_or(0)
            .min(MAX_NAME_WIDTH)
            .max("FastQ files".len());

        let bars = if names.len() > MAX_FILE_BARS {
            // Too many files for a bar each: count completed files instead.
            let bar = multi.add(ProgressBar::new(names.len() as u64));
            bar.set_style(aggregate_style(colors));
            bar.set_prefix(pad_label(
                &paint(colors, "FastQ files", |s| s.bold()),
                "FastQ files",
                label_width,
            ));
            bar.enable_steady_tick(SPINNER_TICK);
            Bars::Aggregate(bar)
        } else {
            let running = running_style(colors);
            let bars = names
                .iter()
                .map(|name| {
                    let bar = multi.add(ProgressBar::new(SCALE));
                    let shown = truncate_str(name, label_width, "…").into_owned();
                    bar.set_style(running.clone());
                    bar.set_prefix(pad_label(
                        &paint(colors, &shown, |s| s.bold()),
                        &shown,
                        label_width,
                    ));
                    bar.set_message("waiting");
                    bar.tick();
                    bar
                })
                .collect();
            Bars::PerFile(bars)
        };

        // The live statistics table only makes sense for a handful of files.
        let table = if names.len() <= MAX_TABLE_COLUMNS {
            Some(Arc::new(Table::new(&multi, names, term_width, colors)))
        } else {
            None
        };

        // A blank line below everything, so the shell prompt does not land
        // flush against the display.
        let trailer = static_line(&multi);
        trailer.set_message(" ");

        *ACTIVE_DISPLAY.lock().unwrap_or_else(|e| e.into_inner()) = Some(multi.clone());

        let live = Live {
            multi,
            header,
            bars,
            table,
            trailer,
            ticker: Mutex::new(None),
            stop: Arc::new(AtomicBool::new(false)),
            colors,
            done_style: done_style(colors),
            failed_style: failed_style(colors),
        };
        live.start_ticker();
        live
    }

    /// Drive the statistics table from a background thread: the analysis
    /// threads only ever publish counters, they never render.
    fn start_ticker(&self) {
        let Some(table) = self.table.as_ref().map(Arc::clone) else {
            return;
        };
        let stop = Arc::clone(&self.stop);
        let handle = std::thread::Builder::new()
            .name("fastqc-progress".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    table.refresh();
                    std::thread::sleep(TABLE_REFRESH);
                }
                // One last render so the table shows the finished values.
                table.refresh();
            });
        if let Ok(handle) = handle {
            *self.ticker.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        }
    }

    fn bar(&self, index: usize) -> Option<&ProgressBar> {
        match &self.bars {
            Bars::PerFile(bars) => bars.get(index),
            Bars::Aggregate(_) => None,
        }
    }

    fn start(&self, index: usize) {
        if let Some(bar) = self.bar(index) {
            bar.set_message("reading");
            bar.enable_steady_tick(SPINNER_TICK);
        }
    }

    fn progress(&self, index: usize, fraction: f64, reads: u64) {
        if let Some(bar) = self.bar(index) {
            bar.set_position((fraction.clamp(0.0, 1.0) * SCALE as f64) as u64);
            bar.set_message(format!("{} reads", human_count(reads)));
        }
    }

    fn stage(&self, index: usize, stage: &str) {
        if let Some(bar) = self.bar(index) {
            bar.set_position(SCALE);
            bar.set_message(stage.to_string());
        }
    }

    fn finish_file(&self, index: usize, reads: u64) {
        match &self.bars {
            Bars::PerFile(bars) => {
                if let Some(bar) = bars.get(index) {
                    bar.set_style(self.done_style.clone());
                    bar.set_position(SCALE);
                    bar.set_message(format!("{} reads", human_count(reads)));
                    bar.finish();
                }
            }
            Bars::Aggregate(bar) => bar.inc(1),
        }
    }

    fn fail_file(&self, index: usize) {
        match &self.bars {
            Bars::PerFile(bars) => {
                if let Some(bar) = bars.get(index) {
                    bar.set_style(self.failed_style.clone());
                    bar.set_message("failed");
                    bar.abandon();
                }
            }
            Bars::Aggregate(bar) => bar.inc(1),
        }
    }

    fn finish(&self) {
        if let Bars::Aggregate(bar) = &self.bars {
            bar.set_style(aggregate_done_style(self.colors));
            bar.finish();
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.ticker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
        // indicatif erases any bar that is still unfinished when it is dropped,
        // so the static lines have to be explicitly finished for the completed
        // display to survive the end of the run.
        *ACTIVE_DISPLAY.lock().unwrap_or_else(|e| e.into_inner()) = None;
        for line in &self.header {
            line.finish();
        }
        if let Some(table) = &self.table {
            table.finish();
        }
        self.trailer.finish();
    }
}

/// Add a line of static text to the redrawn region. Implemented as a progress
/// bar that renders nothing but its message, which is how indicatif keeps a
/// block of text pinned below the bars.
fn static_line(multi: &MultiProgress) -> ProgressBar {
    let line = multi.add(ProgressBar::new(0));
    line.set_style(ProgressStyle::with_template("{msg}").expect("static template"));
    line.tick();
    line
}

/// A stderr terminal that reports a usable size even when stderr is not a tty.
///
/// `console::Term` writes its cursor-movement and clear-line escapes
/// unconditionally, so it drives the display fine over a pipe; what it cannot
/// do is measure a pipe, and it falls back to a hardcoded 80 columns. That
/// would leave `{wide_bar}` sized differently from the statistics table, which
/// measures itself. Both go through this type instead, so they agree — and
/// `COLUMNS`/`LINES` give a forced run (a recording, a CI job) a way to say how
/// wide the output should be.
#[derive(Debug)]
struct ForcedTerm {
    inner: Term,
    width: u16,
    height: u16,
}

impl ForcedTerm {
    fn new() -> Self {
        // Buffered, like the `Term` behind indicatif's own stderr draw target.
        // It matters: indicatif emits a frame as a run of writes and flushes at
        // the end, and with an unbuffered terminal a concurrent draw can land
        // in the middle of one, running two bar lines onto the same row.
        let inner = Term::buffered_stderr();
        let measured = inner.size_checked();
        ForcedTerm {
            width: measured
                .map(|(_, cols)| cols)
                .or_else(|| env_dimension("COLUMNS"))
                .unwrap_or(DEFAULT_TERM_WIDTH as u16),
            height: measured
                .map(|(rows, _)| rows)
                .or_else(|| env_dimension("LINES"))
                .unwrap_or(DEFAULT_TERM_HEIGHT),
            inner,
        }
    }
}

fn env_dimension(name: &str) -> Option<u16> {
    std::env::var(name)
        .ok()?
        .parse::<u16>()
        .ok()
        .filter(|n| *n > 0)
}

impl TermLike for ForcedTerm {
    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn move_cursor_up(&self, n: usize) -> std::io::Result<()> {
        self.inner.move_cursor_up(n)
    }

    fn move_cursor_down(&self, n: usize) -> std::io::Result<()> {
        self.inner.move_cursor_down(n)
    }

    fn move_cursor_right(&self, n: usize) -> std::io::Result<()> {
        self.inner.move_cursor_right(n)
    }

    fn move_cursor_left(&self, n: usize) -> std::io::Result<()> {
        self.inner.move_cursor_left(n)
    }

    fn write_line(&self, s: &str) -> std::io::Result<()> {
        self.inner.write_line(s)
    }

    fn write_str(&self, s: &str) -> std::io::Result<()> {
        self.inner.write_str(s)
    }

    fn clear_line(&self) -> std::io::Result<()> {
        self.inner.clear_line()
    }

    fn flush(&self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// The live Basic Statistics table shown underneath the progress bars.
///
/// The whole table is a *single* zero-length progress bar whose message spans
/// several lines — indicatif splits a message on newlines and accounts for
/// every line. One bar rather than one per row matters: a refresh is then a
/// single message update and so a single redraw, instead of a dozen redraws of
/// the entire display several times a second, which churns the terminal and
/// races with anything trying to print above it.
struct Table {
    line: ProgressBar,
    columns: Vec<Column>,
    /// Row labels, in report order.
    measures: Vec<&'static str>,
    label_width: usize,
    value_width: usize,
    colors: bool,
}

struct Column {
    /// File name as shown in the header, already truncated to the column width.
    heading: String,
    live: Arc<LiveStats>,
}

impl Table {
    fn new(multi: &MultiProgress, names: &[String], term_width: usize, colors: bool) -> Self {
        let measures: Vec<&'static str> = BasicStatsCounters::default()
            .rows()
            .into_iter()
            .map(|(measure, _)| measure)
            .collect();

        let (label_width, value_width) = layout(&measures, names.len(), term_width);

        let columns = names
            .iter()
            .map(|name| Column {
                heading: truncate_str(name, value_width, "…").into_owned(),
                live: Arc::new(LiveStats::new()),
            })
            .collect();

        let table = Table {
            line: static_line(multi),
            columns,
            measures,
            label_width,
            value_width,
            colors,
        };
        table.refresh();
        table
    }

    /// Mark the table finished so it is not erased when the progress bars
    /// behind it are dropped at the end of the run.
    fn finish(&self) {
        self.line.finish();
    }

    /// Re-render every line from the latest published counters.
    fn refresh(&self) {
        // One formatted set of values per column. A column that has not
        // published yet shows "-" everywhere, so an idle file reads as idle
        // rather than as a file of zero reads.
        let values: Vec<Vec<String>> = self
            .columns
            .iter()
            .map(|column| match column.live.snapshot() {
                None => vec!["-".to_string(); self.measures.len()],
                Some(counters) => counters.rows().into_iter().map(|(_, v)| v).collect(),
            })
            .collect();

        let mut out: Vec<String> = Vec::with_capacity(self.measures.len() + 5);
        // A single space rather than an empty string: indicatif skips lines
        // that render to nothing, and the spacer is wanted.
        out.push(" ".to_string());
        out.push(self.rule('┌', '┬', '┐'));
        out.push(
            self.row(
                &paint(self.colors, "Measure", |s| s.dim()),
                "Measure",
                self.columns
                    .iter()
                    .map(|c| {
                        (
                            paint(self.colors, &c.heading, |s| s.cyan().bold()),
                            c.heading.clone(),
                        )
                    })
                    .collect(),
            ),
        );
        out.push(self.rule('├', '┼', '┤'));

        // Row labels and ordering come straight from the report's own table.
        for (index, measure) in self.measures.iter().enumerate() {
            let cells = values
                .iter()
                .map(|column| {
                    let shown = truncate_str(&column[index], self.value_width, "…").into_owned();
                    (paint(self.colors, &shown, |s| s.white()), shown)
                })
                .collect();
            out.push(self.row(measure, measure, cells));
        }
        out.push(self.rule('└', '┴', '┘'));

        // One update, one redraw.
        self.line.set_message(out.join("\n"));
    }

    /// A horizontal border line.
    fn rule(&self, left: char, mid: char, right: char) -> String {
        let mut s = String::from("  ");
        s.push(left);
        s.push_str(&"─".repeat(self.label_width + 2));
        for _ in &self.columns {
            s.push(mid);
            s.push_str(&"─".repeat(self.value_width + 2));
        }
        s.push(right);
        paint(self.colors, &s, |st| st.dim())
    }

    /// A content line. Cells are passed as `(styled, plain)` pairs because the
    /// styled form carries ANSI escapes that must not be counted when padding.
    fn row(&self, label: &str, label_plain: &str, cells: Vec<(String, String)>) -> String {
        let bar = paint(self.colors, "│", |s| s.dim());
        let mut s = String::from("  ");
        s.push_str(&bar);
        s.push(' ');
        s.push_str(&pad_label(label, label_plain, self.label_width));
        s.push(' ');
        for (styled, plain) in cells {
            s.push_str(&bar);
            s.push(' ');
            s.push_str(&pad_label(&styled, &plain, self.value_width));
            s.push(' ');
        }
        s.push_str(&bar);
        s
    }
}

/// Work out the column widths for a table of `columns` files, given the
/// measure labels and the terminal width.
///
/// The measure column gets its natural width when there is room; otherwise the
/// value columns are squeezed first, then the measure column, so that the table
/// stays inside the terminal instead of wrapping.
fn layout(measures: &[&str], columns: usize, term_width: usize) -> (usize, usize) {
    let natural_label = measures.iter().map(|m| display_width(m)).max().unwrap_or(8);
    let columns = columns.max(1);
    // Two leading spaces, one border per column plus two outer ones, and a
    // space either side of every cell.
    let overhead = 2 + (columns + 2) + 2 * (columns + 1);
    let available = term_width.saturating_sub(overhead);

    let mut label_width = natural_label;
    let mut value_width = available.saturating_sub(label_width) / columns;
    value_width = value_width.clamp(6, 28);

    // Still too wide? Take it out of the measure column, down to a floor where
    // the labels are at least recognisable.
    let total = label_width + value_width * columns;
    if total > available {
        label_width = available.saturating_sub(value_width * columns).max(10);
    }
    (label_width, value_width)
}

/// Pad `styled` (which may contain ANSI escapes) to `width` columns, measuring
/// with the escape-free `plain` form.
fn pad_label(styled: &str, plain: &str, width: usize) -> String {
    let used = display_width(plain);
    let mut out = String::from(styled);
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

fn display_width(s: &str) -> usize {
    console::measure_text_width(s)
}

/// Apply terminal styling, but only when the destination supports colour.
fn paint(
    colors: bool,
    text: &str,
    apply: impl FnOnce(console::StyledObject<&str>) -> console::StyledObject<&str>,
) -> String {
    if !colors {
        return text.to_string();
    }
    apply(style(text).force_styling(true)).to_string()
}

fn paint_error(colors: bool, text: &str) -> String {
    paint(colors, text, |s| s.red())
}

/// Template key rendering the elapsed time compactly and dimmed, shared by
/// every bar style so the last column always looks the same.
fn elapsed_key(
    colors: bool,
) -> impl Fn(&indicatif::ProgressState, &mut dyn std::fmt::Write) + Clone + Send + Sync + 'static {
    move |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
        let _ = write!(
            w,
            "{}",
            paint(colors, &short_duration(state.elapsed()), |s| s.dim())
        );
    }
}

/// Bar characters chosen to match the heavy-line look of Python's `rich`.
const PROGRESS_CHARS: &str = "━╸━";

fn running_style(colors: bool) -> ProgressStyle {
    ProgressStyle::with_template(
        "  {prefix} {spinner:.cyan} {wide_bar:.cyan/238} {percent:>3}% {msg:<12} {elapsed:>5}",
    )
    .expect("static template")
    .progress_chars(PROGRESS_CHARS)
    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
    .with_key("elapsed", elapsed_key(colors))
}

fn done_style(colors: bool) -> ProgressStyle {
    ProgressStyle::with_template(&format!(
        "  {{prefix}} {} {{wide_bar:.green/238}} {{percent:>3}}% {{msg:<12}} {{elapsed:>5}}",
        paint(colors, "✔", |s| s.green().bold())
    ))
    .expect("static template")
    .progress_chars(PROGRESS_CHARS)
    .with_key("elapsed", elapsed_key(colors))
}

fn failed_style(colors: bool) -> ProgressStyle {
    // Same column layout as the running and finished styles so a failed file
    // does not knock the other bars out of alignment. The bar is left at
    // whatever fraction of the file had been read when the error hit.
    ProgressStyle::with_template(&format!(
        "  {{prefix}} {} {{wide_bar:.red/238}} {{percent:>3}}% {} {{elapsed:>5}}",
        paint(colors, "✘", |s| s.red().bold()),
        paint(colors, "{msg:<12}", |s| s.red()),
    ))
    .expect("static template")
    .progress_chars(PROGRESS_CHARS)
    .with_key("elapsed", elapsed_key(colors))
}

fn aggregate_style(colors: bool) -> ProgressStyle {
    ProgressStyle::with_template(
        "  {prefix} {spinner:.cyan} {wide_bar:.cyan/238} {pos}/{len} files {elapsed:>5}",
    )
    .expect("static template")
    .progress_chars(PROGRESS_CHARS)
    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
    .with_key("elapsed", elapsed_key(colors))
}

fn aggregate_done_style(colors: bool) -> ProgressStyle {
    ProgressStyle::with_template(&format!(
        "  {{prefix}} {} {{wide_bar:.green/238}} {{pos}}/{{len}} files {{elapsed:>5}}",
        paint(colors, "✔", |s| s.green().bold())
    ))
    .expect("static template")
    .progress_chars(PROGRESS_CHARS)
    .with_key("elapsed", elapsed_key(colors))
}

/// Compact elapsed time: `4.2s`, `1m12s`, `1h04m`.
fn short_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{:.1}s", d.as_secs_f64())
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Abbreviate a read count for display: `812`, `12.3k`, `3.0M`.
fn human_count(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEASURES: [&str; 7] = [
        "File type",
        "Encoding",
        "Total Sequences",
        "Total Bases",
        "Sequences flagged as poor quality",
        "Sequence length",
        "%GC",
    ];

    #[test]
    fn test_human_count() {
        assert_eq!(human_count(0), "0");
        assert_eq!(human_count(999), "999");
        assert_eq!(human_count(1_500), "1.5k");
        assert_eq!(human_count(3_000_000), "3.0M");
        assert_eq!(human_count(2_500_000_000), "2.5B");
    }

    #[test]
    fn test_short_duration() {
        assert_eq!(short_duration(Duration::from_millis(4200)), "4.2s");
        assert_eq!(short_duration(Duration::from_secs(72)), "1m12s");
        assert_eq!(short_duration(Duration::from_secs(3840)), "1h04m");
    }

    /// The table must fit inside the terminal for every supported column count
    /// and a wide range of widths, including absurdly narrow ones.
    #[test]
    fn test_table_layout_fits_terminal() {
        for term_width in [40usize, 60, 80, 100, 120, 200] {
            for columns in 1..=MAX_TABLE_COLUMNS {
                let (label_width, value_width) = layout(&MEASURES, columns, term_width);
                let overhead = 2 + (columns + 2) + 2 * (columns + 1);
                let total = overhead + label_width + value_width * columns;
                assert!(
                    total <= term_width.max(overhead + 10 + 6 * columns),
                    "table of {} columns overflows {} cols (needs {})",
                    columns,
                    term_width,
                    total
                );
                assert!(value_width >= 6, "value column collapsed");
                assert!(label_width >= 10, "measure column collapsed");
            }
        }
    }

    /// A wide terminal should give the measure column its natural width.
    #[test]
    fn test_table_layout_natural_width() {
        let (label_width, value_width) = layout(&MEASURES, 2, 160);
        assert_eq!(label_width, "Sequences flagged as poor quality".len());
        assert_eq!(value_width, 28);
    }

    /// Auto-detection: the live display needs both a tty and a terminal that
    /// can be redrawn.
    #[test]
    fn test_choose_mode_auto() {
        let auto = When::Auto;
        // is_terminal, dumb
        assert_eq!(choose_mode(false, auto, true, false), ModeChoice::Live);
        // Piped or redirected stderr: plain lines, never bars.
        assert_eq!(choose_mode(false, auto, false, false), ModeChoice::Plain);
        // A tty that cannot redraw (TERM=dumb, or unset on Unix): indicatif
        // would hide the bars, so fall back rather than going silent.
        assert_eq!(choose_mode(false, auto, true, true), ModeChoice::Plain);
        assert_eq!(choose_mode(false, auto, false, true), ModeChoice::Plain);
    }

    /// `--progress always/never` overrides the detection in both directions.
    #[test]
    fn test_choose_mode_forced() {
        for &is_terminal in &[true, false] {
            for &dumb in &[true, false] {
                assert_eq!(
                    choose_mode(false, When::Always, is_terminal, dumb),
                    ModeChoice::Live,
                    "--progress always must draw the display (tty={is_terminal}, dumb={dumb})"
                );
                assert_eq!(
                    choose_mode(false, When::Never, is_terminal, dumb),
                    ModeChoice::Plain,
                    "--progress never must not draw the display (tty={is_terminal}, dumb={dumb})"
                );
            }
        }
    }

    /// `--quiet` is the stronger statement and beats an explicit
    /// `--progress always`.
    #[test]
    fn test_quiet_beats_forced_progress() {
        for progress in [When::Auto, When::Always, When::Never] {
            for &is_terminal in &[true, false] {
                assert_eq!(
                    choose_mode(true, progress, is_terminal, false),
                    ModeChoice::Silent,
                    "--quiet must win over --progress {progress:?}"
                );
            }
        }
    }

    #[test]
    fn test_when_forced() {
        assert_eq!(When::Auto.forced(), None);
        assert_eq!(When::Always.forced(), Some(true));
        assert_eq!(When::Never.forced(), Some(false));
    }

    #[test]
    fn test_when_parse() {
        for on in [
            "always", "ALWAYS", " always ", "force", "1", "yes", "true", "on",
        ] {
            assert_eq!(When::parse(on), When::Always, "{on:?}");
        }
        for off in ["never", "Never", "none", "0", "no", "false", "off"] {
            assert_eq!(When::parse(off), When::Never, "{off:?}");
        }
        // Anything unrecognised falls back to detection rather than failing the
        // run over a display preference.
        for other in ["", "auto", "maybe", "yes please"] {
            assert_eq!(When::parse(other), When::Auto, "{other:?}");
        }
    }

    /// With no display active, a log line still reaches stderr rather than
    /// being swallowed.
    #[test]
    fn test_log_line_without_a_display() {
        assert!(ACTIVE_DISPLAY
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none());
        log_line("no display active, so this goes straight to stderr");
    }

    /// With colour off, styling helpers must emit the bare text — no escapes,
    /// and no change in width.
    #[test]
    fn test_paint_without_colors_is_plain() {
        assert_eq!(paint(false, "Measure", |s| s.cyan().bold()), "Measure");
        assert_eq!(paint_error(false, "boom"), "boom");
        assert!(!paint(false, "x", |s| s.red()).contains('\u{1b}'));
        // ...and with colour on it really does emit escapes, so the test above
        // is not passing vacuously.
        assert!(paint(true, "x", |s| s.red()).contains('\u{1b}'));
    }

    #[test]
    fn test_pad_label_ignores_ansi() {
        let styled = paint(true, "abc", |s| s.red());
        let padded = pad_label(&styled, "abc", 6);
        assert_eq!(display_width(&padded), 6);
        assert!(padded.starts_with(&styled));
    }

    /// A hidden reporter must be safe to drive exactly like a live one.
    #[test]
    fn test_hidden_reporter_is_inert() {
        let reporter = ProgressReporter::hidden();
        let file = reporter.file(0);
        file.start("a.fastq");
        file.update(50.0, 1000);
        file.stage("writing report");
        file.finish("a.fastq", 2000);
        file.fail();
        assert!(reporter.live_stats(0).is_none());
        reporter.finish();
    }
}
