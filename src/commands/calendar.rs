//! `herald calendar` subcommands — list calendars, list events.

use clap::Subcommand;
use jmap_base_client::JmapClient;
use jmap_calendars_client::JmapCalendarsExt;
use jmap_calendars_types::{CalendarEventComparator, CalendarEventFilterCondition};

use crate::sanitize::sanitize_display;

#[derive(Debug, Subcommand)]
pub enum CalendarCommand {
    /// List calendars
    Calendars,
    /// List upcoming calendar events
    Events {
        /// Maximum number of events to display
        #[arg(long, default_value = "50")]
        limit: u32,
        /// Fetch all events (no time filter or limit)
        #[arg(long)]
        all: bool,
    },
}

pub async fn handle(
    cmd: &CalendarCommand,
    client: &JmapClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        CalendarCommand::Calendars => list_calendars(client).await?,
        CalendarCommand::Events { limit, all } => list_events(client, *limit, *all).await?,
    }
    Ok(())
}

async fn list_calendars(
    client: &JmapClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_calendars_session(session);
    let resp = sc.calendar_get(None, None).await?;

    println!("{:<12} {:<30} Color", "ID", "Name");
    println!("{}", "-".repeat(50));
    for cal in &resp.list {
        let id = cal.id.as_ref().map(|i| i.as_ref()).unwrap_or("-");
        let name = sanitize_display(&cal.name);
        let color = cal.color.as_deref().unwrap_or("");
        println!("{:<12} {:<30} {}", id, name, color);
    }
    Ok(())
}

/// Return the current UTC time as an ISO 8601 string (e.g. "2025-01-15T12:00:00Z")
/// suitable for the CalendarEvent/query `after` filter.
fn utc_now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Convert epoch seconds to a basic UTC datetime string.
    // We compute year/month/day/hour/min/sec from the epoch manually to avoid
    // pulling in a datetime crate just for this.
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

async fn list_events(
    client: &JmapClient,
    limit: u32,
    all: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = client.fetch_session().await?;
    let sc = client.with_calendars_session(session);

    // Build filter: unless --all, only show events from now onward
    let filter = if all {
        None
    } else {
        let mut f = CalendarEventFilterCondition::default();
        f.after = Some(utc_now_iso8601());
        Some(f)
    };

    // Sort by start ascending
    let mut comparator = CalendarEventComparator::default();
    comparator.property = "start".to_owned();
    comparator.is_ascending = true;
    let sort = [comparator];

    // Determine limit for the query
    let query_limit = if all { None } else { Some(u64::from(limit)) };

    // Query event IDs with filter and sort
    let query_resp = sc
        .calendar_event_query(filter.as_ref(), Some(&sort), Some(0), query_limit, None)
        .await?;

    if query_resp.ids.is_empty() {
        println!("No events found.");
        return Ok(());
    }

    // Fetch the actual event objects by their IDs
    let resp = sc
        .calendar_event_get(
            Some(&query_resp.ids),
            Some(&["id", "title", "start", "duration", "status", "utcStart"]),
            None,
        )
        .await?;

    println!(
        "{:<12} {:<20} {:<12} {:<10} Title",
        "ID", "Start", "Duration", "Status"
    );
    println!("{}", "-".repeat(80));
    for event in &resp.list {
        let id = event.id.as_ref().map(|i| i.as_ref()).unwrap_or("-");
        let title = sanitize_display(event.title.as_deref().unwrap_or("(no title)"));
        let start = event.start.as_deref().unwrap_or("-");
        let duration = event.duration.as_deref().unwrap_or("-");
        let status = event.status.as_deref().unwrap_or("-");
        println!(
            "{:<12} {:<20} {:<12} {:<10} {}",
            truncate_str(id, 10),
            truncate_str(start, 18),
            duration,
            status,
            title,
        );
    }
    Ok(())
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        truncated.push('…');
        truncated
    }
}
