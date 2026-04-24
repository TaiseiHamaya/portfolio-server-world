use std::sync::Arc;

use dashmap::DashSet;

use crate::generated::{
    proto_client::{
        PayloadPlayerLoadRequest, record_player_db_service_client::RecordPlayerDbServiceClient,
    },
    proto_inner::PayloadBeginRouteCommand,
    proto_inner::{
        PayloadLobbyEndGameRequest, PayloadLobbyEndGameResponse, PayloadLobbyEnterRequest,
        PayloadLobbyEnterResponse, PayloadLobbyExitRequest, PayloadLobbyExitResponse,
        PayloadLobbyStartGameRequest, PayloadLobbyStartGameResponse,
        lobby_service_server::LobbyService, world_command_client::WorldCommandClient,
    },
};

#[derive(Debug)]
pub struct LobbyServiceImpl {
    world_command_client: WorldCommandClient<tonic::transport::Channel>,
    record_player_db_client: RecordPlayerDbServiceClient<tonic::transport::Channel>,

    user_accounts: Arc<DashSet<u64>>,
}

impl LobbyServiceImpl {
    pub fn new(
        record_player_db_client: RecordPlayerDbServiceClient<tonic::transport::Channel>,
        world_command_client: WorldCommandClient<tonic::transport::Channel>,
    ) -> Self {
        Self {
            record_player_db_client,
            world_command_client,
            user_accounts: Arc::new(DashSet::new()),
        }
    }
}

#[tonic::async_trait]
impl LobbyService for LobbyServiceImpl {
    /// Gateway -> Lobby : player enter lobby request.
    async fn lobby_enter(
        &self,
        request: tonic::Request<PayloadLobbyEnterRequest>,
    ) -> std::result::Result<tonic::Response<PayloadLobbyEnterResponse>, tonic::Status> {
        let user_id = request.into_inner().user_id;

        let mut record_player_db = self.record_player_db_client.clone();
        let Ok(player_data) = record_player_db
            .load_player(PayloadPlayerLoadRequest { user_id })
            .await
            .map(|response| response.into_inner())
        else {
            return Err(tonic::Status::internal("Failed to load player data."));
        };

        let Some(record) = player_data.record else {
            return Err(tonic::Status::not_found("Player data not found."));
        };

        self.user_accounts.insert(user_id);
        return Ok(tonic::Response::new(PayloadLobbyEnterResponse {
            is_succeeded: true,
            character_name: record.username.clone(),
        }));
    }

    /// Gateway -> Lobby : player exit lobby request.
    async fn lobby_exit(
        &self,
        request: tonic::Request<PayloadLobbyExitRequest>,
    ) -> std::result::Result<tonic::Response<PayloadLobbyExitResponse>, tonic::Status> {
        let user_id = request.into_inner().user_id;

        self.user_accounts.remove(&user_id);
        Ok(tonic::Response::new(PayloadLobbyExitResponse {
            success: true,
        }))
    }

    /// Gateway -> Lobby : player start game request.
    async fn start_game(
        &self,
        request: tonic::Request<PayloadLobbyStartGameRequest>,
    ) -> std::result::Result<tonic::Response<PayloadLobbyStartGameResponse>, tonic::Status> {
        let user_id = request.into_inner().user_id;

        let Ok(player_data) = self
            .record_player_db_client
            .clone()
            .load_player(PayloadPlayerLoadRequest { user_id })
            .await
        else {
            return Err(tonic::Status::internal("Failed to load player data."));
        };

        let Some(record) = player_data.into_inner().record else {
            return Err(tonic::Status::not_found("Player data not found."));
        };

        let mut world_command_client = self.world_command_client.clone();
        let Ok(result) = world_command_client
            .begin_zone_route(PayloadBeginRouteCommand {
                user_id,
                source_zone_id: None,
                target_zone_id: Some(record.zone_id),
            })
            .await
        else {
            return Err(tonic::Status::internal(
                "Failed to send zone route begin command.",
            ));
        };

        let Some(player_entity_id) = result.into_inner().player_entity_id else {
            return Err(tonic::Status::internal(
                "Failed to begin zone route for player.",
            ));
        };

        self.user_accounts.remove(&user_id);
        return Ok(tonic::Response::new(PayloadLobbyStartGameResponse {
            player_entity_id,
        }));
    }

    /// Gateway -> Lobby : player end game request.
    async fn end_game(
        &self,
        request: tonic::Request<PayloadLobbyEndGameRequest>,
    ) -> std::result::Result<tonic::Response<PayloadLobbyEndGameResponse>, tonic::Status> {
        let user_id = request.into_inner().user_id;
        self.user_accounts.remove(&user_id);
        Ok(tonic::Response::new(PayloadLobbyEndGameResponse {
            success: true,
        }))
    }
}
