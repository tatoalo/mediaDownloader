use deadpool_redis::{Config, Pool, Runtime};
use redis::{
    AsyncCommands, ConnectionAddr, ConnectionInfo, RedisConnectionInfo, RedisError, SetExpiry,
    SetOptions,
};
use serde::Deserialize;
use std::fmt::{Debug, Formatter};
use tracing::{debug, error, instrument};

use crate::DEFAULT_REDIS_TTL;

#[derive(Debug, Deserialize)]
pub struct RedisConfig {
    pub username: String,
    pub password: String,
    pub channel: String,
    pub host: Option<String>,
    pub port: Option<u16>,
}

#[derive(Clone, Default)]
pub struct RedisBuilder {
    username: String,
    password: String,
    host: Option<String>,
    port: Option<u16>,
}

pub struct RedisManager {
    manager: Pool,
}

#[derive(Debug)]
pub struct MetadataArchive {
    pub values: Vec<RetrievedMetadata>,
}

#[derive(Debug)]
pub struct RetrievedMetadata {
    pub key: String,
    pub value: String,
    pub ttl: Option<i32>,
}

const DEFAULT_REDIS_HOST: &str = "localhost";
const DEFAULT_REDIS_PORT: u16 = 6379;
const MISSING_REDIS_KEY: &str = "MISSING_KEY";

pub trait Builder: Default {
    fn from_config(config: &RedisConfig) -> Self;
}

impl Debug for RedisBuilder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut ds = f.debug_struct("Builder");

        ds.field("username", &self.username);
        ds.field("password", &"<redacted>");
        ds.field(
            "host",
            &self.host.clone().unwrap_or(DEFAULT_REDIS_HOST.to_string()),
        );
        ds.field("port", &self.port.unwrap_or(DEFAULT_REDIS_PORT));
        ds.finish()
    }
}

impl RedisBuilder {
    pub fn username(&mut self, username: &str) -> &mut Self {
        self.username = username.to_string();
        self
    }

    pub fn password(&mut self, password: &str) -> &mut Self {
        self.password = password.to_string();
        self
    }

    pub fn host(&mut self, host: &str) -> &mut Self {
        self.host = Some(host.to_string());
        self
    }

    pub fn port(&mut self, port: u16) -> &mut Self {
        self.port = Some(port);
        self
    }
}

impl Builder for RedisBuilder {
    fn from_config(config: &RedisConfig) -> Self {
        let mut builder = RedisBuilder::default();

        let host = config
            .host
            .clone()
            .unwrap_or(DEFAULT_REDIS_HOST.to_string());
        let port = config.port.unwrap_or(DEFAULT_REDIS_PORT);

        builder
            .username(&config.username)
            .password(&config.password)
            .host(&host)
            .port(port);

        builder
    }
}

impl RedisManager {
    pub async fn new(builder: RedisBuilder) -> Result<RedisManager, RedisError> {
        let redis_db = 0;

        let redis_conn_info = RedisConnectionInfo {
            db: redis_db,
            username: Some(builder.username.clone()),
            password: Some(builder.password.clone()),
        };

        let tcp_tls_addr = ConnectionAddr::Tcp(builder.host.unwrap(), builder.port.unwrap());

        let conn_info = ConnectionInfo {
            addr: tcp_tls_addr,
            redis: redis_conn_info,
        };

        let cfg = Config::from_connection_info(conn_info);
        let pool = cfg.create_pool(Some(Runtime::Tokio1)).unwrap();

        debug!("Connected!");

        debug!("Pool status: {:?}", pool.status());

        Ok(Self { manager: pool })
    }

    pub async fn build(builder: RedisBuilder) -> Result<RedisManager, RedisError> {
        RedisManager::new(builder).await
    }

    pub async fn retrieve_connection(&self) -> Result<deadpool_redis::Connection, RedisError> {
        let manager = self.manager.clone();
        let conn = manager.get().await.unwrap();
        Ok(conn)
    }

    pub async fn get(&self, key: &str) -> Result<String, RedisError> {
        let mut conn = self.manager.get().await.unwrap();
        let result: String = conn.get(key).await?;
        Ok(result)
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<(), RedisError> {
        let opts = SetOptions::default().with_expiration(SetExpiry::EX(DEFAULT_REDIS_TTL));

        let mut conn = self.manager.get().await.unwrap();
        conn.set_options(key, value, opts).await?;
        Ok(())
    }

    pub async fn del(&self, key: &str) -> Result<(), RedisError> {
        let mut conn = self.manager.get().await.unwrap();
        conn.del(key).await?;
        Ok(())
    }

    pub async fn send_to_channel(&self, channel: &str, message: &str) -> Result<(), RedisError> {
        let mut conn = self.manager.get().await.unwrap();
        conn.publish(channel, message).await?;
        Ok(())
    }

    pub async fn flushdb(&self) -> Result<(), RedisError> {
        let mut conn = self.manager.get().await.unwrap();
        let _scan_result: redis::RedisResult<Vec<redis::Value>> =
            redis::cmd("FLUSHDB").query_async(&mut conn).await;
        Ok(())
    }

    #[instrument(level = "debug", name = "retrieve_metadata", skip_all)]
    pub async fn retrieve_metadata(&self) -> Result<MetadataArchive, RedisError> {
        let mut conn = self.manager.get().await.unwrap();
        let scan_result: redis::RedisResult<Vec<redis::Value>> =
            redis::cmd("SCAN").arg("0").query_async(&mut conn).await;

        let mut retrieved_metadata = MetadataArchive { values: Vec::new() };

        let bulk_values = &scan_result.unwrap()[1];

        debug!("Bulk values: {:?}", bulk_values);

        match bulk_values {
            redis::Value::Bulk(bulk_values) => {
                if bulk_values.is_empty() {
                    warn!("No keys retrieved!");
                }
                for value in bulk_values {
                    match value {
                        redis::Value::Data(data) => {
                            let key = std::str::from_utf8(data).unwrap_or(MISSING_REDIS_KEY);
                            if key == MISSING_REDIS_KEY {
                                warn!("Key is missing!");
                                continue;
                            }

                            let ttl: i32 = conn.ttl(key).await.unwrap();
                            let val: String = conn.get(key).await.unwrap();

                            if ttl != -1 {
                                debug!("Key: {:?} ~ Val: {:?}", key, val);
                                retrieved_metadata.values.push(RetrievedMetadata {
                                    key: key.to_string(),
                                    value: val,
                                    ttl: Some(ttl),
                                });
                            } else {
                                debug!("Key: {:?} ~ Val: {:?} ~ TTL: {:?}", key, val, ttl);
                                retrieved_metadata.values.push(RetrievedMetadata {
                                    key: key.to_string(),
                                    value: val,
                                    ttl: None,
                                });
                            }
                        }
                        _ => {
                            error!("NOT redis::Value::Data ~ {:?}", value);
                        }
                    }
                }
            }
            _ => {
                error!("Bulk values are NOT bulk, wtf bruh ~ {:?}", bulk_values);
                return Err(RedisError::from((
                    redis::ErrorKind::TypeError,
                    "Expected redis::Value::Bulk",
                )));
            }
        }
        Ok(retrieved_metadata)
    }
}
