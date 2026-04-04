use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;

use crate::error::AppError;

/// Write a JSON metadata file to S3 at the session prefix.
pub async fn write_metadata(
    client: &Client,
    bucket: &str,
    s3_prefix: &str,
    filename: &str,
    data: &serde_json::Value,
) -> Result<(), AppError> {
    let key = format!("{}/{}", s3_prefix, filename);
    let body = serde_json::to_vec_pretty(data)
        .map_err(|e| AppError::Internal(format!("json serialize: {e}")))?;

    client
        .put_object()
        .bucket(bucket)
        .key(&key)
        .body(ByteStream::from(body))
        .content_type("application/json")
        .send()
        .await
        .map_err(|e| AppError::S3(format!("write metadata failed: {e}")))?;

    Ok(())
}

/// Read a JSON metadata file from S3.
pub async fn read_metadata(
    client: &Client,
    bucket: &str,
    s3_prefix: &str,
    filename: &str,
) -> Result<serde_json::Value, AppError> {
    let key = format!("{}/{}", s3_prefix, filename);

    let result = client
        .get_object()
        .bucket(bucket)
        .key(&key)
        .send()
        .await
        .map_err(|e| AppError::S3(format!("read metadata failed: {e}")))?;

    let bytes = result
        .body
        .collect()
        .await
        .map_err(|e| AppError::S3(format!("read body failed: {e}")))?
        .into_bytes();

    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Internal(format!("json parse: {e}")))?;

    Ok(value)
}
