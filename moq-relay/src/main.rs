use anyhow::Context;
use clap::Parser;
use log::info;
use tokio::process::Command;

mod config;
mod error;
mod origin;
mod quic;
mod session;
mod tls;
mod web;

pub use config::*;
pub use error::*;
pub use origin::*;
pub use quic::*;
pub use session::*;
pub use tls::*;
pub use web::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	env_logger::init();

	// Disable tracing so we don't get a bunch of Quinn spam.
	let tracer = tracing_subscriber::FmtSubscriber::builder()
		.with_max_level(tracing::Level::WARN)
		.finish();
	tracing::subscriber::set_global_default(tracer).unwrap();

	let config = Config::parse();
	let tls = Tls::load(&config)?;
	let handle = tokio::spawn(async move {

		let parent_pid = std::process::id();
		loop {
			// Use the ps command to get defunct processes
			let output = Command::new("ps")
				.arg("-eo")
				.arg("ppid,pid,stat")
				.output().await
				.expect("Failed to execute ps command");

			if let Ok(s) = String::from_utf8(output.stdout) {
				for line in s.lines() {
					let parts: Vec<&str> = line.split_whitespace().collect();
					if parts.len() == 3 && parts[1] == parent_pid.to_string() && parts[2].contains('Z') {
						// Found a zombie process, now kill it
						let pid = parts[0];
						info!("killing zombie process {}", pid);
						Command::new("kill")
							.arg("-9")
							.arg(pid)
							.output()
							.await
							.expect("Failed to kill zombie process");
					}
				}
			}

			// Sleep for a specified duration before checking again
			tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
		}
	});
	// Create a QUIC server for media.
	let quic = Quic::new(config.clone(), tls.clone())
		.await
		.context("failed to create server")?;

	// Create the web server if the --dev flag was set.
	// This is currently only useful in local development so it's not enabled by default.
	if config.dev {
		let web = Web::new(config, tls);

		// Unfortunately we can't use preconditions because Tokio still executes the branch; just ignore the result
		tokio::select! {
			res = quic.serve() => res.context("failed to run quic server"),
			res = handle => res.context("failed to run reaper"),
			res = web.serve() => res.context("failed to run web server"),
		}
	} else {
		quic.serve().await.context("failed to run quic server")
	}
}
