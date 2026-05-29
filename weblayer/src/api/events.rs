use super::{dom, rule_curation, AppState};
use crate::{
    core::{DomAnalysisBatch, DomCommand},
    sites,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub(super) async fn events_ws(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_event_socket(socket, state))
}

async fn handle_event_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel::<ServerEvent>();

    let writer = tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            let Ok(json) = serde_json::to_string(&event) else {
                warn!("failed to serialize WebSocket event");
                continue;
            };

            if let Err(error) = sender.send(Message::Text(json.into())).await {
                debug!(%error, "failed to send WebSocket event");
                break;
            }
        }
    });

    while let Some(message) = receiver.next().await {
        match message {
            Ok(Message::Text(text)) => {
                handle_client_event(text.as_str(), state.clone(), event_sender.clone()).await;
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Binary(_)) => {}
            Err(error) => {
                debug!(%error, "WebSocket receive failed");
                break;
            }
        }
    }

    drop(event_sender);
    if let Err(error) = writer.await {
        debug!(%error, "WebSocket writer task failed");
    }
}

async fn handle_client_event(
    text: &str,
    state: AppState,
    event_sender: mpsc::UnboundedSender<ServerEvent>,
) {
    let event = match serde_json::from_str::<ClientEvent>(text) {
        Ok(event) => event,
        Err(error) => {
            warn!(%error, "failed to parse WebSocket event");
            return;
        }
    };

    match event {
        ClientEvent::AnalyzeDom {
            request_id,
            page,
            elements,
        } => {
            let batch = DomAnalysisBatch { page, elements };
            if state.log_captured_content {
                dom::log_dom_batch(&batch);
            }

            if let Some(commands) =
                sites::cached_dom_commands(&batch, &state.ai_analyzer, &state.content_store)
            {
                rule_curation::schedule_x_rule_curation(state);
                let _ = event_sender.send(ServerEvent::commands(
                    request_id,
                    AnalysisPhase::Final,
                    commands,
                ));
                return;
            }

            let _ = event_sender.send(ServerEvent::commands(
                request_id.clone(),
                AnalysisPhase::Pending,
                sites::pending_dom_commands(&batch, &state.ai_analyzer, &state.content_store),
            ));

            let final_sender = event_sender.clone();
            tokio::spawn(async move {
                let commands =
                    sites::analyze_dom(&batch, &state.ai_analyzer, &state.content_store).await;
                rule_curation::schedule_x_rule_curation(state);
                let _ = final_sender.send(ServerEvent::commands(
                    request_id,
                    AnalysisPhase::Final,
                    commands,
                ));
            });
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ClientEvent {
    AnalyzeDom {
        #[serde(rename = "requestId")]
        request_id: String,
        page: crate::core::PageSnapshot,
        #[serde(default)]
        elements: Vec<crate::core::DomElementSnapshot>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerEvent {
    r#type: &'static str,
    request_id: String,
    phase: AnalysisPhase,
    commands: Vec<DomCommand>,
}

impl ServerEvent {
    fn commands(request_id: String, phase: AnalysisPhase, commands: Vec<DomCommand>) -> Self {
        Self {
            r#type: "commands",
            request_id,
            phase,
            commands,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum AnalysisPhase {
    Pending,
    Final,
}
