use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    routing::get,
    Router,
};
use bytes::Bytes;
use lru::LruCache;
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use std::{
    convert::TryInto,
    hash::{DefaultHasher, Hash, Hasher},
    num::NonZeroUsize,
    sync::Arc,
};
use tokio::{net::TcpListener, sync::Mutex};
use tracing::{info, instrument};

// 引入 protobuf 生成的代码，我们暂且不用太关心他们
mod pb;

use pb::*;

// 参数使用 serde 做 Deserialize，axum 会自动识别并解析
#[derive(Deserialize)]
struct Params {
    spec: String,
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化 tracing
    tracing_subscriber::fmt::init();
    let cache: Cache = Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(1024).unwrap())));
    // 构建路由
    let app = Router::new()
        .route("/image/:spec/:url", get(generate))
        .with_state(cache);

    // 运行 web 服务器
    let addr = "127.0.0.1:3000";
    tracing::debug!("listening on {}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}

// 目前我们就只把参数解析出来
async fn generate(
    Path(Params { spec, url }): Path<Params>,
    State(cache): State<Cache>,
) -> Result<(HeaderMap, Vec<u8>), StatusCode> {
    let url = percent_decode_str(&url).decode_utf8_lossy();
    let _spec: ImageSpec = spec
        .as_str()
        .try_into()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    // Ok(format!("url: {}\n spec: {:#?}", url, spec))

    let url: &str = &percent_decode_str(&url).decode_utf8_lossy();
    let data = retrieve_image(url, cache)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("image/jpeg"));
    Ok((headers, data.to_vec()))
}

type Cache = Arc<Mutex<LruCache<u64, Bytes>>>;

#[instrument(level = "info", skip(cache))]
async fn retrieve_image(url: &str, cache: Cache) -> Result<Bytes> {
    let mut hasher = DefaultHasher::new();
    // 将str的hash值计算出来
    url.hash(&mut hasher);
    // 获取最终的hash值
    let key = hasher.finish();

    let g = &mut cache.lock().await;

    let data = match g.get(&key) {
        Some(v) => {
            info!("Match cache {}", key);
            v.to_owned()
        }
        None => {
            info!("Fetch url {}", url);
            let resp = reqwest::get(url).await?;
            let data = resp.bytes().await?;
            info!("Put cache {}", key);
            g.put(key, data.clone());
            data
        }
    };
    Ok(data)
}
