//! Live state-change subscription over the JMAP event source (RFC 8620 §7.3).
//!
//! One reconnecting SSE loop shared by the CLI (`herald mail watch`) and the
//! TUI. Each [`StateChange`] push is handed to a caller-supplied callback;
//! the callback returns `false` to stop watching (e.g. its channel closed).

use std::time::Duration;

use futures::StreamExt;
use jmap_base_client::{JmapClient, SseEvent, StateChange, SubscribeEventsSessionParams};

/// Server ping interval requested on the event source, in seconds. Keeps
/// intermediaries from idling out the connection and lets us detect a dead
/// stream within roughly this window.
const PING_SECS: u32 = 30;

/// First reconnect delay; doubled after every failed attempt up to the max.
const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Subscribe to state changes for `types` (comma-separated JMAP type names,
/// or `"*"` for everything) and invoke `on_change` for each push.
///
/// Reconnects forever with exponential backoff, resuming via `Last-Event-ID`
/// where the server supports it. Returns only when `on_change` returns
/// `false`. Intended to run inside `tokio::spawn`.
pub async fn watch_state_changes(
    client: JmapClient,
    types: &'static str,
    mut on_change: impl FnMut(StateChange) -> bool,
) {
    let mut last_event_id: Option<String> = None;
    let mut backoff = BACKOFF_START;

    loop {
        match open_stream(&client, types, last_event_id.as_deref()).await {
            Ok(mut stream) => {
                while let Some(frame) = stream.next().await {
                    match frame {
                        Ok(frame) => {
                            backoff = BACKOFF_START;
                            if frame.id.is_some() {
                                last_event_id = frame.id;
                            }
                            if let SseEvent::StateChange(change) = frame.event {
                                if !on_change(change) {
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!("event source stream error: {e}");
                            break;
                        }
                    }
                }
                tracing::debug!("event source disconnected; reconnecting");
            }
            Err(e) => {
                tracing::debug!("event source connect failed: {e}");
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

async fn open_stream(
    client: &JmapClient,
    types: &str,
    last_event_id: Option<&str>,
) -> Result<
    futures::stream::BoxStream<
        'static,
        Result<jmap_base_client::SseFrame, jmap_base_client::ClientError>,
    >,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let session = client.fetch_session().await?;
    let stream = client
        .subscribe_events_session(
            &session,
            SubscribeEventsSessionParams {
                types: Some(types),
                close_after: Some("no"),
                ping: Some(PING_SECS),
                last_event_id,
            },
        )
        .await?;
    Ok(stream)
}
