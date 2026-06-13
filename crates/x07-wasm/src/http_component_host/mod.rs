use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt as _, Limited};

pub async fn collect_body_with_limit<B>(body: B, max_bytes: usize) -> Result<Vec<u8>>
where
    B: http_body::Body<Data = Bytes> + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let collected = Limited::new(body, max_bytes)
        .collect()
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok(collected.to_bytes().to_vec())
}
