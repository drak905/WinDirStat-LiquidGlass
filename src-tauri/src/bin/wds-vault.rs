use std::path::PathBuf;
use std::process::ExitCode;

use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use windirstat_liquid_glass::vault::{
    default_manifest_db_path, RollbackBatchResult, VaultError, VaultService,
};

#[derive(Debug, Parser)]
#[command(
    name = "wds-vault",
    version,
    about = "WinDirStat 7-Day Quarantine Vault standalone emergency recovery CLI"
)]
struct Cli {
    /// Path to vault_manifest.db (defaults to standard %LOCALAPPDATA%\\WinDirStat\\vault_manifest.db or $WDS_VAULT_DB)
    #[arg(long, global = true, value_name = "PATH")]
    db: Option<PathBuf>,

    /// Emit machine-readable JSON output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// List quarantined batches and items from the last N days
    List(ListArgs),
    /// Roll back quarantined files by batch ID, item ID, or all recent batches
    Rollback(RollbackArgs),
}

#[derive(Debug, Args)]
struct ListArgs {
    /// Lookback window (e.g. 7d, 14d, 24h, or integer days)
    #[arg(long, default_value = "7d", value_name = "WINDOW")]
    last: String,
}

#[derive(Debug, Args)]
#[command(group(
    clap::ArgGroup::new("target")
        .required(true)
        .args(["batch", "item", "all_recent"])
))]
struct RollbackArgs {
    /// Roll back all quarantined items belonging to this batch UUID
    #[arg(long, value_name = "BATCH_ID")]
    batch: Option<String>,

    /// Roll back a single quarantined item UUID
    #[arg(long, value_name = "ITEM_ID")]
    item: Option<String>,

    /// Roll back all quarantined items from the recent lookback window (default 7d)
    #[arg(long)]
    all_recent: bool,

    /// Lookback window used with --all-recent (e.g. 7d)
    #[arg(long, default_value = "7d", value_name = "WINDOW")]
    last: String,
}

fn parse_window_days(raw: &str) -> Result<i64, VaultError> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(VaultError::InvalidArgument(
            "time window cannot be empty".to_string(),
        ));
    }
    if let Some(days_str) = trimmed.strip_suffix('d') {
        let days: i64 = days_str.parse().map_err(|_| {
            VaultError::InvalidArgument(format!("invalid day window '{raw}'"))
        })?;
        if days < 0 {
            return Err(VaultError::InvalidArgument(format!(
                "window cannot be negative: '{raw}'"
            )));
        }
        return Ok(days);
    }
    if let Some(hours_str) = trimmed.strip_suffix('h') {
        let hours: u64 = hours_str.parse().map_err(|_| {
            VaultError::InvalidArgument(format!("invalid hour window '{raw}'"))
        })?;
        let days = i64::try_from(hours.div_ceil(24)).map_err(|_| {
            VaultError::InvalidArgument(format!("hour window out of range: '{raw}'"))
        })?;
        return Ok(days);
    }
    let days: i64 = trimmed.parse().map_err(|_| {
        VaultError::InvalidArgument(format!(
            "invalid window '{raw}' (expected e.g. 7d, 24h, or integer days)"
        ))
    })?;
    if days < 0 {
        return Err(VaultError::InvalidArgument(format!(
            "window cannot be negative: '{raw}'"
        )));
    }
    Ok(days)
}

fn run(cli: Cli) -> Result<(), VaultError> {
    let db_path = cli.db.unwrap_or_else(default_manifest_db_path);

    match cli.command {
        Commands::List(args) => {
            let days = parse_window_days(&args.last)?;
            let now = Utc::now();
            let (batches, items) = if db_path.exists() {
                let service = VaultService::with_db_path(&db_path)?;
                (
                    service.list_batches_last_days(days, now)?,
                    service.list_items_last_days(days, now)?,
                )
            } else {
                (Vec::new(), Vec::new())
            };

            if cli.json {
                let payload = serde_json::json!({
                    "manifest_db": db_path,
                    "window_days": days,
                    "batches": batches,
                    "items": items,
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_default()
                );
                return Ok(());
            }

            println!("Manifest: {}", db_path.display());
            println!(
                "Batches (last {}d): {} | Items: {}",
                days,
                batches.len(),
                items.len()
            );
            for b in &batches {
                println!(
                    "BATCH {} | provider={} | quarantined={} | expires={} | total={} (Q:{} R:{} P:{}) | bytes={} | reason={}",
                    b.batch_id,
                    b.agent_cli_provider,
                    b.quarantined_at_utc,
                    b.expires_at_utc,
                    b.total_items,
                    b.quarantined_count,
                    b.restored_count,
                    b.purged_count,
                    b.total_logical_bytes,
                    b.reason
                );
            }
            for item in &items {
                println!(
                    "  ITEM {} | batch={} | status={} | bytes={} | original={} | vault={}",
                    item.item_id,
                    item.batch_id,
                    item.status,
                    item.logical_size_bytes,
                    item.original_full_path,
                    item.vault_storage_path
                );
            }
        }
        Commands::Rollback(args) => {
            if !db_path.exists() {
                return Err(VaultError::ItemNotFound {
                    path: db_path.display().to_string(),
                });
            }
            let service = VaultService::with_db_path(&db_path)?;
            if let Some(batch_id) = args.batch {
                let res = service.rollback_batch(&batch_id)?;
                print_rollback_result(&res, cli.json);
            } else if let Some(item_id) = args.item {
                let item = service.rollback_item(&item_id)?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&item).unwrap_or_default()
                    );
                } else {
                    println!(
                        "RESTORED item {} -> {} (xxhash64={}, sha256={})",
                        item.item_id,
                        item.original_full_path,
                        item.xxhash64_checksum,
                        item.sha256_checksum
                    );
                }
            } else if args.all_recent {
                let days = parse_window_days(&args.last)?;
                let res = service.rollback_all_recent(days)?;
                print_rollback_result(&res, cli.json);
            }
        }
    }

    Ok(())
}

fn print_rollback_result(res: &RollbackBatchResult, as_json: bool) {
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(res).unwrap_or_default()
        );
        return;
    }
    println!(
        "Rollback complete: restored={}, skipped_already_restored={}, failed_purged={}",
        res.restored.len(),
        res.skipped_already_restored.len(),
        res.failed_purged.len()
    );
    for item in &res.restored {
        println!(
            "  RESTORED {} -> {} ({} bytes)",
            item.item_id, item.original_full_path, item.logical_size_bytes
        );
    }
    for purged_id in &res.failed_purged {
        eprintln!("  SKIPPED_PURGED {purged_id} (already expired and unlinked)");
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("wds-vault error: {err}");
            ExitCode::FAILURE
        }
    }
}
