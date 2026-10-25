//! Progress on stderr while something downloads and extracts.

use std::io::{IsTerminal, Write};

use protonhub_core::install::Step;

/// Progress on stderr: a line per step, redrawn in place on a terminal.
struct Progress {
    tty: bool,
    label: &'static str,
    percent: Option<u64>,
}

impl Progress {
    fn new() -> Self {
        Self {
            tty: std::io::stderr().is_terminal(),
            label: "",
            percent: None,
        }
    }

    fn step(&mut self, step: Step) {
        let (label, fraction) = match step {
            Step::Resolve => ("resolving", None),
            Step::Download { done, total } => ("downloading", Some((done, total))),
            Step::Verify => ("verifying", None),
            Step::Extract { done, total } => ("extracting", Some((done, total))),
            // Installs print none.
            Step::Output(_) => return,
        };
        // Of an unknown total (0), only the MiB so far.
        let percent = fraction.map(|(done, total)| match total {
            0 => done >> 20,
            _ => done * 100 / total,
        });
        if label == self.label && (percent == self.percent || !self.tty) {
            return;
        }
        let mut err = std::io::stderr().lock();
        if label != self.label && !self.label.is_empty() && self.tty {
            let _ = writeln!(err);
        }
        // Off a terminal each step is one line, so a percentage would be
        // stuck at its first value.
        let line = match (percent, fraction) {
            (_, Some((done, 0))) if self.tty => format!("{label} {}", mib(done)),
            (_, Some((_, 0))) | (_, None) => label.to_owned(),
            (Some(p), Some((_, total))) if self.tty => format!("{label} {p:>3}% of {}", mib(total)),
            (_, Some((_, total))) => format!("{label} {}", mib(total)),
        };
        let _ = if self.tty {
            write!(err, "\r{line}")
        } else {
            writeln!(err, "{line}")
        };
        self.label = label;
        self.percent = percent;
    }

    fn finish(&self) {
        if self.tty && !self.label.is_empty() {
            eprintln!();
        }
    }
}

/// Runs `work` with its progress on stderr.
pub fn reported<T>(work: impl FnOnce(&mut dyn FnMut(Step)) -> T) -> T {
    let mut report = Progress::new();
    let result = work(&mut |step| report.step(step));
    report.finish();
    result
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}
