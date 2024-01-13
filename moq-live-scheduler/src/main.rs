mod live_sched;

use clap::Parser;
use crate::live_sched::{LiveScheduler, LiveSchedulerConfig};


#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
	env_logger::init();

	let config = LiveSchedulerConfig::parse();
	let server = LiveScheduler::new(config);
	server.run().await
}
