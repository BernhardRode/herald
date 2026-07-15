//! Calendar operations and time helpers.

use jmap_base_client::JmapClient;
use jmap_mail_client::JmapMailExt;
use serde_json::json;

use super::{check_set_response, JmapResult};

/// Return the current UTC time as an ISO 8601 string (e.g. "2026-07-13T12:00:00")
/// suitable for JMAP calendar date fields and the CalendarEvent/query `after` filter.
pub fn utc_now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Civil date from days since 1970-01-01 (algorithm from Howard Hinnant)
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        y, m, d, hours, minutes, seconds
    )
}

/// Create a calendar event. Empty `start` defaults to now, empty `duration` to one hour.
pub async fn create_event(
    client: &JmapClient,
    title: &str,
    start: &str,
    duration: &str,
) -> JmapResult<()> {
    let session = client.fetch_session().await?;
    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:calendars")
        .ok_or("no primary calendars account in session")?;

    let start = if start.trim().is_empty() {
        utc_now_iso8601()
    } else {
        start.trim().to_string()
    };
    let duration = if duration.trim().is_empty() {
        "PT1H".to_string()
    } else {
        duration.trim().to_string()
    };

    let request_args = json!({
        "accountId": account_id,
        "create": {
            "new1": {
                "@type": "Event",
                "title": title,
                "start": start,
                "duration": duration
            }
        }
    });
    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:calendars".to_string(),
        ],
        vec![(
            "CalendarEvent/set".to_string(),
            request_args,
            "create1".to_string(),
        )],
        None,
    );
    let resp = client.call(session.api_url.as_str(), &request).await?;
    check_set_response(&resp, "CalendarEvent/set", "notCreated")
}

/// A calendar event to create, with optional scheduling (invite) fields.
///
/// When `attendees` is non-empty the event is created with `isDraft: false`
/// and a `participants`/`replyTo` block, which makes the server send iMIP
/// scheduling invitations to each attendee.
pub struct NewEvent<'a> {
    pub title: &'a str,
    /// Local start time, e.g. `2026-07-16T08:30:00`. Empty defaults to now.
    pub start: &'a str,
    /// ISO 8601 duration, e.g. `PT15M`. Empty defaults to `PT1H`.
    pub duration: &'a str,
    /// Calendar id to file the event under. `None` uses the account default.
    pub calendar_id: Option<&'a str>,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    /// IANA time zone, e.g. `Europe/Berlin`.
    pub time_zone: Option<&'a str>,
    pub all_day: bool,
    /// Attendee email addresses to invite.
    pub attendees: &'a [String],
    /// Organizer address. `None` derives it from the primary mail identity.
    pub organizer: Option<&'a str>,
}

/// Create a calendar event, optionally inviting attendees.
///
/// See [`NewEvent`] for the scheduling semantics.
pub async fn create_event_full(client: &JmapClient, ev: &NewEvent<'_>) -> JmapResult<()> {
    let session = client.fetch_session().await?;
    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:calendars")
        .ok_or("no primary calendars account in session")?;

    let start = if ev.start.trim().is_empty() {
        utc_now_iso8601()
    } else {
        ev.start.trim().to_string()
    };
    let duration = if ev.duration.trim().is_empty() {
        "PT1H".to_string()
    } else {
        ev.duration.trim().to_string()
    };

    let mut event = serde_json::Map::new();
    event.insert("@type".into(), json!("Event"));
    event.insert("title".into(), json!(ev.title));
    event.insert("start".into(), json!(start));
    if ev.all_day {
        event.insert("showWithoutTime".into(), json!(true));
    } else {
        event.insert("duration".into(), json!(duration));
    }
    if let Some(tz) = ev.time_zone.map(str::trim).filter(|s| !s.is_empty()) {
        event.insert("timeZone".into(), json!(tz));
    }
    if let Some(desc) = ev.description.map(str::trim).filter(|s| !s.is_empty()) {
        event.insert("description".into(), json!(desc));
    }
    if let Some(loc) = ev.location.map(str::trim).filter(|s| !s.is_empty()) {
        event.insert(
            "locations".into(),
            json!({ "loc1": { "@type": "Location", "name": loc } }),
        );
    }
    // An event must belong to at least one calendar; fall back to the default.
    let calendar_id = match ev.calendar_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(cal) => cal.to_string(),
        None => default_calendar_id(client, &session, account_id).await?,
    };
    event.insert("calendarIds".into(), json!({ calendar_id: true }));

    // Scheduling: with attendees, build participants and let the server send
    // iMIP invites by clearing the draft flag.
    let attendees: Vec<&str> = ev
        .attendees
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect();
    if !attendees.is_empty() {
        let organizer = match ev.organizer.map(str::trim).filter(|s| !s.is_empty()) {
            Some(o) => o.to_string(),
            None => default_identity_email(client, &session).await?,
        };

        let mut participants = serde_json::Map::new();
        participants.insert(
            "owner".into(),
            json!({
                "@type": "Participant",
                "email": organizer,
                "sendTo": { "imip": format!("mailto:{organizer}") },
                "roles": { "owner": true, "attendee": true },
                "participationStatus": "accepted",
                "expectReply": false,
            }),
        );
        for (i, addr) in attendees.iter().enumerate() {
            participants.insert(
                format!("a{i}"),
                json!({
                    "@type": "Participant",
                    "email": addr,
                    "sendTo": { "imip": format!("mailto:{addr}") },
                    "roles": { "attendee": true },
                    "participationStatus": "needs-action",
                    "expectReply": true,
                }),
            );
        }
        event.insert("participants".into(), json!(participants));
        event.insert(
            "replyTo".into(),
            json!({ "imip": format!("mailto:{organizer}") }),
        );
        event.insert("isDraft".into(), json!(false));
    }

    let request_args = json!({
        "accountId": account_id,
        "create": { "new1": event }
    });
    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:calendars".to_string(),
        ],
        vec![(
            "CalendarEvent/set".to_string(),
            request_args,
            "create1".to_string(),
        )],
        None,
    );
    let resp = client.call(session.api_url.as_str(), &request).await?;
    check_set_response(&resp, "CalendarEvent/set", "notCreated")
}

/// Cancel an event: set its status to `cancelled`, which prompts the server to
/// send cancellation notices to attendees.
pub async fn cancel_event(client: &JmapClient, event_id: &str) -> JmapResult<()> {
    let session = client.fetch_session().await?;
    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:calendars")
        .ok_or("no primary calendars account in session")?;

    let request_args = json!({
        "accountId": account_id,
        "update": { event_id: { "status": "cancelled" } }
    });
    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:calendars".to_string(),
        ],
        vec![(
            "CalendarEvent/set".to_string(),
            request_args,
            "cancel1".to_string(),
        )],
        None,
    );
    let resp = client.call(session.api_url.as_str(), &request).await?;
    check_set_response(&resp, "CalendarEvent/set", "notUpdated")
}

/// Fetch the account's default calendar id via a raw `Calendar/get` (the typed
/// client currently fails to deserialize some server calendar objects, so we
/// read the id straight from the JSON).
async fn default_calendar_id(
    client: &JmapClient,
    session: &jmap_base_client::Session,
    account_id: &str,
) -> JmapResult<String> {
    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:calendars".to_string(),
        ],
        vec![(
            "Calendar/get".to_string(),
            json!({ "accountId": account_id }),
            "cal_get1".to_string(),
        )],
        None,
    );
    let resp = client.call(session.api_url.as_str(), &request).await?;
    for (name, result, _) in &resp.method_responses {
        if name == "error" {
            let error_type = result["type"].as_str().unwrap_or("unknown");
            let description = result["description"].as_str().unwrap_or("");
            return Err(format!("Calendar/get error: {error_type} — {description}").into());
        }
        if name == "Calendar/get" {
            if let Some(list) = result["list"].as_array() {
                // Prefer a default calendar, then any writable one, then the first.
                let pick = list
                    .iter()
                    .find(|c| c["isDefault"].as_bool() == Some(true))
                    .or_else(|| {
                        list.iter()
                            .find(|c| c["myRights"]["mayAddItems"].as_bool() == Some(true))
                    })
                    .or_else(|| list.first());
                if let Some(id) = pick.and_then(|c| c["id"].as_str()) {
                    return Ok(id.to_string());
                }
            }
        }
    }
    Err("no calendar found on this account; pass --calendar <id>".into())
}

/// Derive an organizer address from the account's primary mail identity.
async fn default_identity_email(
    client: &JmapClient,
    session: &jmap_base_client::Session,
) -> JmapResult<String> {
    let sc = client.with_mail_session(session.clone());
    let identities = sc.identity_get(None, None).await?;
    identities
        .list
        .first()
        .map(|id| id.email.clone())
        .ok_or_else(|| "no mail identity found to use as organizer; pass --organizer".into())
}

/// Update a calendar event's title, start, and duration in place.
#[allow(dead_code)]
pub async fn update_event(
    client: &JmapClient,
    event_id: &str,
    title: &str,
    start: &str,
    duration: &str,
) -> JmapResult<()> {
    let session = client.fetch_session().await?;
    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:calendars")
        .ok_or("no primary calendars account in session")?;

    let mut patch = serde_json::Map::new();
    patch.insert("title".into(), json!(title));
    if !start.trim().is_empty() {
        patch.insert("start".into(), json!(start.trim()));
    }
    if !duration.trim().is_empty() {
        patch.insert("duration".into(), json!(duration.trim()));
    }

    let request_args = json!({
        "accountId": account_id,
        "update": { event_id: patch }
    });
    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:calendars".to_string(),
        ],
        vec![(
            "CalendarEvent/set".to_string(),
            request_args,
            "upd1".to_string(),
        )],
        None,
    );
    let resp = client.call(session.api_url.as_str(), &request).await?;
    check_set_response(&resp, "CalendarEvent/set", "notUpdated")
}

/// Delete a calendar event by id.
pub async fn delete_event(client: &JmapClient, event_id: &str) -> JmapResult<()> {
    let session = client.fetch_session().await?;
    let account_id = session
        .primary_account_id("urn:ietf:params:jmap:calendars")
        .ok_or("no primary calendars account in session")?;

    let request_args = json!({
        "accountId": account_id,
        "destroy": [event_id]
    });
    let request = jmap_types::JmapRequest::new(
        vec![
            "urn:ietf:params:jmap:core".to_string(),
            "urn:ietf:params:jmap:calendars".to_string(),
        ],
        vec![(
            "CalendarEvent/set".to_string(),
            request_args,
            "del1".to_string(),
        )],
        None,
    );
    let resp = client.call(session.api_url.as_str(), &request).await?;
    check_set_response(&resp, "CalendarEvent/set", "notDestroyed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_now_has_iso8601_shape() {
        let s = utc_now_iso8601();
        // e.g. 2026-07-13T09:41:00
        assert_eq!(s.len(), 19);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
    }
}
