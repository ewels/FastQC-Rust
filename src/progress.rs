//! Live terminal progress reporting.
//!
//! Replaces the line-per-5% `Approx N% complete for <file>` output inherited
//! from Java FastQC with a rich, in-place display:
//!
//! ```text
//! FastQC-Rust v1.0.2-dev0
//!
//! Failed to process notes.txt: ID line didn't start with '@' at line 1
//!   sample_1.fastq.gz  ⠹ ━━━━━━━━━━━━━━━━━━━━╸━━━━━━━  72%  2.1M reads     4s
//!   sample_2.fastq.gz  ✔ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 100%  3.0M reads     6s
//!
//!   ┌───────────────────────────────────┬────────────────┬────────────────┐
//!   │ Measure                           │ sample_1.fast… │ sample_2.fast… │
//!   ├───────────────────────────────────┼────────────────┼────────────────┤
//!   │ File type                         │ Conventional … │ Conventional … │
//!   │ ...                               │ ...            │ ...            │
//!   └───────────────────────────────────┴────────────────┴────────────────┘
//! Complete. Analysed 2 files in 00:06
//! ```
//!
//! The bars, the table and the closing summary are a redrawn region pinned to
//! the bottom of the terminal. The name and version, and any warning or error,
//! are ordinary log lines that scroll up above it.
//!
//! The display adapts to the size of the run:
//!
//! * **1-10 files** get a progress bar each, in the order given on the command
//!   line, showing that file's own progress.
//! * **More than 10 files** collapse to a single bar counting completed files,
//!   because a screenful of bars is worse than no bars at all.
//! * **A live statistics table** is drawn underneath whenever the terminal is
//!   wide enough for every column to be readable — so a wide terminal gets it
//!   for more files, and a narrow one does without. Its columns are the files
//!   and its rows the Basic Statistics measures from the top of the HTML
//!   report. Cells start as `-` and fill in as the analysis runs; the final
//!   values are exactly those in the report, because both are rendered from the
//!   same counters (see
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
//! Anything printed while the display is up has to go through [`log_line`] —
//! or [`log_line_once`] from an inner analysis loop, or
//! [`ProgressReporter::error`]. Writing to stderr directly would land in the
//! middle of the display and be erased by the next frame.
//!
//! Those lines are written *above* the redrawn region and scroll up as ordinary
//! terminal output, which is what rich, tqdm, indicatif, cargo and Nextflow all
//! do. The log is the permanent record and has to be able to grow without
//! bound, and the terminal's scrollback is the only place unbounded output can
//! go. The version banner is printed by [`ProgressPlan::new`] before the run
//! does anything else, so everything the run has to say appears beneath it in
//! the order it happened.
//!
//! The exception is the closing `Complete. Analysed N files in mm:ss`, which is
//! the last line of the redrawn region so that it always lands below the bars
//! and the table. `--quiet` suppresses it along with everything else.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use console::{style, Term};
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

    fn from_env() -> Self {
        std::env::var(PROGRESS_ENV)
            .map(|v| Self::parse(&v))
            .unwrap_or_default()
    }
}

/// Above this many files, per-file bars are replaced by a single bar counting
/// completed files.
const MAX_FILE_BARS: usize = 10;

/// Widest a value column grows before the extra room is left as margin.
const MAX_VALUE_WIDTH: usize = 28;

/// Narrowest a statistics-table value column may be and still be worth reading.
/// Whether the table is shown at all is decided by whether every column can
/// have at least this much room (see [`layout`]), so a wide terminal shows
/// the table for more files and a narrow one drops it sooner.
const MIN_VALUE_WIDTH: usize = 16;

/// How often the live statistics table is re-rendered.
const TABLE_REFRESH: Duration = Duration::from_millis(150);

/// Redraw rate for a forced display, matching indicatif's own default for its
/// stderr draw target.
const DRAW_RATE_HZ: u8 = 20;

/// Spinner animation period for the running bars.
const SPINNER_TICK: Duration = Duration::from_millis(90);

/// Fallback terminal size when it cannot be measured and the environment does
/// not say (`FASTQC_PROGRESS=always` over a pipe, for instance).
const DEFAULT_TERM_WIDTH: usize = 100;
const DEFAULT_TERM_HEIGHT: u16 = 24;

/// Longest a file name may be before it is truncated in a bar label.
const MAX_NAME_WIDTH: usize = 30;

/// Progress is tracked in permille rather than percent so the bars move
/// smoothly rather than in 1% steps.
const SCALE: u64 = 1000;

/// Where log lines go while a display is on screen.
///
/// Lines are written above the redrawn region and scroll up as ordinary
/// terminal output, which is the convention for this kind of display (rich,
/// tqdm, indicatif, cargo, Nextflow all do the same). The log is the permanent
/// record and has to be able to grow without bound; the terminal's scrollback
/// is the only place unbounded output can go, and it is below the display that
/// there is no room.
struct LogSink {
    multi: MultiProgress,
    /// `\r\n` when the display has been forced onto something that is not a
    /// terminal. A tty driver rewrites `\n` as `\r\n` on the way out (ONLCR);
    /// nothing does that for a pipe, so a bare newline would leave the cursor
    /// parked in this line's column and the next frame would start there.
    line_ending: &'static str,
    /// First line of the redrawn region, blank once anything has been logged so
    /// the bars are not flush against the messages above them. It renders as
    /// nothing while empty, which is what keeps it out of the way on the usual
    /// run that logs nothing at all.
    padding: ProgressBar,
}

impl LogSink {
    fn print(&self, message: &str) {
        self.padding.set_message(" ");
        // Erase the region, write the line as ordinary scrollback, and let the
        // next update redraw the display beneath it.
        //
        // Not `println` or `suspend`: both erase using the line count from the
        // last frame indicatif drew, and with bars updating from several
        // threads that count lags what is on screen, which strands a stale copy
        // of the whole display above the live one. `clear` resets the count to
        // zero, so there is no bookkeeping left to be wrong.
        let _ = self.multi.clear();
        eprint!("{}{}", message, self.line_ending);
    }
}

/// The log sink of the display currently drawing to stderr, if there is one.
///
/// Code deep in the analysis (a module noticing bad data, a reader hitting an
/// odd record) has no handle on the reporter, but its warnings must not be
/// written straight to stderr while a redrawn region is on screen — they would
/// land in the middle of the bars and be overwritten by the next frame.
static ACTIVE_LOG: Mutex<Option<Arc<LogSink>>> = Mutex::new(None);

/// Emit a line above the progress display, wherever it is called from. Falls
/// back to plain stderr when no display is active.
pub fn log_line(message: &str) {
    let active = ACTIVE_LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match active {
        Some(sink) => sink.print(message),
        None => eprintln!("{}", message),
    }
}

/// What [`log_line_once`] has already said this run, cleared by
/// [`ProgressPlan::new`].
static SAID: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Emit a line unless this run has already emitted exactly it.
///
/// For warnings raised from the innermost analysis loops, where the same
/// message can be produced millions of times — once per bad base, or once per
/// unreadable read. Deduplicating here rather than with a latch at the call
/// site keeps the scope right: a latch is per *process*, so with several files
/// analysed in parallel only whichever one won the race would say anything,
/// and an embedder calling the library twice would hear nothing the second
/// time.
pub fn log_line_once(message: &str) {
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if !said.insert(message.to_string()) {
        return;
    }
    // Not while holding the lock: printing takes indicatif's draw lock, and
    // this one is taken from every analysis thread.
    drop(said);
    log_line(message);
}

/// The terminal progress display for a whole run.
///
/// Cheap to share across the rayon workers: every method is `&self` and
/// no-ops when the display is disabled.
pub struct ProgressReporter {
    mode: Mode,
    /// When the run started, for the closing summary.
    started: Instant,
}

enum Mode {
    /// `--quiet`: say nothing at all.
    Silent,
    /// No redrawn display: one plain line per file at start and finish. Still
    /// colour-aware, because a CI log viewer renders escape sequences happily
    /// even though it is not a terminal.
    Plain,
    /// A terminal: live bars, and optionally a live statistics table.
    Live(Box<Live>),
}

struct Live {
    bars: Bars,
    table: Option<Arc<Table>>,
    /// The closing summary, drawn as the last line of the region so it always
    /// lands below the bars and the table. Empty until the run finishes.
    summary: ProgressBar,
    /// Blank line kept at the bottom of the redrawn region.
    trailer: ProgressBar,
    /// Where log lines go: above the display, as ordinary scrollback.
    log: Arc<LogSink>,
    ticker: Mutex<Option<JoinHandle<()>>>,
    stop: Arc<AtomicBool>,
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
    /// `bypass_detection` when the display is going somewhere indicatif would
    /// refuse to draw to, which is the one thing `Live::new` needs to know: it
    /// then has to drive the terminal itself rather than through indicatif's
    /// self-hiding stderr target. That is a fact about the output, not about
    /// what was asked for — `FASTQC_PROGRESS=always` in a real terminal is an
    /// ordinary live display.
    Live {
        bypass_detection: bool,
    },
}

/// Decide how to report progress.
///
/// `--quiet` wins over everything: it is the stronger statement, so
/// `--quiet FASTQC_PROGRESS=always` is still silent. Otherwise
/// `FASTQC_PROGRESS` decides if it was given explicitly, and only `auto`
/// consults the environment.
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
    // The same condition indicatif's stderr draw target hides itself on, so
    // when it holds there is nothing to bypass and `always` costs nothing.
    let drawable = stderr_is_terminal && !dumb_terminal;
    match progress {
        When::Always => ModeChoice::Live {
            bypass_detection: !drawable,
        },
        When::Never => ModeChoice::Plain,
        When::Auto if drawable => ModeChoice::Live {
            bypass_detection: false,
        },
        When::Auto => ModeChoice::Plain,
    }
}

/// The mode this process would use, from `--quiet` and the environment.
///
/// Called once, by [`ProgressPlan::new`], which stores the answer so both
/// phases of startup act on the same decision. Split from [`choose_mode`] so
/// that the rule itself can be tested without a terminal or the environment.
fn current_choice(quiet: bool) -> ModeChoice {
    choose_mode(
        quiet,
        When::from_env(),
        // console's own tty probe, not `std::io::IsTerminal`: indicatif decides
        // whether to hide its bars with `console::Term::is_term`, and the two
        // disagree on an MSYS pty, where console recognises a terminal that
        // `IsTerminal` does not.
        Term::stderr().is_term(),
        console::is_dumb(),
    )
}

/// A decided-but-not-yet-drawn display, and the first half of starting a run.
///
/// The display cannot be built until the input files have been validated and
/// grouped, because it needs one bar per group — but the banner has to be
/// printed *before* that work, or the messages validation emits land above it
/// and the "everything the run says appears beneath the banner" property is
/// quietly false. So the decision and the banner happen here, at the very top
/// of the run, and [`start`](Self::start) turns the plan into a reporter once
/// the names are known.
pub struct ProgressPlan {
    choice: ModeChoice,
    started: Instant,
}

impl ProgressPlan {
    /// Decide how this run will report, and announce it. Call this first:
    /// the clock it starts is the one the closing summary reports.
    pub fn new(quiet: bool) -> Self {
        let choice = current_choice(quiet);
        // "Once per run" for `log_line_once` means once per plan.
        SAID.lock().unwrap_or_else(|e| e.into_inner()).clear();
        if let ModeChoice::Live { bypass_detection } = choice {
            // Coloured after the logo: the name in its blue, the `-Rust`
            // suffix in its red, the version dim. Written straight to stderr
            // because nothing has been drawn yet — there is no region to clear
            // and no padding to add.
            let line_ending = line_ending(bypass_detection);
            eprint!(
                "{}{} {}{line_ending}{line_ending}",
                paint("FastQC", |s| s.color256(LOGO_BLUE).bold()),
                paint("-Rust", |s| s.color256(LOGO_RED).bold()),
                paint(&format!("v{}", crate::RUST_VERSION), |s| s.dim()),
            );
        }
        ProgressPlan {
            choice,
            started: Instant::now(),
        }
    }

    /// Draw the display for `names` (the file group display names, in
    /// command-line order).
    pub fn start(self, names: &[String]) -> ProgressReporter {
        let mode = match self.choice {
            ModeChoice::Silent => Mode::Silent,
            ModeChoice::Plain => Mode::Plain,
            ModeChoice::Live { bypass_detection } => {
                Mode::Live(Box::new(Live::new(names, bypass_detection)))
            }
        };
        ProgressReporter {
            mode,
            started: self.started,
        }
    }
}

/// A tty driver rewrites `\n` as `\r\n` on the way out (ONLCR); nothing does
/// that for a pipe, so a bare newline would leave the cursor parked in this
/// line's column and the next frame would start there.
fn line_ending(bypass_detection: bool) -> &'static str {
    if bypass_detection {
        "\r\n"
    } else {
        "\n"
    }
}

impl ProgressReporter {
    /// A reporter that displays nothing. Used by tests and by callers of the
    /// library API that drive the analysis themselves.
    pub fn hidden() -> Self {
        ProgressReporter {
            mode: Mode::Silent,
            started: Instant::now(),
        }
    }

    /// A handle scoped to one file group, for the code that actually runs the
    /// analysis.
    pub fn file(&self, index: usize) -> FileProgress<'_> {
        FileProgress {
            reporter: self,
            index,
        }
    }

    /// Print an error line above the display. Shown even under `--quiet`,
    /// matching the previous behaviour of the runner.
    ///
    /// Routed through [`log_line`] rather than matched on the mode: the two
    /// would say the same thing, since a log sink is registered exactly while
    /// a display is up.
    pub fn error(&self, message: &str) {
        log_line(&paint_error(message));
    }

    /// Tear the display down once every file is done, leaving the final state
    /// on screen, and report what the run got through.
    ///
    /// `analysed` is the number of file groups that completed successfully;
    /// anything that failed has already been reported as an error line.
    pub fn finish(&self, analysed: usize) {
        let summary = format!(
            "Analysed {} {} in {}",
            analysed,
            if analysed == 1 { "file" } else { "files" },
            clock_duration(self.started.elapsed()),
        );
        match &self.mode {
            // --quiet stays quiet: the run said nothing, so it ends saying
            // nothing.
            Mode::Silent => {}
            Mode::Plain => eprintln!("{} {}", paint("Complete.", |s| s.green().bold()), summary),
            Mode::Live(live) => {
                // The last line of the redrawn region, so it lands below the
                // bars and the table rather than scrolling past above them.
                live.summary.set_message(format!(
                    "{} {}",
                    paint("Complete.", |s| s.green().bold()),
                    paint(&summary, |s| s.dim()),
                ));
                live.finish();
            }
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
    /// Attached to the file's BasicStats module so it can publish partial
    /// results as it works.
    pub fn live_stats(&self) -> Option<Arc<LiveStats>> {
        match &self.reporter.mode {
            Mode::Live(live) => live
                .table
                .as_ref()
                .and_then(|t| t.columns.get(self.index))
                .map(|c| Arc::clone(&c.live)),
            _ => None,
        }
    }

    /// Announce that analysis of this file has begun.
    pub fn start(&self, name: &str) {
        match &self.reporter.mode {
            Mode::Silent => {}
            Mode::Plain => eprintln!("Started analysis of {}", paint(name, |s| s.bold())),
            Mode::Live(live) => live.start(self.index),
        }
    }

    /// Report how far through the file the reader is. `reads` is the number of
    /// records handed to the modules so far.
    ///
    /// `percent` is a closure rather than a value because
    /// `SequenceFile::percent_complete` costs a seek on the input file, and
    /// there is often no bar for it to move — under `--quiet`, without a
    /// terminal, or when many files share a single completion-counting bar.
    /// Leaving that decision here keeps the reader from having to ask.
    pub fn update(&self, reads: u64, percent: impl FnOnce() -> f64) {
        if let Mode::Live(live) = &self.reporter.mode {
            live.progress(self.index, reads, percent);
        }
    }

    /// Note that the file has been read and the run has moved on to another
    /// phase (rendering charts, writing the report).
    pub fn stage(&self, stage: &str) {
        if let Mode::Live(live) = &self.reporter.mode {
            live.stage(self.index, stage);
        }
    }

    /// Mark the file finished.
    pub fn finish(&self, name: &str, reads: u64) {
        match &self.reporter.mode {
            Mode::Silent => {}
            Mode::Plain => eprintln!("Analysis complete for {}", paint(name, |s| s.bold())),
            Mode::Live(live) => live.finish_file(self.index, reads),
        }
    }

    /// Mark the file failed.
    pub fn fail(&self) {
        if let Mode::Live(live) = &self.reporter.mode {
            live.fail_file(self.index);
        }
    }
}

impl Live {
    /// `bypass_detection` means stderr is not something indicatif will draw a
    /// redrawn region to, and the display has been forced on anyway.
    fn new(names: &[String], bypass_detection: bool) -> Self {
        let term = ForcedTerm::new();
        let term_width = term.width() as usize;

        // The default stderr draw target hides itself when stderr is not an
        // interactive terminal. `FASTQC_PROGRESS=always` asks for the display
        // anyway, so bypass that check by handing indicatif the terminal
        // directly: `term_like` performs no detection of its own. It also
        // applies no rate limiting unless one is given, which would redraw the
        // whole display on every single position update, so pass the same
        // refresh rate `ProgressDrawTarget::stderr` uses.
        let multi = if bypass_detection {
            MultiProgress::with_draw_target(ProgressDrawTarget::term_like_with_hz(
                Box::new(term),
                DRAW_RATE_HZ,
            ))
        } else {
            MultiProgress::new()
        };

        let log = Arc::new(LogSink {
            multi: multi.clone(),
            line_ending: line_ending(bypass_detection),
            // First line of the region, so the blank it grows lands between the
            // messages and the bars.
            padding: static_line(&multi),
        });
        *ACTIVE_LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&log));

        let label_width = names
            .iter()
            .map(|n| console::measure_text_width(n))
            .max()
            .unwrap_or(0)
            .min(MAX_NAME_WIDTH)
            .max("FastQ files".len());

        let bars = if names.len() > MAX_FILE_BARS {
            // Too many files for a bar each: count completed files instead.
            let bar = multi.add(ProgressBar::new(names.len() as u64));
            bar.set_style(aggregate_style());
            bar.set_prefix(pad_cell(&paint("FastQ files", |s| s.bold()), label_width));
            Bars::Aggregate(bar)
        } else {
            let running = running_style();
            let bars = names
                .iter()
                .map(|name| {
                    let bar = multi.add(ProgressBar::new(SCALE));
                    bar.set_style(running.clone());
                    bar.set_prefix(pad_cell(&paint(name, |s| s.bold()), label_width));
                    bar.set_message("waiting");
                    bar.tick();
                    bar
                })
                .collect();
            Bars::PerFile(bars)
        };

        // Show the statistics table only when the terminal is wide enough to
        // give every column room to be read — which is exactly the question
        // `layout` answers. The table itself is unchanged either way: this
        // decides whether it appears, not how it looks.
        let table = layout(names.len(), term_width)
            .map(|widths| Arc::new(Table::new(&multi, names, widths)));

        // The closing summary is the last line of the region, so it always
        // lands below the bars and the table however the run went. It renders
        // as nothing until it has a message.
        let summary = static_line(&multi);

        // A blank line below everything, so the shell prompt does not land
        // flush against the display.
        let trailer = static_line(&multi);
        trailer.set_message(" ");

        let live = Live {
            bars,
            table,
            summary,
            trailer,
            log,
            ticker: Mutex::new(None),
            stop: Arc::new(AtomicBool::new(false)),
        };
        live.start_ticker();
        live
    }

    /// Animate the display from a single background thread: the analysis threads
    /// only ever publish counters and positions, they never render.
    ///
    /// indicatif's own `enable_steady_tick` spawns a thread per bar, which for
    /// a ten-file run means ten threads competing for the same draw lock purely
    /// to advance a spinner. One thread does both jobs.
    fn start_ticker(&self) {
        let table = self.table.as_ref().map(Arc::clone);
        let spinners: Vec<ProgressBar> = match &self.bars {
            Bars::PerFile(bars) => bars.clone(),
            Bars::Aggregate(bar) => vec![bar.clone()],
        };
        let stop = Arc::clone(&self.stop);
        let handle = std::thread::Builder::new()
            .name("fastqc-progress".into())
            .spawn(move || {
                // The table is re-rendered more slowly than the spinners are
                // advanced, on its own deadline rather than a second thread.
                let mut due = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    for bar in &spinners {
                        if !bar.is_finished() {
                            bar.tick();
                        }
                    }
                    let now = Instant::now();
                    if now >= due {
                        due = now + TABLE_REFRESH;
                        if let Some(table) = &table {
                            table.refresh();
                        }
                    }
                    std::thread::sleep(SPINNER_TICK);
                }
                // One last render so the table shows the finished values.
                if let Some(table) = &table {
                    table.refresh();
                }
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
        }
    }

    /// Both halves of this are guarded by a comparison against the bar's
    /// current state, because both `set_position` and `set_message` reach
    /// indicatif's redraw — which takes the global draw lock and re-formats the
    /// line — while `position` and `message` only take that one bar's lock.
    /// This runs once per thousand reads on every file at once, and most calls
    /// change nothing: the bar has only [`SCALE`] distinct positions, and
    /// `human_count` is coarse enough that past a million reads the same label
    /// is produced for a hundred consecutive updates.
    fn progress(&self, index: usize, reads: u64, percent: impl FnOnce() -> f64) {
        let Some(bar) = self.bar(index) else {
            return;
        };
        let position = (percent().clamp(0.0, 100.0) / 100.0 * SCALE as f64) as u64;
        if bar.position() != position {
            bar.set_position(position);
        }
        let label = format!("{} reads", human_count(reads));
        if bar.message() != label {
            bar.set_message(label);
        }
    }

    fn stage(&self, index: usize, stage: &str) {
        if let Some(bar) = self.bar(index) {
            bar.set_position(SCALE);
            bar.set_message(stage.to_string());
        }
    }

    /// Record how a file ended so the table heading can follow its bar.
    fn set_file_state(&self, index: usize, state: FileState) {
        if let Some(column) = self.table.as_ref().and_then(|t| t.columns.get(index)) {
            column.state.store(state as u8, Ordering::Relaxed);
        }
    }

    fn finish_file(&self, index: usize, reads: u64) {
        self.set_file_state(index, FileState::Analysed);
        match &self.bars {
            Bars::PerFile(bars) => {
                if let Some(bar) = bars.get(index) {
                    bar.set_style(done_style());
                    bar.set_position(SCALE);
                    bar.set_message(format!("{} reads", human_count(reads)));
                    bar.finish();
                }
            }
            Bars::Aggregate(bar) => bar.inc(1),
        }
    }

    fn fail_file(&self, index: usize) {
        self.set_file_state(index, FileState::Failed);
        match &self.bars {
            Bars::PerFile(bars) => {
                if let Some(bar) = bars.get(index) {
                    bar.set_style(failed_style());
                    bar.set_message("failed");
                    bar.abandon();
                }
            }
            Bars::Aggregate(bar) => bar.inc(1),
        }
    }

    fn finish(&self) {
        if let Bars::Aggregate(bar) = &self.bars {
            bar.set_style(aggregate_done_style());
            bar.finish();
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.ticker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
        // indicatif erases any bar that is still unfinished when it is dropped,
        // so the static lines have to be explicitly finished for the completed
        // display to survive the end of the run.
        *ACTIVE_LOG.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(table) = &self.table {
            table.finish();
        }
        self.log.padding.finish();
        self.summary.finish();
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

    /// `ESC [ n <op>`, the cursor-movement form. A zero count is a no-op
    /// rather than an escape, matching what console would emit.
    fn escape(&self, n: usize, op: char) -> std::io::Result<()> {
        if n == 0 {
            return Ok(());
        }
        self.inner.write_str(&format!("\x1b[{n}{op}"))
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

    // Cursor movement is written as ANSI rather than delegated to `Term`.
    // console drives a real Windows console through the Win32 API, which
    // silently does nothing when the handle is a pipe — so delegating would
    // draw every frame and erase none, leaving one long concatenation. This
    // type is only used when the display has been forced onto something that
    // is not a terminal, where the consumer is a recorder or a log viewer that
    // interprets escapes, so emitting them is exactly right. On Unix these are
    // the same bytes `Term` would have written.
    fn move_cursor_up(&self, n: usize) -> std::io::Result<()> {
        self.escape(n, 'A')
    }

    fn move_cursor_down(&self, n: usize) -> std::io::Result<()> {
        self.escape(n, 'B')
    }

    fn move_cursor_right(&self, n: usize) -> std::io::Result<()> {
        self.escape(n, 'C')
    }

    fn move_cursor_left(&self, n: usize) -> std::io::Result<()> {
        self.escape(n, 'D')
    }

    fn write_line(&self, s: &str) -> std::io::Result<()> {
        self.inner.write_line(s)
    }

    fn write_str(&self, s: &str) -> std::io::Result<()> {
        self.inner.write_str(s)
    }

    fn clear_line(&self) -> std::io::Result<()> {
        self.inner.write_str("\r\x1b[2K")
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
    value_width: usize,
    /// The fixed furniture, rendered once: the three borders, the styled
    /// vertical rule, the heading row's label cell and each measure's label
    /// cell. None of it changes during a run, and the table is re-rendered
    /// several times a second.
    top: String,
    divider: String,
    bottom: String,
    pipe: String,
    heading_label: String,
    /// One per measure, in report order — so this also fixes the row count.
    row_labels: Vec<String>,
    /// The last frame rendered, so an unchanged one can be dropped rather than
    /// pushed through indicatif and out to the terminal again.
    last: Mutex<String>,
}

struct Column {
    live: Arc<LiveStats>,
    /// The heading pre-rendered in each state, indexed by [`FileState`]: the
    /// colour is the only thing that varies, and re-styling it per frame would
    /// be pure waste.
    headings: [String; 3],
    /// Drives the heading colour, kept in step with the file's bar.
    state: AtomicU8,
}

impl Column {
    fn styled_heading(&self) -> &str {
        let state = self.state.load(Ordering::Relaxed) as usize;
        self.headings.get(state).unwrap_or(&self.headings[0])
    }
}

/// How a file is doing, for colouring its column heading the same way its
/// progress bar is coloured. The discriminant indexes [`Column::headings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileState {
    /// Waiting or being read.
    Running = 0,
    Analysed = 1,
    Failed = 2,
}

impl Table {
    /// `widths` comes from [`layout`], which is also what decided the table was
    /// worth drawing at all.
    fn new(
        multi: &MultiProgress,
        names: &[String],
        (label_width, value_width): (usize, usize),
    ) -> Self {
        let columns = names
            .iter()
            .map(|name| {
                // The heading tracks the file's bar: accent while it is
                // running, green once analysed, red if it failed.
                let headings = [
                    pad_cell(&paint(name, |s| s.cyan().bold()), value_width),
                    pad_cell(&paint(name, |s| s.green().bold()), value_width),
                    pad_cell(&paint(name, |s| s.red().bold()), value_width),
                ];
                Column {
                    live: Arc::new(LiveStats::new()),
                    headings,
                    state: AtomicU8::new(FileState::Running as u8),
                }
            })
            .collect::<Vec<_>>();

        let rule = |left: char, mid: char, right: char| {
            let mut s = String::from("  ");
            s.push(left);
            s.push_str(&"─".repeat(label_width + 2));
            for _ in &columns {
                s.push(mid);
                s.push_str(&"─".repeat(value_width + 2));
            }
            s.push(right);
            paint(&s, |st| st.dim())
        };

        let table = Table {
            line: static_line(multi),
            top: rule('┌', '┬', '┐'),
            divider: rule('├', '┼', '┤'),
            bottom: rule('└', '┴', '┘'),
            pipe: paint("│", |s| s.dim()),
            heading_label: pad_cell(&paint("Measure", |s| s.dim()), label_width),
            row_labels: BasicStatsCounters::MEASURES
                .iter()
                .map(|m| pad_cell(m, label_width))
                .collect(),
            columns,
            value_width,
            last: Mutex::new(String::new()),
        };
        table.refresh();
        table
    }

    /// Mark the table finished so it is not erased when the progress bars
    /// behind it are dropped at the end of the run.
    fn finish(&self) {
        self.line.finish();
    }

    /// Re-render the table from the latest published counters.
    ///
    /// Only the value cells are built here: the borders, the measure labels and
    /// the three styled forms of each heading are fixed for the run and were
    /// rendered once in `new`. The result is compared against the last frame
    /// and dropped if identical, which is the common case — columns only change
    /// every few thousand reads, and nothing changes at all while the reports
    /// are being written.
    fn refresh(&self) {
        let mut out: Vec<String> = Vec::with_capacity(self.row_labels.len() + 5);
        // A single space rather than an empty string: indicatif skips lines
        // that render to nothing, and the spacer is wanted.
        out.push(" ".to_string());
        out.push(self.top.clone());
        out.push(self.row(
            &self.heading_label,
            self.columns.iter().map(|c| c.styled_heading()),
        ));
        out.push(self.divider.clone());

        // A column that has not published yet shows "-" everywhere, so an idle
        // file reads as idle rather than as a file of zero reads.
        let values: Vec<Vec<String>> = self
            .columns
            .iter()
            .map(|column| match column.live.snapshot() {
                None => vec!["-".to_string(); self.row_labels.len()],
                Some(counters) => counters.rows().into_iter().map(|(_, v)| v).collect(),
            })
            .collect();

        // Row labels and ordering come straight from the report's own table.
        for (index, label) in self.row_labels.iter().enumerate() {
            let cells: Vec<String> = values
                .iter()
                .map(|column| self.value_cell(&column[index]))
                .collect();
            out.push(self.row(label, cells.iter().map(String::as_str)));
        }
        out.push(self.bottom.clone());

        let rendered = out.join("\n");
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if *last == rendered {
            return;
        }
        // One update, one redraw.
        self.line.set_message(rendered.clone());
        *last = rendered;
    }

    /// A value cell: styled, then fitted to the column.
    fn value_cell(&self, value: &str) -> String {
        pad_cell(&paint(value, |s| s.white()), self.value_width)
    }

    /// A content line, from cells that are already styled and padded.
    fn row<'a>(&self, label_cell: &str, cells: impl Iterator<Item = &'a str>) -> String {
        let mut s = String::from("  ");
        s.push_str(&self.pipe);
        s.push(' ');
        s.push_str(label_cell);
        s.push(' ');
        for cell in cells {
            s.push_str(&self.pipe);
            s.push(' ');
            s.push_str(cell);
            s.push(' ');
        }
        s.push_str(&self.pipe);
        s
    }
}

/// Terminal columns consumed by a table of `columns` files that are not cell
/// content: two of indentation, a border between and either side of every
/// cell, and a space either side of each.
fn table_overhead(columns: usize) -> usize {
    2 + (columns + 2) + 2 * (columns + 1)
}

/// The `(label_width, value_width)` of a table for `columns` files at this
/// terminal width, or `None` when it is not worth drawing — which is when the
/// measure column cannot have its natural width or a value column would be
/// narrower than [`MIN_VALUE_WIDTH`].
///
/// This is the single place the geometry is decided; returning `None` is also
/// how the display decides not to show the table at all.
fn layout(columns: usize, term_width: usize) -> Option<(usize, usize)> {
    if columns == 0 {
        return None;
    }
    let label_width = BasicStatsCounters::MEASURES
        .iter()
        .map(|m| console::measure_text_width(m))
        .max()
        .unwrap_or(8);
    let available = term_width
        .saturating_sub(table_overhead(columns))
        .checked_sub(label_width)?;

    let value_width = (available / columns).min(MAX_VALUE_WIDTH);
    (value_width >= MIN_VALUE_WIDTH).then_some((label_width, value_width))
}

/// Fit a cell to exactly `width` columns, padding it out or truncating it with
/// an ellipsis. `console::pad_str` measures with `measure_text_width`, which
/// skips ANSI escapes, so a styled cell lines up with a plain one and is cut
/// without losing its trailing reset.
fn pad_cell(text: &str, width: usize) -> String {
    console::pad_str(text, width, console::Alignment::Left, Some("…")).into_owned()
}

/// Style an error line red for stderr.
fn paint_error(text: &str) -> String {
    paint(text, |s| s.red())
}

/// Style text for stderr.
///
/// `for_stderr` makes console resolve the escape codes against
/// `colors_enabled_stderr()` when the value is rendered, which is the same
/// signal indicatif's template styling uses — so there is nothing for this
/// module to decide or thread around.
fn paint(
    text: &str,
    apply: impl FnOnce(console::StyledObject<&str>) -> console::StyledObject<&str>,
) -> String {
    apply(style(text).for_stderr()).to_string()
}

/// Template key rendering the elapsed time compactly and dimmed, shared by
/// every bar style so the last column always looks the same.
fn elapsed_key(
) -> impl Fn(&indicatif::ProgressState, &mut dyn std::fmt::Write) + Clone + Send + Sync + 'static {
    move |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
        let _ = write!(
            w,
            "{}",
            paint(&short_duration(state.elapsed()), |s| s.dim())
        );
    }
}

/// The FastQC logo's two colours, as the nearest xterm-256 entries.
///
/// Taken from the dark-background variant of the logo
/// (`docs/public/images/fastqc_logo_darkbg.svg`, `#659BFF` and `#AE3939`)
/// rather than the light-background one, whose navy `#000080` all but
/// disappears against a dark terminal. These mid-tones read on either.
///
/// Both are the closest entries in the 6x6x6 cube by CIELAB distance —
/// `#5f87ff` and `#d75f5f`. 256-colour rather than truecolor so that the one
/// code path works on every terminal; console emits 24-bit escapes without
/// checking whether the terminal can render them.
///
/// Public so the test that checks the banner is actually painted in them can
/// name them rather than restate the escape sequences.
pub const LOGO_BLUE: u8 = 69;
pub const LOGO_RED: u8 = 167;

/// Bar characters chosen to match the heavy-line look of Python's `rich`.
const PROGRESS_CHARS: &str = "━╸━";

/// Spinner frames. Inert for the styles whose template has no `{spinner}`.
const TICK_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ";

/// Build a bar style. The five the display uses differ only in the marker
/// before the bar, the bar's colour, and the field between the bar and the
/// elapsed time; everything else — the column layout, the bar characters and
/// the elapsed-time key — is shared, so that a change to the layout is made
/// once rather than five times.
///
/// `marker` and `middle` are substituted into the template, and `format!` does
/// not re-scan a substituted value for braces, so both can carry placeholders
/// of their own.
fn bar_style(marker: &str, color: &str, middle: &str) -> ProgressStyle {
    ProgressStyle::with_template(&format!(
        "  {{prefix}} {marker} {{wide_bar:.{color}/238}} {middle} {{elapsed:>5}}"
    ))
    .expect("static template")
    .progress_chars(PROGRESS_CHARS)
    .tick_chars(TICK_CHARS)
    .with_key("elapsed", elapsed_key())
}

/// The per-file fields: percentage, then a short status or read count.
const FILE_FIELDS: &str = "{percent:>3}% {msg:<12}";
/// The aggregate bar counts files rather than tracking one.
const AGGREGATE_FIELDS: &str = "{pos}/{len} files";

fn running_style() -> ProgressStyle {
    bar_style("{spinner:.cyan}", "cyan", FILE_FIELDS)
}

fn done_style() -> ProgressStyle {
    bar_style(&paint("✔", |s| s.green().bold()), "green", FILE_FIELDS)
}

fn failed_style() -> ProgressStyle {
    // Same column layout as the running and finished styles so a failed file
    // does not knock the other bars out of alignment. The bar is left at
    // whatever fraction of the file had been read when the error hit.
    bar_style(
        &paint("✘", |s| s.red().bold()),
        "red",
        &format!("{{percent:>3}}% {}", paint("{msg:<12}", |s| s.red())),
    )
}

fn aggregate_style() -> ProgressStyle {
    bar_style("{spinner:.cyan}", "cyan", AGGREGATE_FIELDS)
}

fn aggregate_done_style() -> ProgressStyle {
    bar_style(&paint("✔", |s| s.green().bold()), "green", AGGREGATE_FIELDS)
}

/// Wall-clock elapsed time for the closing summary: `mm:ss`, widening to
/// `hh:mm:ss` past an hour rather than letting the minutes run past 59.
fn clock_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 3600 {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    } else {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
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

    /// Auto-detection: the live display needs both a tty and a terminal that
    /// can be redrawn.
    #[test]
    fn test_choose_mode_auto() {
        let auto = When::Auto;
        // is_terminal, dumb
        assert_eq!(
            choose_mode(false, auto, true, false),
            ModeChoice::Live {
                bypass_detection: false
            }
        );
        // Piped or redirected stderr: plain lines, never bars.
        assert_eq!(choose_mode(false, auto, false, false), ModeChoice::Plain);
        // A tty that cannot redraw (TERM=dumb, or unset on Unix): indicatif
        // would hide the bars, so fall back rather than going silent.
        assert_eq!(choose_mode(false, auto, true, true), ModeChoice::Plain);
        assert_eq!(choose_mode(false, auto, false, true), ModeChoice::Plain);
    }

    /// `FASTQC_PROGRESS=always|never` overrides the detection in both
    /// directions. `always` bypasses indicatif's self-hiding draw target only
    /// where that target would in fact hide — driving the terminal by hand on a
    /// terminal that works would be a downgrade, not a force.
    #[test]
    fn test_choose_mode_forced() {
        // is_terminal, dumb, and whether the display has to be driven by hand.
        // Spelled out rather than recomputed from the rule, so that getting the
        // rule wrong fails the test instead of being copied into it.
        for (is_terminal, dumb, bypass_detection) in [
            (true, false, false),
            (true, true, true),
            (false, false, true),
            (false, true, true),
        ] {
            assert_eq!(
                choose_mode(false, When::Always, is_terminal, dumb),
                ModeChoice::Live { bypass_detection },
                "always must draw the display (tty={is_terminal}, dumb={dumb})"
            );
            assert_eq!(
                choose_mode(false, When::Never, is_terminal, dumb),
                ModeChoice::Plain,
                "never must not draw the display (tty={is_terminal}, dumb={dumb})"
            );
        }
    }

    /// `--quiet` is the stronger statement and beats an explicit `always`.
    #[test]
    fn test_quiet_beats_forced_progress() {
        for progress in [When::Auto, When::Always, When::Never] {
            for &is_terminal in &[true, false] {
                assert_eq!(
                    choose_mode(true, progress, is_terminal, false),
                    ModeChoice::Silent,
                    "--quiet must win over {progress:?}"
                );
            }
        }
    }

    /// With no display active, a log line still reaches stderr rather than
    /// being swallowed.
    #[test]
    fn test_log_line_without_a_display() {
        assert!(ACTIVE_LOG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none());
        log_line("no display active, so this goes straight to stderr");
    }

    /// Whenever the table is shown, it must fit inside the terminal, with the
    /// measure column at its natural width and every value column readable.
    #[test]
    fn test_a_shown_table_renders_inside_the_terminal() {
        let natural_label = BasicStatsCounters::MEASURES
            .iter()
            .map(|m| console::measure_text_width(m))
            .max()
            .unwrap();
        for term_width in [40usize, 60, 72, 80, 100, 120, 160, 200, 400] {
            for columns in 1..=12 {
                let Some((label_width, value_width)) = layout(columns, term_width) else {
                    continue;
                };
                let total = table_overhead(columns) + label_width + value_width * columns;
                assert!(
                    total <= term_width,
                    "shown table of {columns} columns overflows {term_width} cols (needs {total})"
                );
                assert!(
                    value_width >= MIN_VALUE_WIDTH,
                    "value column below the readable minimum at {term_width} cols"
                );
                assert_eq!(
                    label_width, natural_label,
                    "measure column was squeezed at {term_width} cols"
                );
            }
        }
    }

    /// The decision is made on width, not on how many files there are: a wider
    /// terminal earns more columns, a narrow one loses the table entirely.
    #[test]
    fn test_table_visibility_follows_terminal_width() {
        let fits = |columns, width| layout(columns, width).is_some();

        // Nothing to tabulate.
        assert!(!fits(0, 200));

        // 80 columns is enough for one or two files, not three.
        assert!(fits(1, 80));
        assert!(fits(2, 80));
        assert!(!fits(3, 80));

        // Widening the terminal brings more columns into range, and the
        // threshold only ever moves one way.
        for columns in 1..=10 {
            // The first width that fits. Asserting that narrower ones do not
            // would be vacuous — `find` returns the smallest by definition.
            // What has to hold is the other direction: every wider terminal
            // fits too, so the table never blinks out again as the window
            // grows.
            let threshold = (1..600)
                .find(|w| fits(columns, *w))
                .expect("some width is wide enough");
            assert!(
                (threshold..600).all(|w| fits(columns, w)),
                "{columns} columns: fits at {threshold} but not at every wider width"
            );
            // More files always need at least as much room.
            if columns > 1 {
                let narrower = (1..600).find(|w| fits(columns - 1, *w)).unwrap();
                assert!(narrower < threshold);
            }
        }

        // A 24-column terminal is never wide enough.
        assert!(!fits(1, 24));
    }

    /// A cell always occupies exactly its column, whether it had to be padded
    /// out or cut down, and styling it does not change that: widths are
    /// measured in display columns, so the escapes do not count.
    #[test]
    fn test_pad_cell_fits_the_column_around_ansi() {
        let styled = style("abc").red().force_styling(true).to_string();
        let padded = pad_cell(&styled, 6);
        assert_eq!(console::measure_text_width(&padded), 6);
        assert!(padded.starts_with(&styled));

        let long = style("abcdefghij").red().force_styling(true).to_string();
        let cut = pad_cell(&long, 6);
        assert_eq!(console::measure_text_width(&cut), 6);
        assert!(cut.contains('…'), "not truncated with an ellipsis: {cut:?}");
        assert!(cut.ends_with("\u{1b}[0m"), "lost its reset: {cut:?}");
    }

    /// The blank line that separates log messages from the bars appears the
    /// first time something is logged, and not before: a run that logs nothing
    /// should not gain a stray gap above its bars.
    #[test]
    fn test_log_padding_appears_with_the_first_message() {
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::hidden());
        let sink = LogSink {
            multi: multi.clone(),
            line_ending: "\n",
            padding: static_line(&multi),
        };
        assert_eq!(
            sink.padding.message(),
            "",
            "padded before anything was said"
        );

        sink.print("something happened");
        assert_eq!(sink.padding.message(), " ", "no padding after a message");

        // Still exactly one blank line, however much is logged.
        sink.print("and again");
        assert_eq!(sink.padding.message(), " ");
    }

    /// A hidden reporter must be safe to drive exactly like a live one.
    #[test]
    fn test_hidden_reporter_is_inert() {
        let reporter = ProgressReporter::hidden();
        let file = reporter.file(0);
        file.start("a.fastq");
        file.update(1000, || 50.0);
        file.stage("writing report");
        file.finish("a.fastq", 2000);
        file.fail();
        assert!(file.live_stats().is_none());
        reporter.finish(1);
    }
}
