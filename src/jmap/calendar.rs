//! Calendar operations and time helpers.

use jmap_base_client::JmapClient;
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
