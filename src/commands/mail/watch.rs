//! `herald mail watch` — subscribe to a folder and print each incoming email.

use jmap_base_client::JmapClient;
use jmap_mail_client::JmapMailExt;
use tokio::sync::mpsc::unbounded_channel;

use crate::jmap;
use crate::text::sanitize_display;

/// Watch a folder (default: Inbox) and print every newly arrived email until
/// interrupted. `all` watches every folder.
pub async fn watch(
    client: &JmapClient,
    folder: Option<&str>,
    all: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session);

    // Resolve the mailbox filter up front so a typo fails fast.
    let target = if all {
        None
    } else {
        let boxes = sc.mailbox_get(None, None).await?;
        let mailbox = match folder {
            Some(name) => boxes
                .list
                .iter()
                .find(|m| {
                    m.name.eq_ignore_ascii_case(name)
                        || m.role
                            .as_ref()
                            .is_some_and(|r| r.to_wire_str().eq_ignore_ascii_case(name))
                })
                .ok_or_else(|| format!("no mailbox named {name:?}"))?,
            None => boxes
                .list
                .iter()
                .find(|m| m.role.as_ref().is_some_and(|r| r.to_wire_str() == "inbox"))
                .ok_or("no Inbox mailbox found")?,
        };
        Some((mailbox.id.clone(), mailbox.name.clone()))
    };

    // Baseline Email state: an empty /get is the cheapest way to obtain it.
    let mut state = sc.email_get(Some(&[]), Some(&["id"]), None).await?.state;

    let (tx, mut rx) = unbounded_channel();
    let watcher = client.clone();
    tokio::spawn(jmap::push::watch_state_changes(watcher, "Email", move |change| {
        tx.send(change).is_ok()
    }));

    match &target {
        Some((_, name)) => println!("Watching {} — Ctrl-C to stop", sanitize_display(name)),
        None => println!("Watching all folders — Ctrl-C to stop"),
    }

    while let Some(_change) = rx.recv().await {
        // The push only says "Email state moved"; Email/changes tells us what.
        loop {
            let resp = match sc.email_changes(&state, None).await {
                Ok(resp) => resp,
                Err(e) => {
                    // e.g. cannotCalculateChanges after a long disconnect:
                    // resync the baseline and keep watching.
                    tracing::warn!("Email/changes failed ({e}); resyncing state");
                    state = sc.email_get(Some(&[]), Some(&["id"]), None).await?.state;
                    break;
                }
            };
            state = resp.new_state.clone();

            if !resp.created.is_empty() {
                // The typed Email deserializer requires all metadata fields
                // (blobId, threadId, mailboxIds, size, receivedAt) — request
                // them even though only a few are printed.
                let emails = sc
                    .email_get(
                        Some(&resp.created),
                        Some(&[
                            "id",
                            "blobId",
                            "threadId",
                            "mailboxIds",
                            "size",
                            "receivedAt",
                            "subject",
                            "from",
                        ]),
                        None,
                    )
                    .await?;
                for e in &emails.list {
                    if let Some((id, _)) = &target {
                        if !e.mailbox_ids.keys().any(|k| k == id) {
                            continue;
                        }
                    }
                    let from = e
                        .from
                        .as_ref()
                        .and_then(|addrs| addrs.first())
                        .map(|a| a.name.as_deref().unwrap_or(&a.email).to_string())
                        .unwrap_or_else(|| "(unknown)".into());
                    println!(
                        "{}  {}  {}  {}",
                        e.received_at.as_ref(),
                        e.id.as_ref(),
                        sanitize_display(&from),
                        sanitize_display(e.subject.as_deref().unwrap_or("(no subject)")),
                    );
                }
            }

            if !resp.has_more_changes {
                break;
            }
        }
    }
    Ok(())
}
