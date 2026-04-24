use dashmap::DashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use std::{env, u64};
use tonic::transport::{Server, channel};

mod ec2_helper;
mod etcd_client_helper;
mod generated;
mod logger;
mod proto_service;

use crate::generated::{
    proto_client::record_player_db_service_client::RecordPlayerDbServiceClient,
    proto_inner::{
        lobby_service_server::LobbyServiceServer, world_command_client::WorldCommandClient,
        world_command_server::WorldCommandServer,
    },
};
use crate::proto_service::{
    lobby_service::LobbyServiceImpl, world_command_service::WorldCommandServiceImpl,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    logger::init().expect("Failed to initialize logger.");

    if dotenvy::dotenv().is_ok() {
        log::info!(".env file loaded successfully.");
    }

    let server_address = ec2_helper::get_local_ip().await;
    let port = env::var("SERVER_PORT")
        .unwrap_or_else(|_| "50051".into())
        .parse()
        .unwrap_or(50051);
    let server_endpoint = SocketAddr::new(IpAddr::V4(server_address), port);

    let etcd_client = etcd_client_helper::create_etcd_client().await;
    // サービスエンドポイントをetcdに登録
    let _keep_alive_task = (
        etcd_client_helper::register_service_endpoint(
            etcd_client.clone(),
            server_endpoint.to_string(),
            "world".to_string(),
        )
        .await,
        etcd_client_helper::register_service_endpoint(
            etcd_client.clone(),
            server_endpoint.to_string(),
            "lobby".to_string(),
        )
        .await,
    );

    // DBサーバーのエンドポイントをetcdから取得
    let db_endpoint =
        etcd_client_helper::get_existing_service_endpoint(etcd_client.clone(), "db".to_string())
            .await
            .map(|kv| {
                kv.value()
                    .to_vec()
                    .into_iter()
                    .map(|b| b as char)
                    .collect::<String>()
            })
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", 50050));
    log::info!("Record DB Server endpoint: {}", db_endpoint);

    let zones = Arc::new(DashMap::new());
    let zones_watch = Arc::clone(&zones);
    etcd_client_helper::watch_changes(etcd_client.clone(), "zones/".to_string(), move |event| {
        let kv = event.kv().unwrap();
        let key = String::from_utf8_lossy(kv.key())
            .strip_prefix("zones/")
            .unwrap_or_default()
            .parse::<u64>()
            .unwrap_or_default();
        let value = String::from_utf8_lossy(kv.value()).to_string();
        match event.event_type() {
            etcd_client::EventType::Put => {
                log::info!("Zone updated: key={}, value={}", key, value);
                // Channel作成
                let channel = channel::Endpoint::from_shared(format!("http://{}", value))
                    .unwrap()
                    .connect_lazy();
                zones_watch.insert(key, channel);
            }
            etcd_client::EventType::Delete => {
                log::info!("Zone deleted: key={}", key);
                zones_watch.remove(&key);
            }
        }
    })
    .await;

    let zones_existing = Arc::clone(&zones);
    etcd_client_helper::get_existing_service_endpoint_prefix(
        etcd_client.clone(),
        Some("zones/".to_string()),
        move |key, value| {
            let key = key
                .strip_prefix("zones/")
                .unwrap_or_default()
                .parse::<u64>()
                .unwrap_or_default();
            log::info!("Existing zone: key={}, value={}", key, value);
            let channel = channel::Endpoint::from_shared(format!("http://{}", value))
                .unwrap()
                .connect_lazy();
            zones_existing.insert(key, channel);
        },
    )
    .await;

    let world_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 50051);
    let lobby_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 50052);

    // DBサーバーに接続
    let record_player_db_client = RecordPlayerDbServiceClient::connect(db_endpoint)
        .await
        .expect("Failed to connect to RecordPlayerDbService");
    log::info!("Connected to Record DB Server.");

    // Worldサーバーの起動
    let world_command_service = WorldCommandServiceImpl::new(zones);

    // バックグラウンドでWorldサーバーを起動
    tokio::spawn(async move {
        match Server::builder()
            .add_service(WorldCommandServer::new(world_command_service))
            .serve(world_addr)
            .await
        {
            Ok(_) => log::info!("World server stopped."),
            Err(e) => log::error!("Error in World server: {}", e),
        }
    });

    // Worldサーバーがポートをlistenするまで少し待つ
    tokio::time::sleep(Duration::from_millis(100)).await;
    log::info!("World server initialized and running.");

    // Lobbyサーバーの初期化
    let world_command_client =
        WorldCommandClient::connect(format!("http://localhost:{}", world_addr.port()))
            .await
            .expect("Failed to connect to World server");
    log::info!("Connected to World Server.");

    let lobby_service =
        LobbyServiceImpl::new(record_player_db_client.clone(), world_command_client);
    tokio::spawn(async move {
        // 起動
        match Server::builder()
            .add_service(LobbyServiceServer::new(lobby_service))
            .serve(lobby_addr)
            .await
        {
            Ok(_) => log::info!("Lobby server stopped."),
            Err(e) => log::error!("Error in Lobby server: {}", e),
        }
    });

    log::info!("All servers initialized and running. Press Ctrl+C to exit.");

    tokio::signal::ctrl_c().await?;
    log::info!("Shutting down gracefuly...");

    Ok(())
}
