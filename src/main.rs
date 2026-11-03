#![forbid(unsafe_code)]

use chain_lens::{analyze_reader, Thresholds};
use clap::Parser;
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "chain-lens",
    version,
    about = "Diagnose convergence across JSONL sampling chains"
)]
struct Cli {
    /// JSONL input path. Reads standard input when omitted.
    #[arg(value_name = "INPUT")]
    input: Option<PathBuf>,

    /// Maximum accepted rank-normalized split-R-hat.
    #[arg(long, default_value_t = 1.01)]
    max_rhat: f64,

    /// Minimum accepted bulk and tail effective sample size.
    #[arg(long, default_value_t = 100.0)]
    min_ess: f64,

    /// Pretty-print the JSON report.
    #[arg(long)]
    pretty: bool,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("chain-lens: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let thresholds = Thresholds {
        max_rhat: cli.max_rhat,
        min_ess: cli.min_ess,
    };
    let report = if let Some(path) = cli.input {
        let file = File::open(path)?;
        analyze_reader(BufReader::new(file), thresholds)?
    } else {
        let stdin = io::stdin();
        analyze_reader(stdin.lock(), thresholds)?
    };

    let stdout = io::stdout();
    let mut output = stdout.lock();
    if cli.pretty {
        serde_json::to_writer_pretty(&mut output, &report)?;
    } else {
        serde_json::to_writer(&mut output, &report)?;
    }
    writeln!(output)?;

    Ok(if report.converged {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}
