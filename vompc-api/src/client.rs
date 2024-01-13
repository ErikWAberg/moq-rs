use std::fmt::format;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{ApiError};


#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    //programme_id:9000000/episode_version_id:066A/start

    channel: String,
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
        let dst = format!("{DEFAULT_PROGRAM_ID_STR}/{episode_version_id}/start");
        let url = self.url.join(dst.as_str())?;
        let rsp = self.client.get(url).send().await?;
        rsp.error_for_status()?;
        Ok(())
    }

    pub async fn start_auto(&mut self) -> Result<(), ApiError> {
        let episode_version_id = self.episodes_offset + self.episodes_created;
        let dst = format!("{DEFAULT_PROGRAM_ID_STR}/{episode_version_id}/start");
        let url = self.url.join(dst.as_str())?;
        let rsp = self.client.get(url).send().await?;
        rsp.error_for_status()?;
        println!("started vompc: {dst}");
        Ok(())
    }

    pub async fn stop_auto(&mut self) -> Result<(), ApiError> {
        let episode_version_id = self.episodes_offset + self.episodes_created;
        let dst = format!("{DEFAULT_PROGRAM_ID_STR}/{episode_version_id}/stop");
        let url = self.url.join(dst.as_str())?;
        let rsp = self.client.get(url).send().await?;
        rsp.error_for_status()?;
        println!("stopped vompc: {dst}");
        Ok(())
    }

    pub async fn delete_auto(&mut self) -> Result<(), ApiError> {
        let episode_version_id = self.episodes_offset + self.episodes_created;
        let dst = format!("{DEFAULT_PROGRAM_ID_STR}/{episode_version_id}/delete");
        let url = self.url.join(dst.as_str())?;
        let rsp = self.client.get(url).send().await?;
        rsp.error_for_status()?;

        println!("deleted vompc: {dst}");
        Ok(())
    }

    pub async fn create(&mut self, channel: &str, title_svt_id: &str, duration: usize) -> Result<String, ApiError> {
        let create_req = self.create_req(channel.to_string(), title_svt_id, duration);
        let url = self.url.join("create2")?;
        let rsp = self.client.post(url)
            .json(&create_req)
            .send()
            .await?
            .text()
            .await?;
        let resource = format!("{DEFAULT_PROGRAM_ID}/{}", self.episodes_offset + self.episodes_created);
        println!("created vompc: {:?}", create_req);

        Ok(resource)
    }

    fn create_req(&mut self, channel: String, title_svt_id: &str, duration: usize) -> CreateRequest {
        self.episodes_created += 1;
        let episode = self.episodes_offset + self.episodes_created;
        CreateRequest {
            channel,
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