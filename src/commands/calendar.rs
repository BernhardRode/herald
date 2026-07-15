//! `herald calendar` subcommands — list calendars, list events, and create,
//! update, cancel, or delete events (including sending invites).

use clap::Subcommand;
use jmap_base_client::JmapClient;
use jmap_calendars_client::JmapCalendarsExt;
use jmap_calendars_types::{CalendarEventComparator, CalendarEventFilterCondition};

use crate::jmap::calendar::{self, utc_now_iso8601, NewEvent};
use crate::text::{sanitize_display, truncate_str};

#[derive(Debug, Subcommand)]
// This is a construct-once CLI arg enum; variant size is irrelevant.
#[allow(clippy::large_enum_variant)]
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
    /// Create, update, cancel, or delete a single event
    Event {
        #[command(subcommand)]
        action: EventCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum EventCommand {
    /// Create an event, optionally inviting attendees
    Create {
        /// Event title
        #[arg(long)]
        title: String,
        /// Local start time, e.g. 2026-07-16T08:30:00 (defaults to now)
        #[arg(long, default_value = "")]
        start: String,
        /// ISO 8601 duration, e.g. PT15M, PT1H (default PT1H)
        #[arg(long, default_value = "")]
        duration: String,
        /// Calendar ID to file the event under (default: account default)
        #[arg(long)]
        calendar: Option<String>,
        /// Attendee email to invite (repeatable). Sends an invitation.
        #[arg(long = "attendee")]
        attendees: Vec<String>,
        /// Organizer address (defaults to your primary mail identity)
        #[arg(long)]
        organizer: Option<String>,
        /// Event description
        #[arg(long)]
        description: Option<String>,
        /// Event location
        #[arg(long)]
        location: Option<String>,
        /// IANA time zone, e.g. Europe/Berlin
        #[arg(long)]
        timezone: Option<String>,
        /// Mark as an all-day event
        #[arg(long)]
        all_day: bool,
    },
    /// Update an event's title, start, or duration
    Update {
        /// Event ID
        #[arg(long)]
        id: String,
        /// New title
        #[arg(long, default_value = "")]
        title: String,
        /// New start time
        #[arg(long, default_value = "")]
        start: String,
        /// New duration
        #[arg(long, default_value = "")]
        duration: String,
    },
    /// Cancel an event and notify attendees
    Cancel {
        /// Event ID
        #[arg(long)]
        id: String,
    },
    /// Delete an event
    Delete {
        /// Event ID
        #[arg(long)]
        id: String,
    },
}

pub async fn handle(
    cmd: &CalendarCommand,
    client: &JmapClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        CalendarCommand::Calendars => list_calendars(client).await?,
        CalendarCommand::Events { limit, all } => list_events(client, *limit, *all).await?,
        CalendarCommand::Event { action } => handle_event(client, action).await?,
    }
    Ok(())
}

async fn handle_event(
    client: &JmapClient,
    action: &EventCommand,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        EventCommand::Create {
            title,
            start,
            duration,
            calendar,
            attendees,
            organizer,
            description,
            location,
            timezone,
            all_day,
        } => {
            let ev = NewEvent {
                title,
                start,
                duration,
                calendar_id: calendar.as_deref(),
                description: description.as_deref(),
                location: location.as_deref(),
                time_zone: timezone.as_deref(),
                all_day: *all_day,
                attendees,
                organizer: organizer.as_deref(),
            };
            calendar::create_event_full(client, &ev).await?;
            if attendees.is_empty() {
                println!("✓ Event created: {}", sanitize_display(title));
            } else {
                println!(
                    "✓ Event created and invited {} attendee(s): {}",
                    attendees.len(),
                    sanitize_display(title)
                );
            }
        }
        EventCommand::Update {
            id,
            title,
            start,
            duration,
        } => {
            calendar::update_event(client, id, title, start, duration).await?;
            println!("✓ Event updated: {}", sanitize_display(id));
        }
        EventCommand::Cancel { id } => {
            calendar::cancel_event(client, id).await?;
            println!("✓ Event cancelled: {}", sanitize_display(id));
        }
        EventCommand::Delete { id } => {
            calendar::delete_event(client, id).await?;
            println!("✓ Event deleted: {}", sanitize_display(id));
        }
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
