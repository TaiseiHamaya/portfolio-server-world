use dashmap::DashMap;
use std::sync::Arc;

use crate::generated::{
    proto_client::{
        PayloadPlayerExitZoneBegin, PayloadPlayerExitZoneComplete, PayloadPlayerExitZoneReady,
        PayloadPlayerZoneEnterBegin, PayloadPlayerZoneEnterComplete, PayloadPlayerZoneEnterReady,
        PlayerRouteResult, world_route_service_client::WorldRouteServiceClient,
    },
    proto_inner::{
        PayloadBeginRouteCommand, PayloadPlayerRouteCommandResponse,
        world_command_server::WorldCommand,
    },
};

#[derive(Debug, Clone)]
pub struct WorldCommandServiceImpl {
    pub zone_clients: Arc<DashMap<u64, tonic::transport::Channel>>,
}

impl WorldCommandServiceImpl {
    pub fn new(zone_clients: Arc<DashMap<u64, tonic::transport::Channel>>) -> Self {
        Self { zone_clients }
    }

    pub fn get_zone_client(
        &self,
        zone_id: u64,
    ) -> Option<WorldRouteServiceClient<tonic::transport::Channel>> {
        let Some(channel) = self.zone_clients.get(&zone_id).map(|entry| entry.clone()) else {
            log::error!("Zone client not found for zone_id {}.", zone_id);
            return None;
        };

        Some(WorldRouteServiceClient::new(channel))
    }
}

#[tonic::async_trait]
impl WorldCommand for WorldCommandServiceImpl {
    /// Zone -> World: player begin zone exit request.
    async fn begin_zone_route(
        &self,
        request: tonic::Request<PayloadBeginRouteCommand>,
    ) -> std::result::Result<tonic::Response<PayloadPlayerRouteCommandResponse>, tonic::Status>
    {
        let inner = request.into_inner();
        let source_zone_id = inner.source_zone_id;
        let target_zone_id = inner.target_zone_id;
        let user_id = inner.user_id;
        log::info!(
            "Received begin zone route command for user_id {}. source_zone_id: {:?}, target_zone_id: {:?}",
            user_id,
            source_zone_id,
            target_zone_id
        );

        // sourceがNoneの場合はゲーム開始時
        if let Some(source_zone_id) = source_zone_id {
            let Some(mut zone_client) = self.get_zone_client(source_zone_id) else {
                // zoneが存在しない
                log::error!(
                    "Error occurred while getting zone_client for source_zone_id {}.",
                    source_zone_id
                );
                return Err(tonic::Status::internal(format!(
                    "Failed to get zone client for source_zone_id {}.",
                    source_zone_id
                )));
            };

            // 元のゾーンにプレイヤーの削除を依頼
            match zone_client
                .begin_player_zone_exit(PayloadPlayerExitZoneBegin { user_id })
                .await
            {
                Ok(_) => {
                    log::info!(
                        "Successfully sent begin player zone exit for user_id {} in source_zone_id {}.",
                        user_id,
                        source_zone_id
                    );
                }
                Err(e) => {
                    log::error!(
                        "Error occurred while beginning player zone exit for user_id {} in source_zone_id {}: {}",
                        user_id,
                        source_zone_id,
                        e
                    );
                    return Err(tonic::Status::internal(format!(
                        "Failed to begin player zone exit for user_id {} in source_zone_id {}: {}",
                        user_id, source_zone_id, e
                    )));
                }
            }

            // チェック
            'check_exit_ready: loop {
                let result = zone_client
                    .check_for_ready_exit_player(PayloadPlayerExitZoneReady { user_id })
                    .await;
                match result {
                    Ok(response) => {
                        let route_result = response.into_inner().result;
                        match PlayerRouteResult::try_from(route_result) {
                            Ok(PlayerRouteResult::Success) => {
                                log::info!("Player {} is ready to exit the zone.", user_id);
                                break 'check_exit_ready; // 成功したのでループを抜ける
                            }
                            Ok(PlayerRouteResult::NotReady) => {
                                log::info!("Player {} is not ready to exit the zone yet.", user_id);
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await; // 1秒待ってから再度チェック
                            }
                            Ok(PlayerRouteResult::Failed) => {
                                log::error!(
                                    "An error occurred while checking player exit readiness for player {}.",
                                    user_id
                                );
                                return Err(tonic::Status::internal(format!(
                                    "Failed to check player exit readiness for user_id {} in source_zone_id {}.",
                                    user_id, source_zone_id
                                )));
                            }
                            Err(_) => {
                                log::error!("Invalid player route result: {}", route_result);
                                return Err(tonic::Status::internal(format!(
                                    "Invalid player route result: {}",
                                    route_result
                                )));
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Error occurred while checking player exit readiness: {}", e);
                        return Err(tonic::Status::internal(format!(
                            "Error occurred while checking player exit readiness: {}",
                            e
                        )));
                    }
                }
            }

            // 退出
            let Ok(result) = zone_client
                .execute_exit_zone_player(PayloadPlayerExitZoneComplete { user_id })
                .await
            else {
                log::error!(
                    "Failed to execute exit zone player for user_id {}.",
                    user_id
                );
                return Err(tonic::Status::internal(format!(
                    "Failed to execute exit zone player for user_id {}.",
                    user_id
                )));
            };

            let result = result.into_inner().result;
            let _ = match PlayerRouteResult::try_from(result) {
                Ok(PlayerRouteResult::Success) => {
                    log::info!("Player {} has successfully exited the zone.", user_id);
                    ()
                }
                Ok(_) => {
                    log::error!(
                        "Failed to execute exit zone player for user_id {}.",
                        user_id
                    );
                    return Err(tonic::Status::internal(format!(
                        "Failed to execute exit zone player for user_id {}.",
                        user_id
                    )));
                }
                Err(_) => {
                    log::error!("Invalid player route result: {}", result);
                    return Err(tonic::Status::internal(format!(
                        "Invalid player route result: {}",
                        result
                    )));
                }
            };
        }

        if let Some(target_zone_id) = target_zone_id {
            let Some(mut zone_client) = self.get_zone_client(target_zone_id) else {
                // zoneが存在しない
                log::error!(
                    "Error occurred while getting zone_client for target_zone_id {}.",
                    target_zone_id
                );
                return Err(tonic::Status::internal(format!(
                    "Failed to get zone client for target_zone_id {}.",
                    target_zone_id
                )));
            };

            // 新しいゾーンにプレイヤーの追加を依頼
            let _ = match zone_client
                .begin_player_zone_enter(PayloadPlayerZoneEnterBegin { user_id })
                .await
            {
                Ok(_) => {
                    log::info!(
                        "Successfully sent begin player zone enter for user_id {} in target_zone_id {}.",
                        user_id,
                        target_zone_id
                    );
                    ()
                }
                Err(e) => {
                    log::error!(
                        "Error occurred while beginning player zone enter for user_id {} in target_zone_id {}: {}",
                        user_id,
                        target_zone_id,
                        e
                    );
                    return Err(tonic::Status::internal(format!(
                        "Failed to begin player zone enter for user_id {} in target_zone_id {}: {}",
                        user_id, target_zone_id, e
                    )));
                }
            };

            'check_enter_ready: loop {
                let result = zone_client
                    .check_for_ready_enter_player(PayloadPlayerZoneEnterReady { user_id })
                    .await;
                match result {
                    Ok(response) => {
                        let route_result = response.into_inner().result;
                        match PlayerRouteResult::try_from(route_result) {
                            Ok(PlayerRouteResult::Success) => {
                                log::info!("Player {} is ready to enter the new zone.", user_id);
                                break 'check_enter_ready; // 成功したのでループを抜ける
                            }
                            Ok(PlayerRouteResult::NotReady) => {
                                log::info!(
                                    "Player {} is not ready to enter the new zone yet.",
                                    user_id
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await; // 1秒待ってから再度チェック
                            }
                            Ok(PlayerRouteResult::Failed) => {
                                log::error!(
                                    "An error occurred while checking player enter readiness for player {}.",
                                    user_id
                                );
                                return Err(tonic::Status::internal(format!(
                                    "Failed to check player enter readiness for user_id {} in target_zone_id {}.",
                                    user_id, target_zone_id
                                )));
                            }
                            Err(_) => {
                                log::error!("Invalid player route result: {}", route_result);
                                return Err(tonic::Status::internal(format!(
                                    "Invalid player route result: {}",
                                    route_result
                                )));
                            }
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "Error occurred while checking player enter readiness: {}",
                            e
                        );
                        return Err(tonic::Status::internal(format!(
                            "Error occurred while checking player enter readiness: {}",
                            e
                        )));
                    }
                }
            }

            // execute enter
            let Ok(result) = zone_client
                .execute_enter_zone_player(PayloadPlayerZoneEnterComplete { user_id })
                .await
            else {
                log::error!(
                    "Failed to execute enter zone player for user_id {} in target_zone_id {}.",
                    user_id,
                    target_zone_id
                );
                return Err(tonic::Status::internal(format!(
                    "Failed to execute enter zone player for user_id {} in target_zone_id {}.",
                    user_id, target_zone_id
                )));
            };

            let player_entity_id = result.into_inner().player_entity_id;
            log::info!(
                "Player {} has successfully entered the new zone with player_entity_id {:?}.",
                user_id,
                player_entity_id
            );
            return Ok(tonic::Response::new(PayloadPlayerRouteCommandResponse {
                player_entity_id,
            }));
        }

        return Ok(tonic::Response::new(PayloadPlayerRouteCommandResponse {
            player_entity_id: None,
        }));
    }
}
