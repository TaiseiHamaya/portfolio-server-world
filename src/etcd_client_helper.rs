#![allow(dead_code)]

use std::{
    env,
    net::{Ipv4Addr, SocketAddr},
};

use etcd_client::{KeyValue, PutOptions};

pub async fn create_etcd_client() -> etcd_client::Client {
    let etcd_endpoints = [format!(
        "http://{}:2379",
        env::var("ETCD_ADDR").unwrap_or(Ipv4Addr::LOCALHOST.to_string())
    )];

    log::info!(
        "Initializing etcd client with endpoints: {:?}",
        etcd_endpoints
    );

    let etcd_client = etcd_client::Client::connect(etcd_endpoints, None)
        .await
        .expect("Failed to connect to etcd cluster");

    log::info!("Successfully connected to etcd cluster.");

    etcd_client
}

pub async fn register_service_endpoint(
    mut etcd_client: etcd_client::Client,
    endpoint: SocketAddr,
    key: String,
) -> tokio::task::JoinHandle<()> {
    log::info!(
        "Registering service endpoint in etcd with key: {}, endpoint: {}",
        key,
        endpoint
    );

    let lease = etcd_client
        .lease_grant(5, None)
        .await
        .expect("Failed to create etcd lease");
    let option = PutOptions::new().with_lease(lease.id());
    etcd_client
        .put(key, endpoint.to_string(), Some(option))
        .await
        .expect("Failed to register service endpoint in etcd");

    let mut etcd_client_checker = etcd_client.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if let Err(e) = etcd_client_checker.lease_keep_alive(lease.id()).await {
                log::error!("Failed to keep etcd lease alive: {}", e);
                break;
            }
        }
    })
}

pub async fn get_existing_service_endpoint(
    mut etcd_client: etcd_client::Client,
    key: String,
) -> Option<KeyValue> {
    match etcd_client.get(key.as_str(), None).await {
        Ok(response) => match response.kvs().first() {
            Some(kv) => {
                log::info!(
                    "Existing service endpoint found in etcd: key={}, value={}",
                    String::from_utf8_lossy(kv.key()),
                    String::from_utf8_lossy(kv.value())
                );
                Some(kv.clone())
            }
            None => {
                log::info!(
                    "No existing service endpoint found in etcd for key: {}",
                    key
                );
                None
            }
        },
        Err(e) => {
            log::error!("Failed to get existing service endpoint from etcd: {}", e);
            None
        }
    }
}

pub async fn get_existing_service_endpoint_prefix(
    mut etcd_client: etcd_client::Client,
    prefix: Option<String>,
    handler: impl Fn(String, String) + Send + 'static,
) {
    let option = if prefix.is_some() {
        Some(etcd_client::GetOptions::new().with_prefix())
    } else {
        None
    };

    match etcd_client
        .get(prefix.unwrap_or_else(|| "".into()), option)
        .await
    {
        Ok(response) => {
            for kv in response.kvs() {
                handler(
                    String::from_utf8_lossy(kv.key()).into_owned(),
                    String::from_utf8_lossy(kv.value()).into_owned(),
                );
            }
        }
        Err(e) => log::error!("Failed to get existing service endpoints from etcd: {}", e),
    }
}

pub async fn watch_changes(
    mut etcd_client: etcd_client::Client,
    prefix: String,
    handler: impl Fn(&etcd_client::Event) + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let watch_option = etcd_client::WatchOptions::new().with_prefix();
    let mut watch_stream = etcd_client
        .watch(prefix.as_str(), Some(watch_option))
        .await
        .expect("Failed to start watching etcd for zone changes");

    tokio::spawn(async move {
        while let Some(response) = watch_stream
            .message()
            .await
            .expect("Failed to receive watch message")
        {
            for event in response.events() {
                log::info!(
                    "Received etcd event: type={:?}, key={:?}, value={:?}",
                    event.event_type(),
                    event.kv().map(|kv| String::from_utf8_lossy(kv.key())),
                    event.kv().map(|kv| String::from_utf8_lossy(kv.value()))
                );
                handler(&event);
            }
        }
    })
}
