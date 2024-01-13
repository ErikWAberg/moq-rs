use anyhow::Context;

use moq_transport::cache::{segment, track};
use crate::catalog::Catalog;

pub async fn get_catalog(track: &mut track::Subscriber) -> anyhow::Result<Catalog> {
    let segment = get_segment(track).await?;
    let catalog = Catalog::from_slice(segment.as_slice()).context("failed to parse catalog")?;
    Ok(catalog)
}

pub async fn get_segment(track: &mut track::Subscriber) -> anyhow::Result<Vec<u8>> {
    return if let Some(segment) = track.segment().await.context("failed to get segment")? {
        Ok(recv_segment(segment).await?)
    } else {
        Err(anyhow::anyhow!("not implemented"))
    };
}

async fn recv_segment(mut segment: segment::Subscriber) -> anyhow::Result<Vec<u8>> {
    let mut base = Vec::new();
    while let Some(mut fragment) = segment.fragment().await? {
        log::debug!("next fragment: {:?}", fragment);
        while let Some(data) = fragment.chunk().await? {
            base.extend_from_slice(&data);
        }
    }
    Ok(base)
}