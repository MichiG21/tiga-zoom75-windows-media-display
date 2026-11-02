//! Finding the keyboard's display among the Bluetooth devices.

use crate::failure::{Classify, Failed, Failure};
use crate::logging::{log, utc_now};
use crate::protocol::*;
use crate::sidecars::{run_id, sidecar_path, sidecar_value, write_sidecar};
use btleplug::api::{Central, Peripheral as _, ScanFilter};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use tokio::time::sleep;
use uuid::Uuid;

// ---- Device selection (issue #14) -------------------------------------------------
//
// Two peripherals near this keyboard advertise a name containing "zoom75"; only one
// exposes the media service. A name match therefore only makes a device a candidate.
// A candidate qualifies once it is connected and shows the media service with all four
// 1f4000xx characteristics. One that does not is disconnected and skipped for the rest
// of the run.
//
// Order: the device that last completed an upload (cached in `<stream>.device`) is
// tried as soon as the scan sees it. Everything else waits until the scan window has
// closed, so every candidate is logged, and is then tried by name: exactly
// "Zoom75 Tiga" first, other "zoom75" names next, "... BT<n>" names (the keyboard's
// typing connections) last; discovery order within each group.
//
// If every candidate seen was rejected, the scan runs one more window in case the
// keyboard was slow to advertise; it is still rejected-for-this-run devices only that
// are skipped.

pub(crate) const SERVICE_MEDIA: Uuid = Uuid::from_u128(0x1f40eaf8_aab4_14a3_f1ba_f61f35cddbaa);
pub(crate) const SCAN_WINDOW: Duration = Duration::from_secs(5);
/// If nothing at all has been seen when the window closes, keep scanning this long and
/// try the first candidate that appears. Right after a stack restart on 2026-10-15 the
/// keyboard stayed invisible for 20-38 s, so two 5 s scans failed with code 10.
pub(crate) const NOTHING_SEEN_LIMIT: Duration = Duration::from_secs(15);
pub(crate) const SCAN_POLL: Duration = Duration::from_millis(250);

pub(crate) struct Seen<D> {
    pub(crate) address: String,
    pub(crate) name: String,
    pub(crate) rssi: Option<i16>,
    pub(crate) dev: D,
}

pub(crate) enum Probe<L> {
    /// Connected and has the media service; `L` is what the upload needs from it.
    Qualified(L),
    /// Could not connect. The device may still be the right one.
    Unreachable(String),
    /// Connected, but it is not the keyboard's media endpoint. Disconnected again.
    Unqualified(String),
}

/// What device selection needs from Bluetooth. Faked in the tests.
pub(crate) trait Radio {
    type Dev;
    type Link;
    /// Every peripheral the running scan has seen so far.
    async fn visible(&mut self) -> Result<Vec<Seen<Self::Dev>>, String>;
    async fn probe(&mut self, dev: &Self::Dev) -> Probe<Self::Link>;
    /// Time since the scan started.
    fn elapsed(&self) -> Duration;
    async fn pause(&mut self, d: Duration);
    /// Start a fresh scan window: `elapsed` counts from zero again.
    async fn rescan(&mut self);
    /// Stop scanning. Called once a device has qualified (connected, media service
    /// present): from then on this run never needs to scan again, and a running scan
    /// would compete with the upload for radio time.
    async fn stop_scan(&mut self);
}

pub(crate) const SCAN_ROUNDS: u32 = 2;

pub(crate) struct Chosen<D, L> {
    pub(crate) address: String,
    pub(crate) name: String,
    pub(crate) dev: D,
    pub(crate) link: L,
}

pub(crate) fn same_address(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

pub(crate) fn is_candidate(name: &str, address: &str, cached: Option<&str>) -> bool {
    name == TARGET_NAME
        || name.to_ascii_lowercase().contains("zoom75")
        // The cached device qualifies to be tried even before its name has been seen.
        || cached.is_some_and(|c| same_address(c, address))
}

/// `"Zoom75 Tiga BT1"`, `"... BT2"`, ...: the keyboard's Bluetooth typing connections
/// (pairing slots), seen on hardware 2026-10-15. Still candidates, in case a firmware
/// variant names things differently, but tried last.
pub(crate) fn is_typing_slot_name(name: &str) -> bool {
    let Some((_, suffix)) = name.rsplit_once(' ') else { return false };
    let suffix = suffix.as_bytes();
    suffix.len() > 2
        && suffix[..2].eq_ignore_ascii_case(b"BT")
        && suffix[2..].iter().all(u8::is_ascii_digit)
}

/// Lower is tried first: the cached device, then the exact display name, then any
/// other "zoom75" name, then the typing slots.
pub(crate) fn candidate_tier(name: &str, address: &str, cached: Option<&str>) -> u8 {
    if cached.is_some_and(|c| same_address(c, address)) {
        0
    } else if name == TARGET_NAME {
        1
    } else if is_typing_slot_name(name) {
        3
    } else {
        2
    }
}

pub(crate) async fn select_device<R: Radio>(radio: &mut R, cached: Option<&str>) -> Result<Chosen<R::Dev, R::Link>, Failed> {
    let mut known: Vec<String> = Vec::new(); // candidate addresses in the order first seen
    let mut tried: Vec<String> = Vec::new();
    let mut outcomes: Vec<String> = Vec::new();
    let mut any_unreachable = false;
    let mut extended = false;
    let mut round = 1;

    loop {
        let mut untried = Vec::new();
        for s in radio.visible().await.class(Failure::Unreachable)? {
            if !is_candidate(&s.name, &s.address, cached) {
                continue;
            }
            if !known.iter().any(|k| same_address(k, &s.address)) {
                let is_cached = cached.is_some_and(|c| same_address(c, &s.address));
                log!(
                    "CANDIDATE name={:?} address={} rssi={}{}",
                    s.name,
                    s.address,
                    s.rssi.map(|r| r.to_string()).unwrap_or_else(|| "?".into()),
                    if is_cached { " (last device that completed an upload)" } else { "" }
                );
                known.push(s.address.clone());
            }
            if !tried.iter().any(|t| same_address(t, &s.address)) {
                untried.push(s);
            }
        }

        let window_closed = radio.elapsed() >= SCAN_WINDOW;
        let rank = |s: &Seen<R::Dev>| {
            let order = known.iter().position(|k| same_address(k, &s.address));
            (candidate_tier(&s.name, &s.address, cached), order)
        };
        untried.sort_by_key(rank);
        let next = match untried.into_iter().next() {
            Some(s) if window_closed || cached.is_some_and(|c| same_address(c, &s.address)) => Some(s),
            _ => None,
        };

        let Some(s) = next else {
            if window_closed && tried.is_empty() {
                if radio.elapsed() >= NOTHING_SEEN_LIMIT {
                    break;
                }
                if !extended {
                    extended = true;
                    log!(
                        "Nothing seen in {} s; scanning up to {} s (the keyboard can be invisible for a while after a restart)",
                        SCAN_WINDOW.as_secs(),
                        NOTHING_SEEN_LIMIT.as_secs()
                    );
                }
            } else if window_closed {
                if round >= SCAN_ROUNDS {
                    break;
                }
                round += 1;
                log!(
                    "RESCAN: all {} candidate(s) seen so far were rejected; scanning another {} s for others",
                    tried.len(),
                    SCAN_WINDOW.as_secs()
                );
                radio.rescan().await;
                continue;
            }
            radio.pause(SCAN_POLL).await;
            continue;
        };

        tried.push(s.address.clone());
        match radio.probe(&s.dev).await {
            Probe::Qualified(link) => {
                log!(
                    "SELECTED {} ({:?}) after trying {} candidate(s)",
                    s.address,
                    s.name,
                    tried.len()
                );
                radio.stop_scan().await;
                return Ok(Chosen { address: s.address, name: s.name, dev: s.dev, link });
            }
            Probe::Unreachable(why) => {
                log!("REJECTED {} for this run: {why}", s.address);
                outcomes.push(format!("{} {why}", s.address));
                any_unreachable = true;
            }
            Probe::Unqualified(why) => {
                log!("REJECTED {} for this run: {why}", s.address);
                outcomes.push(format!("{} {why}", s.address));
            }
        }
    }

    if tried.is_empty() {
        return Err(Failed::new(
            Failure::Unreachable,
            format!("{TARGET_NAME} not found (no candidate seen in a {} s scan)", NOTHING_SEEN_LIMIT.as_secs()),
        ));
    }
    // 11 only when every candidate was reached and none had the service. If any could
    // not be reached, that one may have been the keyboard.
    let kind = if any_unreachable { Failure::Unreachable } else { Failure::Gatt };
    Err(Failed::new(
        kind,
        format!(
            "tried {} candidate(s) over {round} scan(s), none qualified: {}",
            tried.len(),
            outcomes.join("; ")
        ),
    ))
}

/// Every service a device exposes, with its characteristics, for the rejection log.
/// The second "zoom75" device on the test PC is "Zoom75 Tiga BT1", the typing connection;
/// on 2026-10-15 it failed to connect, so what it exposes has still not been logged.
pub(crate) fn describe_services(services: &std::collections::BTreeSet<btleplug::api::Service>) -> String {
    if services.is_empty() {
        return "exposes no services".into();
    }
    let list: Vec<String> = services
        .iter()
        .map(|s| {
            let chars: Vec<String> = s.characteristics.iter().map(|c| c.uuid.to_string()).collect();
            format!("{} [{}]", s.uuid, chars.join(", "))
        })
        .collect();
    format!("exposes {} service(s): {}", services.len(), list.join("; "))
}

/// The four characteristics of the media service, or why this device lacks them.
pub(crate) fn media_link(services: &std::collections::BTreeSet<btleplug::api::Service>) -> Result<GattLink, String> {
    let Some(service) = services.iter().find(|s| s.uuid == SERVICE_MEDIA) else {
        return Err(format!("connected, but no media service {SERVICE_MEDIA}; {}", describe_services(services)));
    };
    let find = |uuid: Uuid| service.characteristics.iter().find(|c| c.uuid == uuid).cloned();
    match (find(CHAR_CONTROL), find(CHAR_NOTIFY), find(CHAR_BULK), find(CHAR_NOTIFY2)) {
        (Some(control), Some(notify1), Some(bulk), Some(notify2)) => Ok(GattLink { control, notify1, bulk, notify2 }),
        _ => {
            let missing: Vec<String> = [CHAR_CONTROL, CHAR_NOTIFY, CHAR_BULK, CHAR_NOTIFY2]
                .iter()
                .filter(|u| find(**u).is_none())
                .map(|u| u.to_string()[..8].to_string())
                .collect();
            Err(format!(
                "connected, media service present but missing {}; {}",
                missing.join(", "),
                describe_services(services)
            ))
        }
    }
}

#[derive(Debug)]
pub(crate) struct GattLink {
    pub(crate) control: btleplug::api::Characteristic,
    pub(crate) notify1: btleplug::api::Characteristic,
    pub(crate) bulk: btleplug::api::Characteristic,
    pub(crate) notify2: btleplug::api::Characteristic,
}

pub(crate) struct BleRadio<'a> {
    pub(crate) adapter: &'a btleplug::platform::Adapter,
    pub(crate) started: Instant,
}

impl Radio for BleRadio<'_> {
    type Dev = btleplug::platform::Peripheral;
    type Link = GattLink;

    async fn visible(&mut self) -> Result<Vec<Seen<Self::Dev>>, String> {
        let mut seen = Vec::new();
        for p in self.adapter.peripherals().await.map_err(|e| e.to_string())? {
            // One peripheral whose properties cannot be read is not a reason to stop.
            let Ok(props) = p.properties().await else { continue };
            let props = props.unwrap_or_default();
            seen.push(Seen {
                address: p.address().to_string(),
                name: props.local_name.unwrap_or_default(),
                rssi: props.rssi,
                dev: p,
            });
        }
        Ok(seen)
    }

    async fn probe(&mut self, p: &Self::Dev) -> Probe<GattLink> {
        if !p.is_connected().await.unwrap_or(false) {
            if let Err(e) = p.connect().await {
                return Probe::Unreachable(format!("connect failed: {e}"));
            }
        }
        let verdict = match p.discover_services().await {
            Ok(()) => media_link(&p.services()),
            Err(e) => Err(format!("connected, but service discovery failed: {e}")),
        };
        match verdict {
            Ok(link) => Probe::Qualified(link),
            Err(why) => {
                if let Err(e) = p.disconnect().await {
                    log!("warning: could not disconnect from {}: {e}", p.address());
                }
                Probe::Unqualified(why)
            }
        }
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    async fn pause(&mut self, d: Duration) {
        sleep(d).await;
    }

    async fn rescan(&mut self) {
        // Restarting the scan is best effort: the one already running keeps reporting.
        let _ = self.adapter.stop_scan().await;
        if let Err(e) = self.adapter.start_scan(ScanFilter::default()).await {
            log!("warning: could not restart the scan: {e}");
        }
        self.started = Instant::now();
    }

    async fn stop_scan(&mut self) {
        // Best effort: if it fails, the upload runs with the scan on, as it always did
        // before this was added.
        match self.adapter.stop_scan().await {
            Ok(()) => log!("Scan stopped: connected to the keyboard; this run needs no further scan"),
            Err(e) => log!("warning: could not stop the scan (the upload continues with it running): {e}"),
        }
    }
}

/// Address of the device that last completed an upload, from `<stream>.device`.
pub(crate) fn read_cached_device(stream_path: &Path) -> Option<String> {
    let text = fs::read_to_string(sidecar_path(stream_path, ".device")).ok()?;
    sidecar_value(&text, "address").filter(|a| !a.is_empty()).map(str::to_string)
}

pub(crate) fn write_cached_device(stream_path: &Path, address: &str, name: &str) {
    write_sidecar(
        stream_path,
        ".device",
        &format!("address={address}\nname={name}\nlast_completed_at={}\nrun_id={}\n", utc_now(), run_id()),
    );
}
