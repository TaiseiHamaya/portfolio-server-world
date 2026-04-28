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

    // 他のサーバーからアクセス可能なIPアドレスとポートを取得
    let server_address = ec2_helper::get_local_ip().await;

    let world_server_port = env::var("SERVER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50051);
    let world_server_endpoint = SocketAddr::new(IpAddr::V4(server_address), world_server_port);

    let lobby_server_port = env::var("LOBBY_SERVER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50052);
    let lobby_server_endpoint = SocketAddr::new(IpAddr::V4(server_address), lobby_server_port);

    // etcdクライアントの作成
    let etcd_client = etcd_client_helper::create_etcd_client().await;
    // サービスエンドポイントをetcdに登録
    let _keep_alive_task = (
        etcd_client_helper::register_service_endpoint(
            etcd_client.clone(),
            world_server_endpoint,
            "world".to_string(),
        )
        .await,
        etcd_client_helper::register_service_endpoint(
            etcd_client.clone(),
            lobby_server_endpoint,
            "lobby".to_string(),
        )
        .await,
    );

    // DBサーバーのエンドポイントをetcdから取得
    let db_endpoint =
        etcd_client_helper::get_existing_service_endpoint(etcd_client.clone(), "db".to_string())
            .await
            .map(|kv| ["http://", &String::from_utf8_lossy(kv.value())].concat())
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
                let channel = channel::Endpoint::from_shared(["http://", &value].concat())
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
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or_default();
            log::info!("Existing zone: key={}, value={}", key, value);
            let channel = channel::Endpoint::from_shared(["http://", &value].concat())
                .unwrap()
                .connect_lazy();
            zones_existing.insert(key, channel);
        },
    )
    .await;

    let world_server_serve_addr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), world_server_port);
    let lobby_server_serve_addr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), lobby_server_port);

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
            .serve(world_server_serve_addr)
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
        WorldCommandClient::connect(format!("http://{}", world_server_endpoint))
            .await
            .expect("Failed to connect to World server");
    log::info!("Connected to World Server.");

    let lobby_service =
        LobbyServiceImpl::new(record_player_db_client.clone(), world_command_client);
    tokio::spawn(async move {
        // 起動
        match Server::builder()
            .add_service(LobbyServiceServer::new(lobby_service))
            .serve(lobby_server_serve_addr)
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
