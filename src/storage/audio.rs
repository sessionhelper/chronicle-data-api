use aws_sdk_s3::Client;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::ByteStream;
use serde::Serialize;

use crate::error::AppError;

#[derive(Debug, Serialize)]
pub struct ChunkInfo {
    pub key: String,
    pub seq: u32,
    pub size: i64,
}

/// Upload a PCM audio chunk to S3.
pub async fn upload_chunk(
    client: &Client,
    bucket: &str,
    s3_prefix: &str,
    pseudo_id: &str,
    seq: u32,
    body: Vec<u8>,
) -> Result<String, AppError> {
    let key = format!("{}/audio/{}/chunk_{:04}.pcm", s3_prefix, pseudo_id, seq);

    tracing::info!(bucket, key, size = body.len(), "uploading audio chunk to S3");

    client
        .put_object()
        .bucket(bucket)
        .key(&key)
        .body(ByteStream::from(body))
        .content_type("audio/pcm")
        .send()
        .await
        .map_err(|e| AppError::S3(format!("upload failed: {e}")))?;

    tracing::debug!(key, "chunk uploaded");
    Ok(key)
}

/// List all audio chunks for a speaker in a session.
pub async fn list_chunks(
    client: &Client,
    bucket: &str,
    s3_prefix: &str,
    pseudo_id: &str,
) -> Result<Vec<ChunkInfo>, AppError> {
    let prefix = format!("{}/audio/{}/", s3_prefix, pseudo_id);

    tracing::debug!(bucket, prefix, "listing audio chunks");

    let result = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(&prefix)
        .send()
        .await
        .map_err(|e| AppError::S3(format!("list failed: {e}")))?;

    let mut chunks: Vec<ChunkInfo> = result
        .contents()
        .iter()
        .filter_map(|obj| {
            let key = obj.key()?;
            let filename = key.rsplit('/').next()?;
            let seq_str = filename.strip_prefix("chunk_")?.strip_suffix(".pcm")?;
            let seq: u32 = seq_str.parse().ok()?;
            Some(ChunkInfo {
                key: key.to_string(),
                seq,
                size: obj.size.unwrap_or(0),
            })
        })
        .collect();

    chunks.sort_by_key(|c| c.seq);
    tracing::debug!(prefix, count = chunks.len(), "listed chunks");
    Ok(chunks)
}

/// Download a specific audio chunk.
pub async fn download_chunk(
    client: &Client,
    bucket: &str,
    s3_prefix: &str,
    pseudo_id: &str,
    seq: u32,
) -> Result<Vec<u8>, AppError> {
    let key = format!("{}/audio/{}/chunk_{:04}.pcm", s3_prefix, pseudo_id, seq);

    tracing::debug!(key, "downloading chunk");

    let result = client
        .get_object()
        .bucket(bucket)
        .key(&key)
        .send()
        .await
        .map_err(|e| {
            if matches!(e.as_service_error(), Some(GetObjectError::NoSuchKey(_))) {
                return AppError::NotFound(format!("chunk not found: {key}"));
            }
            AppError::S3(format!("download failed: {e}"))
        })?;

    let bytes = result
        .body
        .collect()
        .await
        .map_err(|e| AppError::S3(format!("read body failed: {e}")))?
        .into_bytes()
        .to_vec();

    tracing::debug!(key, size = bytes.len(), "chunk downloaded");
    Ok(bytes)
}

/// Delete all audio for a session.
pub async fn delete_session_audio(
    client: &Client,
    bucket: &str,
    s3_prefix: &str,
) -> Result<u32, AppError> {
    let prefix = format!("{}/audio/", s3_prefix);

    tracing::info!(prefix, "deleting session audio");

    let result = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(&prefix)
        .send()
        .await
        .map_err(|e| AppError::S3(format!("list for delete failed: {e}")))?;

    let mut deleted = 0u32;
    for obj in result.contents() {
        if let Some(key) = obj.key() {
            client
                .delete_object()
                .bucket(bucket)
                .key(key)
                .send()
                .await
                .map_err(|e| AppError::S3(format!("delete {key} failed: {e}")))?;
            deleted += 1;
        }
    }

    tracing::info!(deleted, "audio deleted");
    Ok(deleted)
}
