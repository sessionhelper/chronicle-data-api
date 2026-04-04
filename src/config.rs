use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub s3_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_bucket: String,
    pub shared_secret: String,
    pub bind_addr: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .expect("DATABASE_URL must be set"),
            s3_endpoint: env::var("S3_ENDPOINT")
                .expect("S3_ENDPOINT must be set"),
            s3_access_key: env::var("S3_ACCESS_KEY")
                .expect("S3_ACCESS_KEY must be set"),
            s3_secret_key: env::var("S3_SECRET_KEY")
                .expect("S3_SECRET_KEY must be set"),
            s3_bucket: env::var("S3_BUCKET")
                .unwrap_or_else(|_| "ttrpg-dataset-raw".to_string()),
            shared_secret: env::var("SHARED_SECRET")
                .expect("SHARED_SECRET must be set"),
            bind_addr: env::var("BIND_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8001".to_string()),
        }
    }
}
