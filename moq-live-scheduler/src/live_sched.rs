use std::net;
use clap::Parser;

/// Runs a HTTP API to create/get origins for broadcasts.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct LiveSchedulerConfig {
    /// Connect to the given redis instance
    #[arg(long)]
    pub redis: url::Url,
}

pub struct LiveScheduler {
    config: LiveSchedulerConfig,
}

impl LiveScheduler {
    pub fn new(config: LiveSchedulerConfig) -> Self {
        Self { config }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        log::info!("connecting to redis: url={}", self.config.redis);

        // Create the redis client.
        let redis = redis::Client::open(self.config.redis)?;
        let mut con = redis.get_connection()?;
        let mut pubsub = con.as_pubsub();
        pubsub.subscribe("event-starts")?;

        //we dont know when events end


        loop {
            let msg = pubsub.get_message()?;
            let payload : String = msg.get_payload()?;
            log::info!("channel '{}': {}", msg.get_channel_name(), payload);
            // channel 'event-starts': {"url":"http://localhost:4443/e7a2ff34-13d4-4c3d-af7e-5a662a622b57"}

            // TODO move call of vompc here
            // TODO start moq-sub
            // TODO start event recorder
        }

    }
}