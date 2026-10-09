//! Debug traces (feature `traces`, never in packages): `--trace kind[=file]`, a text file per kind.
use core::fmt;
use core::str::FromStr;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Lines wait in memory at most about this long.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraceKind {
    /// Input and position per tick: the own bean's (client), every pawn's (server); `cargo xtask stress` reads them.
    Input,
    /// Contacts, tackles and dives, with where each side saw the others.
    Hits,
    /// Every left click through window, picking, button and action, with a verdict (client).
    Clicks,
}

impl TraceKind {
    const ALL: [TraceKind; 3] = [TraceKind::Input, TraceKind::Hits, TraceKind::Clicks];

    pub fn name(self) -> &'static str {
        match self {
            TraceKind::Input => "input",
            TraceKind::Hits => "hits",
            TraceKind::Clicks => "clicks",
        }
    }
}

/// One `--trace` value.
#[derive(Clone, Debug)]
pub struct TraceArg {
    pub kind: TraceKind,
    pub file: Option<PathBuf>,
}

impl FromStr for TraceArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let (name, file) = s
            .split_once('=')
            .map_or((s, None), |(n, f)| (n, Some(PathBuf::from(f))));
        let kind = TraceKind::ALL.into_iter().find(|k| k.name() == name).ok_or_else(|| {
            let names: Vec<&str> = TraceKind::ALL.iter().map(|k| k.name()).collect();
            format!("no trace `{name}` (there are {})", names.join(", "))
        })?;
        Ok(TraceArg { kind, file })
    }
}

#[derive(clap::Args, Clone, Debug, Default)]
pub struct TraceOpts {
    /// Debug traces into text files: `input`, `hits`, `clicks` (the client's), each into `<kind>-<side>.txt` here
    /// or `kind=file`, e.g. `--trace hits,input=in.txt`.
    #[arg(long = "trace", value_name = "KIND[=FILE]", value_delimiter = ',')]
    pub trace: Vec<TraceArg>,
}

impl TraceOpts {
    pub fn on(&self, kind: TraceKind) -> bool {
        self.trace.iter().any(|a| a.kind == kind)
    }

    /// The file of `kind` if traced (`side` names the default one); panics when it cannot be created.
    #[expect(clippy::panic, reason = "a trace that cannot be written ends the run")]
    pub fn open(&self, kind: TraceKind, side: &str) -> Option<TraceFile> {
        let arg = self.trace.iter().rev().find(|a| a.kind == kind)?;
        let path = arg
            .file
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("{}-{side}.txt", kind.name())));
        let file = File::create(&path).unwrap_or_else(|e| panic!("--trace {}: {}: {e}", kind.name(), path.display()));
        Some(TraceFile {
            out: BufWriter::new(file),
            flushed: Instant::now(),
        })
    }
}

/// A trace's file, buffered; the lines reach it within FLUSH_EVERY and when it is dropped.
pub struct TraceFile {
    out: BufWriter<File>,
    flushed: Instant,
}

impl TraceFile {
    pub fn line(&mut self, l: fmt::Arguments) {
        let _ = self.out.write_fmt(l);
        let _ = self.out.write_all(b"\n");
        if self.flushed.elapsed() >= FLUSH_EVERY {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        self.flushed = Instant::now();
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_is_a_kind_and_maybe_a_file() {
        let a: TraceArg = "hits".parse().unwrap();
        assert_eq!((a.kind, a.file), (TraceKind::Hits, None));
        let a: TraceArg = "input=x/in.txt".parse().unwrap();
        assert_eq!((a.kind, a.file), (TraceKind::Input, Some(PathBuf::from("x/in.txt"))));
        assert!("hit".parse::<TraceArg>().unwrap_err().contains("input, hits, clicks"));
    }
}
