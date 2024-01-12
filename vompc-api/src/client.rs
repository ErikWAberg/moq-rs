use serde::{Deserialize, Serialize};
use url::Url;

use crate::{ApiError};


#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    //programme_id:9000000/episode_version_id:066A/start

    channel_id: String,
    title_svt_id: String,
    #[serde(rename = "programmeId")]
    program_id: u32,
    episode: u32,
    start_delay_seconds: u32,
    duration: usize,
    encrypted: bool,
    sign_interpreted: bool,
    audio_described: bool
}

const DEFAULT_PROGRAM_ID: u32 = 9123400;
const DEFAULT_PROGRAM_ID_STR: &str = "9123400";
const DEFAULT_EPISODE_OFFSET: u32 = 10;

#[derive(Clone)]
pub struct Client {
    url: Url,
    client: reqwest::Client,
    episodes_offset: u32,
    episodes_created: u32,
}
impl Client {
    pub fn new(url: Url) -> Self {
        let client = reqwest::Client::new();
        Self { url, client, episodes_offset: DEFAULT_EPISODE_OFFSET, episodes_created: 0}
    }
    pub fn new_with_offset(url: Url, episodes_offset: u32) -> Self {
        let client = reqwest::Client::new();
        Self { url, client, episodes_offset, episodes_created: 0 }
    }

    pub async fn start(&mut self, episode_version_id: &str) -> Result<(), ApiError> {
        let url = self.url.join(DEFAULT_PROGRAM_ID_STR)?.join(episode_version_id)?;
        let rsp = self.client.get(url).send().await?;
        rsp.error_for_status()?;
        Ok(())
    }

    pub async fn start_auto(&mut self) -> Result<(), ApiError> {
        let episode = format!("{}", self.episodes_offset + self.episodes_created);
        let url = self.url.join(DEFAULT_PROGRAM_ID_STR)?.join(episode.as_str())?;
        let rsp = self.client.get(url).send().await?;
        rsp.error_for_status()?;
        Ok(())
    }

    pub async fn delete_auto(&mut self) -> Result<(), ApiError> {
        let episode = format!("{}", self.episodes_offset + self.episodes_created);
        let url = self.url.join(DEFAULT_PROGRAM_ID_STR)?.join(episode.as_str())?;
        let rsp = self.client.delete(url).send().await?;
        rsp.error_for_status()?;
        Ok(())
    }

    pub async fn create(&mut self, channel: &str, title_svt_id: &str, duration: usize) -> Result<(), ApiError> {
        let create_req = self.create_req(channel, title_svt_id, duration);
        let body = serde_json::to_string(&create_req).unwrap();
        let rsp = self.client.post(self.url.as_str()).body(body)
            .send().await?;
        rsp.error_for_status()?;
        Ok(())
    }

    fn create_req(&mut self, channel: &str, title_svt_id: &str, duration: usize) -> CreateRequest {
        self.episodes_created += 1;
        let episode = self.episodes_offset + self.episodes_created;
        CreateRequest {
            channel_id: channel.to_string(),
            title_svt_id: title_svt_id.to_string(),
            program_id: DEFAULT_PROGRAM_ID,
            episode,
            start_delay_seconds: 0,
            duration,
            encrypted: false,
            sign_interpreted: false,
            audio_described: false
        }
    }
}