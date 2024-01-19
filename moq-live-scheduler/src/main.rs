mod live_sched;
mod config;
mod subscriber;
mod catalog;

use clap::Parser;
use crate::config::Config;
use crate::live_sched::LiveScheduler;


#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
	env_logger::init();

	// Disable tracing so we don't get a bunch of Quinn spam.
	let tracer = tracing_subscriber::FmtSubscriber::builder()
		.with_max_level(tracing::Level::WARN)
		.finish();
	tracing::subscriber::set_global_default(tracer).unwrap();

	let config = Config::parse();


	let server = LiveScheduler::new(config);
	server.run().await
}
