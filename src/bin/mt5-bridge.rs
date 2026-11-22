//! mt5-bridge operational CLI.
//!
//! Commands: `doctor`, `stats`, `tail`, `replay`, `wal verify`.
//!
//! This binary is built only with the `cli` feature; clap is a CLI-only
//! dependency and the library remains free of CLI dependencies.

use clap::Parser;
use mt5_bridge::{
    health::{doctor, resolve_dll_path, HealthReport, HealthStatus},
    Mt5Client,
};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "mt5-bridge")]
#[command(about = "mt5-bridge operational CLI")]
enum Cli {
    /// Run health and readiness checks: DLL, pipe, auth, protocol, account,
    /// reconciliation, WAL.
    Doctor(DoctorArgs),
    /// Print bridge statistics.
    Stats(StatsArgs),
    /// Stream trade/tick events (requires the `async` feature).
    Tail(TailArgs),
    /// Replay a captured event stream (requires the `async` feature).
    Replay(ReplayArgs),
    /// Verify a WAL store (requires the `wal` feature).
    Wal(WalArgs),
}

#[derive(Parser, Debug)]
struct DoctorArgs {
    /// Path to the bridge DLL (defaults to MT5_DLL_PATH or mt5_bridge.dll next
    /// to the executable).
    #[arg(long)]
    dll: Option<PathBuf>,
    /// Attempt to connect to MT5 for protocol, account, and stats checks.
    #[arg(long)]
    connect: bool,
    /// Path to the WAL store to verify.
    #[arg(long)]
    wal: Option<PathBuf>,
}

#[derive(Parser, Debug)]
struct StatsArgs {}

#[derive(Parser, Debug)]
struct TailArgs {
    /// Symbol to tail ticks for.
    #[arg(long)]
    symbol: Option<String>,
    /// Maximum number of events to print.
    #[arg(long, short = 'n')]
    count: Option<usize>,
}

#[derive(Parser, Debug)]
struct ReplayArgs {
    /// Capture file to replay.
    path: PathBuf,
}

#[derive(Parser, Debug)]
struct WalArgs {
    #[command(subcommand)]
    command: WalCommand,
}

#[derive(Parser, Debug)]
enum WalCommand {
    /// Verify a WAL store.
    Verify {
        /// Path to the WAL store.
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli {
        Cli::Doctor(args) => run_doctor(args),
        Cli::Stats(args) => run_stats(args),
        Cli::Tail(args) => run_tail(args),
        Cli::Replay(args) => run_replay(args),
        Cli::Wal(args) => run_wal(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn run_doctor(args: DoctorArgs) -> Result<(), Box<dyn std::error::Error>> {
    let dll_path = resolve_dll_path(args.dll.as_deref())
        .ok_or_else(|| "no bridge DLL found; pass --dll or set MT5_DLL_PATH".to_string())?;

    let manager = mt5_bridge::reconciliation::OrderManager::new(
        mt5_bridge::constants::EA_DEFAULT_STRATEGY_MAGIC,
    );

    let client = if args.connect {
        Some(Mt5Client::connect(0, "", "")?)
    } else {
        None
    };

    let report = doctor(&dll_path, client.as_ref(), &manager, args.wal.as_deref());
    print_report(&report);

    if report.is_healthy() {
        Ok(())
    } else {
        Err("doctor reports unhealthy status".into())
    }
}

fn run_stats(_args: StatsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let client = Mt5Client::connect(0, "", "")?;
    let stats = client.stats()?;
    println!("uptime_secs: {}", stats.uptime_secs);
    println!("requests_total: {}", stats.requests_total);
    println!("tick_drops: {}", stats.tick_drops);
    println!("trade_drops: {}", stats.trade_drops);
    println!("book_drops: {}", stats.book_drops);
    println!("bar_drops: {}", stats.bar_drops);
    println!("tick_queue_depth: {}", stats.tick_queue_depth);
    println!("trade_queue_depth: {}", stats.trade_queue_depth);
    println!("last_error: {}", stats.last_error);
    println!("protocol: v{}", stats.protocol);
    Ok(())
}

fn run_tail(args: TailArgs) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "async")]
    {
        let _ = args;
        Err("tail is not yet implemented against a live terminal".into())
    }
    #[cfg(not(feature = "async"))]
    {
        let _ = args;
        Err("tail requires the async feature".into())
    }
}

fn run_replay(args: ReplayArgs) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "async")]
    {
        let _ = args;
        Err("replay is not yet implemented against a live terminal".into())
    }
    #[cfg(not(feature = "async"))]
    {
        let _ = args;
        Err("replay requires the async feature".into())
    }
}

fn run_wal(args: WalArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        WalCommand::Verify { path } => {
            #[cfg(feature = "wal")]
            {
                mt5_bridge::wal::WalOrderStore::open(&path)?;
                println!("WAL at {} is valid", path.display());
                Ok(())
            }
            #[cfg(not(feature = "wal"))]
            {
                let _ = path;
                Err("wal verify requires the wal feature".into())
            }
        }
    }
}

fn print_report(report: &HealthReport) {
    for check in &report.checks {
        let status = match check.status {
            HealthStatus::Ok => "OK",
            HealthStatus::Warn => "WARN",
            HealthStatus::Fail => "FAIL",
        };
        println!("[{status}] {}: {}", check.name, check.detail);
    }
    let overall = if report.is_healthy() {
        "healthy"
    } else {
        "unhealthy"
    };
    println!("overall: {overall}");
}
