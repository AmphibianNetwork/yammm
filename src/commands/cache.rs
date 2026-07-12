use crate::app::AppContext;
use crate::output;
use crate::storage::CacheManager;
use anyhow::Result;
use clap::{Parser, Subcommand};

/// Shared "wipe the entire HTTP meta cache" path used when neither
/// `--stale` nor `--max-age` is specified on `cache clear-http-meta`.
fn clear_all_http_meta(
	http_cache: &crate::api::http_cache::HttpMetaCache
) -> Result<()> {
	match http_cache.clear() {
		Ok(()) => {
			if output::is_json_mode() {
				output::emit_json(&serde_json::json!({
					"command": "cache clear-http-meta",
					"status": "cleared",
				}))?;
			} else {
				output::success(
					"HTTP metadata cache cleared. Next API metadata \
					 fetch will re-validate from upstream.",
				);
			}
			Ok(())
		}
		Err(e) => Err(crate::errors::YammmError::general(format!(
			"Failed to clear HTTP metadata cache: {}",
			e
		))
		.into()),
	}
}

/// Inspect or manage the local file cache.
#[derive(Parser, Debug)]
pub struct CacheCommand {
	#[command(subcommand)]
	pub command: CacheSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum CacheSubcommand {
	Status,
	Clean,
	Obliterate,
	/// Wipe the HTTP metadata cache (ETag entries used to short-circuit
	/// Modrinth/CurseForge metadata fetches). Useful when an upstream
	/// API misreports `304 Not Modified` or you want to force a fresh
	/// metadata roundtrip without touching the JAR cache.
	ClearHttpMeta {
		/// Only remove entries past the default max-age window (24h),
		/// leaving fresh ones in place. Use this for routine maintenance;
		/// omit it to wipe everything. Implied by `--max-age`.
		#[arg(long)]
		stale: bool,

		/// Only remove entries older than this duration. Accepts a
		/// positive integer with optional `s`/`m`/`h`/`d` suffix
		/// (e.g. `30s`, `5m`, `2h`, `7d`). Overrides the default
		/// 24-hour window used by `--stale`.
		#[arg(long, value_name = "DURATION")]
		max_age: Option<String>,
	},
}

impl CacheCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		tracing::debug!("CacheCommand running");

		let cache_mgr = CacheManager::new(ctx.cache_dir().to_path_buf());
		cache_mgr.init()?;

		match self.command {
			CacheSubcommand::Status => {
				let status = cache_mgr.status()?;
				let http_meta = crate::api::http_cache::HttpMetaCache::shared()
					.stats()
					.unwrap_or_default();
				let total_size = status.total_size() + http_meta.total_bytes;
				let total_files = status.total_files() + http_meta.count;

				if output::is_json_mode() {
					output::emit_json(&serde_json::json!({
						"root": cache_mgr.cache_root().display().to_string(),
						"jars": {
							"file_count": status.jars.file_count,
							"total_size": status.jars.total_size,
						},
						"minecraft": {
							"file_count": status.minecraft.file_count,
							"total_size": status.minecraft.total_size,
						},
						"loaders": {
							"file_count": status.loaders.file_count,
							"total_size": status.loaders.total_size,
						},
						"http_meta": {
							"file_count": http_meta.count,
							"total_size": http_meta.total_bytes,
						},
						"total": {
							"file_count": total_files,
							"total_size": total_size,
						},
					}))?;
					return Ok(());
				}

				output::heading("Cache Status");
				output::bullet(format!(
					"Root: {}",
					cache_mgr.cache_root().display()
				));
				output::blank_line();

				output::bullet(format!(
					"jars/: {} files, {}",
					status.jars.file_count,
					crate::utils::format_size(status.jars.total_size)
				));
				output::bullet(format!(
					"minecraft/: {} files, {}",
					status.minecraft.file_count,
					crate::utils::format_size(status.minecraft.total_size)
				));
				output::bullet(format!(
					"loaders/: {} files, {}",
					status.loaders.file_count,
					crate::utils::format_size(status.loaders.total_size)
				));
				output::bullet(format!(
					"http-meta/: {} files, {}",
					http_meta.count,
					crate::utils::format_size(http_meta.total_bytes)
				));

				output::blank_line();
				output::bullet(format!(
					"Total: {} files, {}",
					total_files,
					crate::utils::format_size(total_size)
				));
			}
			CacheSubcommand::Clean => {
				output::require_json_support("cache clean")?;
				output::info("Cleaning oldest files from cache...");
				let max_size_bytes =
					ctx.global().cache_max_size_mb.unwrap_or(5000)
						* 1024 * 1024;
				let removed = cache_mgr.clean(max_size_bytes)?;
				if removed > 0 {
					output::success(format!(
						"Cleaned. Freed {}.",
						crate::utils::format_size(removed)
					));
				} else {
					output::success(
						"Cache is within threshold. Nothing to clean.",
					);
				}
			}
			CacheSubcommand::Obliterate => {
				output::require_json_support("cache obliterate")?;
				output::warning(
					"This will remove ALL cached files (JARs, Minecraft, loaders)!",
				);
				let proceed = dialoguer::Confirm::new()
					.with_prompt(
						"Are you sure you want to completely obliterate the cache?",
					)
					.default(false)
					.interact()?;

				if proceed {
					cache_mgr.obliterate()?;
					output::success("Cache completely obliterated.");
				} else {
					output::cancelled("Cache obliteration");
				}
			}
			CacheSubcommand::ClearHttpMeta { stale, max_age } => {
				let http_cache =
					crate::api::http_cache::HttpMetaCache::shared();
				// --max-age implies stale-only semantics with a custom
				// threshold. --stale alone uses the default 24h. Neither
				// flag set means wipe everything.
				let threshold_secs = match max_age.as_deref() {
					Some(s) => Some(
						crate::api::http_cache::parse_duration_secs(s)
							.map_err(|e| {
								crate::errors::YammmError::invalid_args(
									format!("invalid --max-age '{}': {}", s, e),
								)
							})?,
					),
					None if stale => None, // sentinel: use default
					None => return clear_all_http_meta(http_cache),
				};

				let result = match threshold_secs {
					Some(secs) => http_cache.clear_older_than(secs),
					None => http_cache.clear_stale(),
				};
				let threshold_label = match threshold_secs {
					Some(secs) => format!("older than {}s", secs),
					None => "stale (default 24h)".to_string(),
				};
				match result {
					Ok(removed) => {
						if output::is_json_mode() {
							output::emit_json(&serde_json::json!({
								"command": "cache clear-http-meta",
								"status": "pruned",
								"threshold_secs": threshold_secs.unwrap_or(crate::api::http_cache::default_max_age_secs()),
								"removed": removed,
							}))?;
						} else {
							output::success(format!(
								"Pruned {} HTTP metadata entr{} ({}).",
								removed,
								if removed == 1 { "y" } else { "ies" },
								threshold_label
							));
						}
					}
					Err(e) => {
						return Err(crate::errors::YammmError::general(
							format!("Failed to prune HTTP metadata: {}", e),
						)
						.into());
					}
				}
			}
		}

		Ok(())
	}
}
