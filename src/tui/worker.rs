//! Non-blocking JMAP worker: every data command spawns a tokio task with a
//! cloned client; results come back as events on the message channel. The
//! render loop never blocks on the network.

use jmap_base_client::JmapClient;
use jmap_mail_client::JmapMailExt;
use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::Profile;
use crate::jmap::{self, JmapResult};

use super::messages::{Command, Event, Message};
use super::model::folders::RawFolder;
use super::types::{ContactEntry, EventEntry, MailEntry};

/// Emails fetched per page.
pub const MAIL_PAGE_SIZE: usize = 50;

pub struct Worker {
    sender: UnboundedSender<Message>,
    client: Option<JmapClient>,
    profile: Profile,
    profile_name: String,
}

impl Worker {
    pub fn new(sender: UnboundedSender<Message>, profile: Profile, profile_name: String) -> Self {
        Self {
            sender,
            client: None,
            profile,
            profile_name,
        }
    }

    fn send(&self, event: Event) {
        let _ = self.sender.send(Message::Event(event));
    }

    /// Handle one message; data commands spawn async tasks.
    pub fn handle(&mut self, message: &Message) {
        match message {
            Message::Event(Event::ClientReady(client)) => {
                self.client = Some((**client).clone());
            }
            Message::Command(cmd) => self.handle_command(cmd),
            _ => {}
        }
    }

    fn client(&self) -> Option<JmapClient> {
        if self.client.is_none() {
            let _ = self
                .sender
                .send(Message::Event(Event::ActionFailed("not connected".into())));
        }
        self.client.clone()
    }

    fn handle_command(&mut self, cmd: &Command) {
        let tx = self.sender.clone();
        match cmd {
            Command::Connect => {
                let profile = self.profile.clone();
                let name = self.profile_name.clone();
                tokio::spawn(async move {
                    match crate::auth::create_client(&profile, &name).await {
                        Ok(client) => {
                            let _ = tx.send(Message::Event(Event::ClientReady(Box::new(client))));
                        }
                        Err(e) => {
                            let _ = tx.send(Message::Event(Event::ConnectFailed(e.to_string())));
                        }
                    }
                });
            }

            Command::LoadFolders => {
                let Some(client) = self.client() else { return };
                let mappings = self.profile.folders.clone();
                tokio::spawn(async move {
                    match fetch_raw_folders(&client).await {
                        Ok(raw) => {
                            let folders =
                                super::model::folders::build_folder_list(&raw, &mappings);
                            let _ = tx.send(Message::Event(Event::FoldersLoaded(folders)));
                        }
                        Err(e) => {
                            let _ = tx.send(Message::Event(Event::ActionFailed(format!(
                                "Mailbox error: {e}"
                            ))));
                        }
                    }
                });
            }

            Command::LoadMailPage {
                folder_id,
                position,
            } => {
                let Some(client) = self.client() else { return };
                let folder_id = folder_id.clone();
                let position = *position;
                tokio::spawn(async move {
                    match fetch_mail_page(&client, folder_id.as_deref(), position).await {
                        Ok(mails) => {
                            let _ = tx.send(Message::Event(Event::MailPageLoaded {
                                mails,
                                position,
                                all_folders: folder_id.is_none(),
                            }));
                        }
                        Err(e) => {
                            let _ = tx.send(Message::Event(Event::ActionFailed(format!(
                                "Mail error: {e}"
                            ))));
                        }
                    }
                });
            }

            Command::LoadMailBody(id) => {
                let Some(client) = self.client() else { return };
                let id = id.clone();
                tokio::spawn(async move {
                    match jmap::mail::fetch_full_email(&client, &id).await {
                        Ok(full) => {
                            let _ =
                                tx.send(Message::Event(Event::MailBodyLoaded(Box::new(full))));
                        }
                        Err(e) => {
                            let _ = tx.send(Message::Event(Event::ActionFailed(format!(
                                "Read error: {e}"
                            ))));
                        }
                    }
                });
            }

            Command::LoadContacts => {
                let Some(client) = self.client() else { return };
                tokio::spawn(async move {
                    match fetch_contacts(&client).await {
                        Ok(contacts) => {
                            let _ = tx.send(Message::Event(Event::ContactsLoaded(contacts)));
                        }
                        Err(e) => {
                            let _ = tx.send(Message::Event(Event::ActionFailed(format!(
                                "Contacts error: {e}"
                            ))));
                        }
                    }
                });
            }

            Command::LoadEvents => {
                let Some(client) = self.client() else { return };
                tokio::spawn(async move {
                    match fetch_events(&client).await {
                        Ok(events) => {
                            let _ = tx.send(Message::Event(Event::EventsLoaded(events)));
                        }
                        Err(e) => {
                            let _ = tx.send(Message::Event(Event::ActionFailed(format!(
                                "Calendar error: {e}"
                            ))));
                        }
                    }
                });
            }

            Command::SendMail {
                to,
                cc,
                bcc,
                subject,
                body,
                sent_mailbox_id,
            } => {
                let Some(client) = self.client() else { return };
                let profile = self.profile.clone();
                let (to, cc, bcc) = (to.clone(), cc.clone(), bcc.clone());
                let (subject, body) = (subject.clone(), body.clone());
                let sent_id = sent_mailbox_id.clone();
                tokio::spawn(async move {
                    let Some(from_email) = profile.from_email.as_deref() else {
                        let _ = tx.send(Message::Event(Event::ActionFailed(
                            "set from_email in the profile to send mail".into(),
                        )));
                        return;
                    };
                    let mail = jmap::mail::OutgoingMail {
                        from_email,
                        from_name: profile.from_name.as_deref().unwrap_or(""),
                        to: &to,
                        cc: &cc,
                        bcc: &bcc,
                        subject: &subject,
                        body: &body,
                    };
                    let result =
                        jmap::mail::send_message(&client, &mail, sent_id.as_deref()).await;
                    let _ = tx.send(Message::Event(done("✓ Email sent", "send", result)));
                });
            }

            Command::MoveMail {
                email_id,
                source_mailbox_id,
                target_mailbox_id,
                action,
            } => {
                let Some(client) = self.client() else { return };
                let (id, src, dst) = (
                    email_id.clone(),
                    source_mailbox_id.clone(),
                    target_mailbox_id.clone(),
                );
                let action = action.clone();
                tokio::spawn(async move {
                    let result = jmap::mail::move_email(&client, &id, &src, &dst).await;
                    let _ = tx.send(Message::Event(done(
                        &format!("✓ {action} done"),
                        &action,
                        result,
                    )));
                });
            }

            Command::CreateContact { name, email, phone } => {
                let Some(client) = self.client() else { return };
                let (name, email, phone) = (name.clone(), email.clone(), phone.clone());
                tokio::spawn(async move {
                    let result = jmap::contacts::create_contact(&client, &name, &email, &phone).await;
                    let _ = tx.send(Message::Event(done("✓ Contact created", "contact", result)));
                });
            }

            Command::UpdateContact {
                id,
                name,
                email,
                phone,
            } => {
                let Some(client) = self.client() else { return };
                let (id, name, email, phone) =
                    (id.clone(), name.clone(), email.clone(), phone.clone());
                tokio::spawn(async move {
                    let result =
                        jmap::contacts::update_contact(&client, &id, &name, &email, &phone).await;
                    let _ = tx.send(Message::Event(done("✓ Contact updated", "contact", result)));
                });
            }

            Command::DeleteContact(id) => {
                let Some(client) = self.client() else { return };
                let id = id.clone();
                tokio::spawn(async move {
                    let result = jmap::contacts::delete_contact(&client, &id).await;
                    let _ = tx.send(Message::Event(done("✓ Contact deleted", "contact", result)));
                });
            }

            Command::CreateEvent {
                title,
                start,
                duration,
            } => {
                let Some(client) = self.client() else { return };
                let (title, start, duration) = (title.clone(), start.clone(), duration.clone());
                tokio::spawn(async move {
                    let result =
                        jmap::calendar::create_event(&client, &title, &start, &duration).await;
                    let _ = tx.send(Message::Event(done("✓ Event created", "event", result)));
                });
            }

            Command::UpdateEvent {
                id,
                title,
                start,
                duration,
            } => {
                let Some(client) = self.client() else { return };
                let (id, title, start, duration) =
                    (id.clone(), title.clone(), start.clone(), duration.clone());
                tokio::spawn(async move {
                    let result =
                        jmap::calendar::update_event(&client, &id, &title, &start, &duration)
                            .await;
                    let _ = tx.send(Message::Event(done("✓ Event updated", "event", result)));
                });
            }

            Command::DeleteEvent(id) => {
                let Some(client) = self.client() else { return };
                let id = id.clone();
                tokio::spawn(async move {
                    let result = jmap::calendar::delete_event(&client, &id).await;
                    let _ = tx.send(Message::Event(done("✓ Event deleted", "event", result)));
                });
            }

            _ => {}
        }
    }
}

/// Map an operation result to the completion/failure event.
fn done(ok_msg: &str, label: &str, result: JmapResult<()>) -> Event {
    match result {
        Ok(()) => Event::ActionCompleted(ok_msg.to_string()),
        Err(e) => Event::ActionFailed(format!("{label}: {e}")),
    }
}

async fn fetch_raw_folders(client: &JmapClient) -> JmapResult<Vec<RawFolder>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session);
    let resp = sc.mailbox_get(None, None).await?;
    Ok(resp
        .list
        .iter()
        .map(|m| RawFolder {
            id: m.id.as_ref().to_string(),
            name: m.name.clone(),
            parent_id: m.parent_id.as_ref().map(|p| p.as_ref().to_string()),
            role: m.role.as_ref().map(|r| r.to_wire_str().to_string()),
            sort_order: m.sort_order,
            total_emails: m.total_emails,
            unread_emails: m.unread_emails,
        })
        .collect())
}

/// One page of emails, newest first. `folder_id = None` queries all folders.
async fn fetch_mail_page(
    client: &JmapClient,
    folder_id: Option<&str>,
    position: usize,
) -> JmapResult<Vec<MailEntry>> {
    let session = client.fetch_session().await?;
    let sc = client.with_mail_session(session);

    let filter = folder_id.map(|id| json!({ "inMailbox": id }));
    let sort = json!([{ "property": "receivedAt", "isAscending": false }]);
    let query_resp = sc
        .email_query(
            filter,
            Some(sort),
            Some(position as u64),
            Some(MAIL_PAGE_SIZE as u64),
            None,
        )
        .await?;

    if query_resp.ids.is_empty() {
        return Ok(Vec::new());
    }

    let email_resp = sc
        .email_get(
            Some(&query_resp.ids),
            Some(&[
                "id",
                "blobId",
                "threadId",
                "mailboxIds",
                "size",
                "receivedAt",
                "subject",
                "from",
                "preview",
            ]),
            None,
        )
        .await?;

    Ok(email_resp
        .list
        .iter()
        .map(|e| {
            let from = e
                .from
                .as_ref()
                .and_then(|addrs| addrs.first())
                .map(|a| a.name.as_deref().unwrap_or(&a.email).to_string())
                .unwrap_or_else(|| "(unknown)".into());
            MailEntry {
                id: e.id.as_ref().to_string(),
                subject: e.subject.as_deref().unwrap_or("(no subject)").to_string(),
                from,
                date: e.received_at.as_ref().to_string(),
                preview: e.preview.as_deref().unwrap_or("").to_string(),
                folder_id: e.mailbox_ids.keys().next().map(|id| id.as_ref().to_string()),
            }
        })
        .collect())
}

async fn fetch_contacts(client: &JmapClient) -> JmapResult<Vec<ContactEntry>> {
    use jmap_contacts_client::JmapContactsExt;

    let session = client.fetch_session().await?;
    let sc = client.with_contacts_session(session);
    let resp = sc
        .contact_card_get(None, Some(&["id", "name", "emails", "phones"]))
        .await?;

    Ok(resp
        .list
        .iter()
        .map(|card| ContactEntry {
            id: card
                .id
                .as_ref()
                .map(|i| i.as_ref().to_string())
                .unwrap_or_default(),
            name: jmap::contacts::extract_contact_name(card),
            email: jmap::contacts::extract_first_email(card),
            phone: jmap::contacts::extract_first_phone(card),
        })
        .collect())
}

async fn fetch_events(client: &JmapClient) -> JmapResult<Vec<EventEntry>> {
    use jmap_calendars_client::JmapCalendarsExt;

    let session = client.fetch_session().await?;
    let sc = client.with_calendars_session(session);
    let resp = sc
        .calendar_event_get(
            None,
            Some(&["id", "title", "start", "duration", "status"]),
            None,
        )
        .await?;

    Ok(resp
        .list
        .iter()
        .map(|e| EventEntry {
            id: e
                .id
                .as_ref()
                .map(|i| i.as_ref().to_string())
                .unwrap_or_default(),
            title: e.title.as_deref().unwrap_or("(no title)").to_string(),
            start: e.start.as_deref().unwrap_or("").to_string(),
            duration: e.duration.as_deref().unwrap_or("").to_string(),
            status: e.status.as_deref().unwrap_or("").to_string(),
        })
        .collect())
}
