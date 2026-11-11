use super::*;

#[test]
fn stop_request_is_a_sidecar() {
    let dir = std::env::temp_dir().join(format!("tiga_stop_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");
    assert!(!stop_requested(&stream));
    fs::write(sidecar_path(&stream, ".stop"), "requested_at=2026-10-16T20:00:00Z
").unwrap();
    assert!(stop_requested(&stream));
    // Other sidecars are not a stop request.
    fs::remove_file(sidecar_path(&stream, ".stop")).unwrap();
    fs::write(sidecar_path(&stream, ".session"), "frame_id=x
").unwrap();
    assert!(!stop_requested(&stream));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn median_and_p90() {
    assert!(median_p90(&[]).is_none());
    assert_eq!(median_p90(&[5.0]), Some((5.0, 5.0)));
    assert_eq!(median_p90(&[4.0, 1.0, 3.0, 2.0]), Some((2.5, 4.0)));
    // 1..=10: nearest-rank p90 is the 9th value.
    let v: Vec<f64> = (1..=10).rev().map(f64::from).collect();
    assert_eq!(median_p90(&v), Some((5.5, 9.0)));
    // 53 healthy blocks with one slow one: p90 stays healthy, max does not.
    let mut v = vec![150.0; 53];
    v[40] = 2400.0;
    assert_eq!(median_p90(&v), Some((150.0, 150.0)));
}

#[test]
fn summary_finds_the_slowest_block() {
    let t = UploadTimings {
        control1_ack_ms: Some(42.4),
        control2_ack_ms: None,
        block_ms: vec![150.0, 1800.0, 900.0, 1800.0],
    };
    let s = t.summary();
    assert_eq!((s.blocks, s.slowest_block), (4, 2));
    assert_eq!((s.median_ms, s.max_ms), (1350.0, 1800.0));
    let line = s.log_line("r-f0001");
    assert!(line.starts_with("TIMING frame=r-f0001 blocks_acked=4 median=1350ms p90=1800ms max=1800ms slowest=block 2"), "{line}");
    assert!(line.ends_with("control1_ack=42ms control2_ack=-ms"), "{line}");
    let side = s.sidecar_lines();
    assert_eq!(sidecar_value(&side, "slowest_block"), Some("2"));
    assert_eq!(sidecar_value(&side, "control2_ack_ms"), Some("-"));

    // Failed before any block: no panic, zeros.
    let s = UploadTimings::default().summary();
    assert_eq!((s.blocks, s.slowest_block, s.max_ms), (0, 0, 0.0));
}

#[test]
fn slow_block_threshold() {
    assert!(slow_block_line(7, 53, 1000.0, 3.0).is_none());
    assert!(slow_block_line(7, 53, 150.0, 3.0).is_none());
    assert_eq!(
        slow_block_line(7, 53, 1834.4, 4.214).as_deref(),
        Some("SLOW block 7/53 took 1834 ms (elapsed=4.21s)")
    );
}

#[test]
fn first_upload_warning_threshold() {
    // The slowing runs: 2.35, 2.42, 2.97 s. Healthy first uploads: 0.71-0.85 s.
    for s in [2.35, 2.42, 2.97] {
        assert!(first_upload_warning(true, 5, s).is_some(), "{s}");
    }
    for s in [0.71, 0.85, 1.5] {
        assert!(first_upload_warning(true, 5, s).is_none(), "{s}");
    }
    // Later uploads on a connection reach block 5 at 1.3-2.04 s when healthy.
    assert!(first_upload_warning(false, 5, 2.04).is_none());
    assert!(first_upload_warning(true, 4, 2.5).is_none());
    assert!(first_upload_warning(true, 6, 2.5).is_none());
    assert_eq!(
        first_upload_warning(true, 5, 2.42).as_deref(),
        Some("WARNING first upload is slow (block 5 at 2.42 s; healthy 0.7–0.85 s)")
    );
}

#[test]
fn delivery_record_carries_the_summary() {
    let dir = std::env::temp_dir().join(format!("tiga_timing_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");
    let t = UploadTimings { control1_ack_ms: Some(40.0), control2_ack_ms: Some(55.0), block_ms: vec![150.0, 160.0] };
    record_delivery(&stream, "r-f0002", "0badf00d", 8.7, Some(&t.summary()));
    let d = fs::read_to_string(sidecar_path(&stream, ".delivered")).unwrap();
    assert_eq!(sidecar_value(&d, "frame_id"), Some("r-f0002"));
    assert_eq!(sidecar_value(&d, "elapsed_s"), Some("8.70"));
    assert_eq!(sidecar_value(&d, "block_median_ms"), Some("155"));
    assert_eq!(sidecar_value(&d, "block_max_ms"), Some("160"));
    assert_eq!(sidecar_value(&d, "control1_ack_ms"), Some("40"));
    assert_eq!(sidecar_value(&d, "control2_ack_ms"), Some("55"));
    fs::remove_dir_all(&dir).unwrap();
}

fn notification(value: &[u8]) -> btleplug::api::ValueNotification {
    btleplug::api::ValueNotification { uuid: CHAR_NOTIFY, service_uuid: SERVICE_MEDIA, value: value.to_vec() }
}

#[test]
fn notification_log_skips_only_progress_and_complete() {
    assert!(!should_log_notification(PROGRESS));
    assert!(!should_log_notification(COMPLETE));
    for v in [C1_ACK_A, C1_ACK_B, C2_ACK_A, C2_ACK_B, &[0x88, 0, 0, 0, 0, 0, 0x03, 0x0b, 0x08, 0x06, 0x07][..], &[][..]] {
        assert!(should_log_notification(v), "{v:02x?}");
    }
}

const STATUS28: &[u8] = &[
    0x88, 0, 0, 0, 0, 0, 0x14, 0x2a, 0x03, 0x0d, 0, 0, 0x01, 0x40, 0, 0, 0x01, 0x40, 0, 0, 0x01, 0x40, 0, 0, 0x01,
    0x64, 0, 0,
];

#[test]
fn repeat_filter_logs_a_repeat_once_then_a_summary() {
    let mut f = RepeatFilter::new();
    let line = |v: &[u8]| notification_line(&CHAR_NOTIFY, v, "between uploads", Duration::ZERO);
    let first = f.note(&CHAR_NOTIFY, STATUS28, line(STATUS28), "T1");
    assert_eq!(first.len(), 1);
    assert!(first[0].starts_with("NOTIFY char=1f400002 len=28 value=88 00 00 00 00 00 14 2a 03 0d"), "{}", first[0]);
    // Identical ones are only counted.
    for t in ["T2", "T3", "T4"] {
        assert!(f.note(&CHAR_NOTIFY, STATUS28, line(STATUS28), t).is_empty());
    }
    // A different one: the summary first, then the new line, in full.
    let other = &[0x88, 0x01];
    let out = f.note(&CHAR_NOTIFY, other, line(other), "T5");
    assert_eq!(out.len(), 2, "{out:?}");
    assert_eq!(
        out[0],
        "NOTIFY repeated 3 more times char=1f400002 len=28 value=88 00 00 00 00 00 14 2a 03 0d 00 00 01 40 00 00 01 40 \
         00 00 01 40 00 00 01 64 00 00 first=T2 last=T4"
    );
    assert_eq!(out[1], line(other));
    // The status message after that is new again, so it is logged in full.
    assert_eq!(f.note(&CHAR_NOTIFY, STATUS28, line(STATUS28), "T6").len(), 1);
}

#[test]
fn repeat_filter_flushes_at_upload_boundaries_and_keeps_counting() {
    let mut f = RepeatFilter::new();
    let line = notification_line(&CHAR_NOTIFY, STATUS28, "between uploads", Duration::ZERO);
    assert_eq!(f.flush(), None, "nothing seen yet");
    f.note(&CHAR_NOTIFY, STATUS28, line.clone(), "T1");
    assert_eq!(f.flush(), None, "logged once, no repeats yet: no summary");
    f.note(&CHAR_NOTIFY, STATUS28, line.clone(), "T2");
    let s = f.flush().unwrap();
    assert!(s.starts_with("NOTIFY repeated 1 more time char=1f400002 len=28") && s.ends_with("first=T2 last=T2"), "{s}");
    assert_eq!(f.flush(), None, "flushing twice adds nothing");
    // After a summary (say at UPLOAD START) the same value is counted, not re-logged.
    assert!(f.note(&CHAR_NOTIFY, STATUS28, line.clone(), "T3").is_empty());
    assert!(f.note(&CHAR_NOTIFY, STATUS28, line.clone(), "T4").is_empty());
    assert!(f.flush().unwrap().contains("repeated 2 more times") );
}

#[test]
fn repeat_filter_tells_characteristics_apart() {
    let mut f = RepeatFilter::new();
    let v = &[0x01, 0x02];
    assert_eq!(f.note(&CHAR_NOTIFY, v, "a".into(), "T1"), ["a"]);
    // Same bytes on the other characteristic: different notification, logged.
    assert_eq!(f.note(&CHAR_NOTIFY2, v, "b".into(), "T2"), ["b"]);
    assert!(f.note(&CHAR_NOTIFY2, v, "b".into(), "T3").is_empty());
}

#[test]
fn notification_line_format() {
    let line = notification_line(&CHAR_NOTIFY, C2_ACK_B, "waiting for CONTROL2 ACK", Duration::from_millis(76));
    assert_eq!(
        line,
        "NOTIFY char=1f400002 len=11 value=88 00 00 00 00 00 03 0e 08 04 02 (CONTROL2 ACK B) \
         while=waiting for CONTROL2 ACK after=76ms"
    );
    let line = notification_line(&CHAR_NOTIFY2, &[0x88, 0xee], "waiting for PROGRESS of block 7/53", Duration::from_millis(2412));
    assert_eq!(line, "NOTIFY char=1f400004 len=2 value=88 ee while=waiting for PROGRESS of block 7/53 after=2412ms");
    assert_eq!(notification_line(&CHAR_NOTIFY, &[], "between uploads", Duration::ZERO), "NOTIFY char=1f400002 len=0 value= while=between uploads after=0ms");
}

#[tokio::test]
async fn listening_between_uploads_logs_and_survives_a_closed_stream() {
    let items = vec![notification(PROGRESS), notification(&[0x88, 0x01]), notification(COMPLETE), notification(C1_ACK_A)];
    let mut stream = futures::stream::iter(items);
    let mut ended = false;
    let started = Instant::now();
    let logged = listen_between_uploads(&mut stream, Duration::from_millis(60), &mut ended).await;
    assert_eq!(logged, 2, "PROGRESS and COMPLETE are not logged");
    assert!(ended, "the end of the stream is noticed");
    assert!(started.elapsed() >= Duration::from_millis(60), "still waits the full time");
    // Once ended, later calls just wait.
    let again = Instant::now();
    assert_eq!(listen_between_uploads(&mut stream, Duration::from_millis(30), &mut ended).await, 0);
    assert!(again.elapsed() >= Duration::from_millis(30));
}

#[tokio::test]
async fn listening_returns_on_time_with_an_open_quiet_stream() {
    let mut quiet = futures::stream::pending::<btleplug::api::ValueNotification>();
    let mut ended = false;
    let started = Instant::now();
    assert_eq!(listen_between_uploads(&mut quiet, Duration::from_millis(40), &mut ended).await, 0);
    assert!(!ended && started.elapsed() >= Duration::from_millis(40) && started.elapsed() < Duration::from_secs(2));
}

#[test]
fn crc32_matches_zlib() {
    // zlib.crc32(b"123456789") == 0xcbf43926
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn utc_now_is_iso8601() {
    let s = utc_now();
    assert_eq!(s.len(), 20, "{s}");
    assert!(s.ends_with('Z') && s.as_bytes()[10] == b'T', "{s}");
    assert!(s.as_str() >= "2024-01-01T00:00:00Z", "{s}");
}

#[test]
fn sidecar_lookup() {
    let text = "frame_id=renderer-x-f0003\ncrc32=DEADBEEF\nreason=a=b\n";
    assert_eq!(sidecar_value(text, "frame_id"), Some("renderer-x-f0003"));
    assert_eq!(sidecar_value(text, "reason"), Some("a=b"));
    assert_eq!(sidecar_value(text, "missing"), None);
}

#[test]
fn frame_id_requires_matching_crc() {
    let dir = std::env::temp_dir().join(format!("tiga_sidecar_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");
    assert_eq!(frame_id_for(&stream, "00000000"), "unknown(no-sidecar)");
    fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0001\ncrc32=0badf00d\n").unwrap();
    assert_eq!(frame_id_for(&stream, "0badf00d"), "r-f0001");
    assert_eq!(frame_id_for(&stream, "11111111"), "unknown(sidecar-is-r-f0001)");
    record_delivery(&stream, "r-f0001", "0badf00d", 7.5, None);
    let delivered = fs::read_to_string(sidecar_path(&stream, ".delivered")).unwrap();
    assert_eq!(sidecar_value(&delivered, "frame_id"), Some("r-f0001"));
    assert_eq!(sidecar_value(&delivered, "elapsed_s"), Some("7.50"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn identical_rerender_is_recorded_once() {
    let dir = std::env::temp_dir().join(format!("tiga_redisplay_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");
    let mut last_id = "r-f0001".to_string();

    // Sidecar still names the uploaded frame: nothing to record.
    fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0001\ncrc32=0badf00d\n").unwrap();
    assert!(!note_already_displayed(&stream, "0badf00d", &mut last_id));

    // Same bytes, new frame ID: recorded once.
    fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0002\ncrc32=0badf00d\n").unwrap();
    assert!(note_already_displayed(&stream, "0badf00d", &mut last_id));
    assert!(!note_already_displayed(&stream, "0badf00d", &mut last_id));
    let delivered = fs::read_to_string(sidecar_path(&stream, ".delivered")).unwrap();
    assert_eq!(sidecar_value(&delivered, "frame_id"), Some("r-f0002"));

    // Sidecar already describes a different (not yet uploaded) stream: not recorded.
    fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0003\ncrc32=11111111\n").unwrap();
    assert!(!note_already_displayed(&stream, "0badf00d", &mut last_id));

    // Nothing uploaded yet: never recorded.
    assert!(!note_already_displayed(&stream, "", &mut String::new()));
    fs::remove_dir_all(&dir).unwrap();
}

fn hold_line(gate: Gate) -> String {
    match gate {
        Gate::Hold(Some(line)) => line,
        other => panic!("expected a hold with a line, got {other:?}"),
    }
}

#[test]
fn stream_without_sidecar_is_held_until_the_renderer_writes_one() {
    let dir = std::env::temp_dir().join(format!("tiga_gate_hold_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");
    let mut gate = RendererGate::new(false);

    // Phase 8: the seeded template, before the renderer's first frame. Logged once.
    let line = hold_line(gate.check(&stream, "57e8e0cf"));
    assert!(line.starts_with("UPLOAD HELD frame=unknown(no-sidecar) crc32=57e8e0cf: no "), "{line}");
    assert!(line.contains("s.bin.frame"), "{line}");
    assert_eq!(gate.check(&stream, "57e8e0cf"), Gate::Hold(None));
    assert_eq!(gate.check(&stream, "57e8e0cf"), Gate::Hold(None));

    // The sidecar appears: one more look (the stream follows it), then upload.
    fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0001\ncrc32=056ed3e6\n").unwrap();
    let line = hold_line(gate.check(&stream, "57e8e0cf"));
    assert!(line.starts_with("RENDERER sidecar appeared after "), "{line}");
    assert_eq!(gate.check(&stream, "056ed3e6"), Gate::Upload);
    assert_eq!(gate.check(&stream, "d91aebb3"), Gate::Upload);

    // A sidecar that goes away later means holding again, with a new line.
    fs::remove_file(sidecar_path(&stream, ".frame")).unwrap();
    hold_line(gate.check(&stream, "d91aebb3"));
    assert_eq!(gate.check(&stream, "d91aebb3"), Gate::Hold(None));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mismatched_sidecar_is_uploaded_as_before() {
    let dir = std::env::temp_dir().join(format!("tiga_gate_mismatch_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");
    // A sidecar left from an earlier frame: not held, and named as before.
    fs::write(sidecar_path(&stream, ".frame"), "frame_id=r-f0009\ncrc32=0badf00d\n").unwrap();
    let mut gate = RendererGate::new(false);
    assert_eq!(gate.check(&stream, "57e8e0cf"), Gate::Upload);
    assert_eq!(frame_id_for(&stream, "57e8e0cf"), "unknown(sidecar-is-r-f0009)");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn allow_no_sidecar_uploads_the_template() {
    let dir = std::env::temp_dir().join(format!("tiga_gate_allow_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");
    let mut gate = RendererGate::new(true);
    assert_eq!(gate.check(&stream, "57e8e0cf"), Gate::Upload);
    assert_eq!(frame_id_for(&stream, "57e8e0cf"), "unknown(no-sidecar)");
    fs::remove_dir_all(&dir).unwrap();

    let args: Vec<String> = ["--confirm-flow-watch", "--allow-no-sidecar"].iter().map(|s| s.to_string()).collect();
    assert!(parse_opts(&args).unwrap().allow_no_sidecar);
    assert!(!parse_opts(&args[..1]).unwrap().allow_no_sidecar);
}

#[test]
fn tracked_template_builds_expected_chunks() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("extracted")
        .join("session_01_bulk_stream.redgreen-backup.bin");
    let stream = fs::read(path).unwrap();
    assert_eq!(build_chunks(&stream, false).unwrap().len(), CAPTURED_BULK_WRITES);
    assert_eq!(build_chunks(&stream, true).unwrap().len(), CAPTURED_BULK_WRITES);
}

#[test]
fn exit_codes_are_stable() {
    // The supervisors depend on these numbers.
    let codes: Vec<u8> = [
        Failure::Unreachable,
        Failure::Gatt,
        Failure::Interrupted,
        Failure::NoCompletion,
        Failure::Fatal,
    ]
    .iter()
    .map(|f| f.code())
    .collect();
    assert_eq!(codes, [10, 11, 20, 21, 30]);
    assert!(Failure::Interrupted.meaning().contains("battery for at least 30 s"));
    assert!(Failure::NoCompletion.meaning().contains("battery for at least 30 s"));
    assert!(!Failure::Interrupted.meaning().contains("power switch,"));
    assert!(Failure::Unreachable.meaning().contains("safe to retry"));
}

#[test]
fn stall_signatures_from_the_record() {
    let healthy = |n: usize| vec![155.0; n];
    // 2026-10-18 13:20 / 13:41 / 14:03: a healthy pace, then nothing.
    assert_eq!(stall_signature(&healthy(26), true), Signature::Sudden);
    assert_eq!(stall_signature(&healthy(51), true), Signature::Sudden);
    // 2026-10-16 20:58: a later upload's slow early block (802 ms) is not slowness.
    assert_eq!(stall_signature(&[150.0, 160.0, 802.0, 170.0, 188.0], true), Signature::Sudden);
    // 2026-10-18 14:09: block 6 took 1860 ms, then the stall: one slow block, sudden.
    assert_eq!(stall_signature(&[170.0, 160.0, 175.0, 168.0, 180.0, 1860.0], true), Signature::Sudden);
    // 2026-10-15 22:44: at least four of blocks 6-10 took ~1.8-2.5 s.
    let mut b = healthy(5);
    b.extend([1900.0, 2100.0, 1800.0, 2300.0, 1500.0]);
    assert_eq!(stall_signature(&b, true), Signature::Slowing);
    // 2026-10-16 18:49: 35 blocks at 1.2-2.5 s.
    let mut b = healthy(5);
    b.extend(vec![1800.0; 35]);
    assert_eq!(stall_signature(&b, true), Signature::Slowing);
    // Two slow blocks are already sustained.
    let mut b = healthy(20);
    b.extend([1200.0, 1300.0]);
    assert_eq!(stall_signature(&b, true), Signature::Slowing);
    // No pace to judge, or not a PROGRESS timeout: unclassified (the supervisor halts).
    assert_eq!(stall_signature(&[], true), Signature::Unclassified);
    assert_eq!(stall_signature(&healthy(30), false), Signature::Unclassified);
}

#[test]
fn a_stall_on_a_short_sample_is_sudden_whatever_its_pace() {
    // Halted as "slowing" before the minimum existed; all from ble.log.
    assert_eq!(stall_signature(&[993.0], true), Signature::Sudden, "2026-11-02 17:14");
    assert_eq!(stall_signature(&[987.0], true), Signature::Sudden, "2026-11-02 18:33");
    assert_eq!(stall_signature(&[1865.0, 173.0], true), Signature::Sudden, "2026-11-02 16:36");
    assert_eq!(stall_signature(&[778.0, 226.0], true), Signature::Sudden, "2026-10-25 11:04");
    // Healthy uploads start this slowly (2026-11-02 18:18 and 18:33): had they stalled here.
    assert_eq!(stall_signature(&[1540.0], true), Signature::Sudden);
    assert_eq!(stall_signature(&[420.0, 1403.0], true), Signature::Sudden);
    // The rule is about the sample, not the pace: 4 very slow blocks are still too few.
    assert_eq!(stall_signature(&[2000.0; 4], true), Signature::Sudden);
}

#[test]
fn the_pace_is_judged_from_the_fifth_block() {
    assert_eq!(SLOWING_MIN_BLOCKS, 5);
    assert_eq!(stall_signature(&[2000.0; 5], true), Signature::Slowing, "median");
    // Two blocks over 1 s among five, with a healthy median.
    assert_eq!(stall_signature(&[150.0, 160.0, 170.0, 1100.0, 1200.0], true), Signature::Slowing);
    assert_eq!(stall_signature(&[150.0, 160.0, 170.0, 180.0, 1200.0], true), Signature::Sudden);
}

#[test]
fn real_slowdowns_from_the_log_still_halt() {
    // 2026-11-02 17:44: 21 blocks, median 1454 ms. Blocks 1-4 took about 770 ms in all; 11 and
    // 13 are worked out from the elapsed values.
    let b = [
        190.0, 190.0, 195.0, 195.0, 2288.0, 1732.0, 1792.0, 2183.0, 1478.0, 2152.0, 973.0, 1447.0, 857.0, 1733.0, 1920.0,
        1102.0, 2115.0, 1283.0, 1305.0, 1454.0, 1545.0,
    ];
    assert_eq!(median_p90(&b).unwrap().0, 1454.0);
    assert_eq!(stall_signature(&b, true), Signature::Slowing);
    // 2026-10-22 22:24: 7 blocks, median 645 ms, blocks 5-7 over 1 s.
    assert_eq!(stall_signature(&[150.0, 160.0, 170.0, 645.0, 1845.0, 2355.0, 1470.0], true), Signature::Slowing);
    // 2026-10-25 10:36: 7 blocks, median 150 ms, blocks 6 and 7 over 1 s.
    assert_eq!(stall_signature(&[150.0, 150.0, 150.0, 150.0, 150.0, 1612.0, 1568.0], true), Signature::Slowing);
}

#[test]
fn the_minimum_changes_nothing_else() {
    // No block ACKed, or not a PROGRESS timeout: still unclassified, so still a halt.
    assert_eq!(stall_signature(&[], true), Signature::Unclassified);
    assert_eq!(stall_signature(&[993.0], false), Signature::Unclassified);
    assert_eq!(stall_signature(&[2000.0; 21], false), Signature::Unclassified);
    // Short and healthy was sudden before and is sudden now.
    assert_eq!(stall_signature(&[150.0, 160.0], true), Signature::Sudden);
}

#[test]
fn control2_is_the_boundary() {
    // Anything before CONTROL2 is retryable, however far it got.
    assert_eq!(classify_upload_failure(false, false), Failure::Unreachable);
    // From CONTROL2 on, a session may be open, even with zero bulk writes.
    assert_eq!(classify_upload_failure(true, false), Failure::Interrupted);
    assert_eq!(classify_upload_failure(true, true), Failure::NoCompletion);
}

#[test]
fn upload_progress_reports_block_only_after_control2() {
    let path = Path::new("unused.bin");
    let mut p = UploadProgress {
        stream_path: path,
        frame_id: "r-f0007",
        total_blocks: 53,
        control2_attempted: false,
        last_acked_block: 0,
        all_blocks_acked: false,
    };
    let f = p.fail("C1 ACK timed out");
    assert_eq!((f.kind, f.last_acked_block), (Failure::Unreachable, None));
    assert_eq!(f.frame_id.as_deref(), Some("r-f0007"));

    p.control2_attempted = true;
    let f = p.fail("C2 ACK timed out");
    assert_eq!((f.kind, f.last_acked_block), (Failure::Interrupted, Some(0)));
    assert!(f.detail.contains("last ACKed block 0/53"), "{}", f.detail);

    p.last_acked_block = 17;
    let f = p.fail("notification stream ended");
    assert_eq!((f.kind, f.last_acked_block), (Failure::Interrupted, Some(17)));

    p.last_acked_block = 53;
    p.all_blocks_acked = true;
    assert_eq!(p.fail("no completion").kind, Failure::NoCompletion);
}

#[test]
fn session_marker_and_exit_record() {
    let dir = std::env::temp_dir().join(format!("tiga_session_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");

    assert!(read_session_marker(&stream).is_none());
    write_session_marker(&stream, "r-f0003", 12, 53);
    let m = read_session_marker(&stream).unwrap();
    assert_eq!(sidecar_value(&m, "frame_id"), Some("r-f0003"));
    assert_eq!(sidecar_value(&m, "last_acked_block"), Some("12"));
    clear_session_marker(&stream);
    assert!(read_session_marker(&stream).is_none());
    clear_session_marker(&stream); // absent is fine

    let f = Failed {
        kind: Failure::Interrupted,
        detail: "line one\nline two".into(),
        frame_id: Some("r-f0003".into()),
        last_acked_block: Some(12),
        progress_timeout: true,
        signature: Some(Signature::Sudden),
    };
    record_exit(&stream, &f);
    let e = fs::read_to_string(sidecar_path(&stream, ".exit")).unwrap();
    assert_eq!(sidecar_value(&e, "code"), Some("20"));
    assert_eq!(sidecar_value(&e, "signature"), Some("sudden"));
    assert_eq!(sidecar_value(&e, "detail"), Some("line one line two"));
    assert_eq!(sidecar_value(&e, "last_acked_block"), Some("12"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bad_numeric_argument_is_fatal() {
    let args: Vec<String> = ["--intra-block-ms", "fast"].iter().map(|s| s.to_string()).collect();
    let Err(f) = parse_opts(&args) else { panic!("accepted a bad number") };
    assert_eq!(f.kind, Failure::Fatal);
    let ok: Vec<String> = ["--stream", "x.bin", "--confirm-flow-watch"].iter().map(|s| s.to_string()).collect();
    let o = parse_opts(&ok).unwrap();
    assert!(o.live && !o.once && o.intra_block_ms == 2 && o.block_timeout_ms == 2_500);
}

#[test]
fn captured_stream_control1_unchanged() {
    assert_eq!(control1_for(STREAM_LEN), CONTROL1);
}

fn synthetic_stream(len: usize) -> Vec<u8> {
    let tpl = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("extracted")
            .join("session_01_bulk_stream.redgreen-backup.bin"),
    )
    .unwrap();
    let mut s = tpl[..33].to_vec();
    s.resize(len - 4, 0);
    s.extend_from_slice(&[0xfc, 0xff, 0, 0]);
    let body = len - 8;
    s[4] = (body >> 16) as u8;
    s[5] = (body >> 8) as u8;
    s[6] = body as u8;
    s
}

#[test]
fn single_frame_stream_needs_flag() {
    for len in SINGLE_FRAME_STREAM_LENS {
        let s = synthetic_stream(len);
        assert!(validate_stream(&s, false).is_err(), "{len} accepted without flag");
        assert!(validate_stream(&s, true).is_ok(), "{len} rejected with flag");
    }
    // Other sizes stay rejected even with the flag.
    assert!(validate_stream(&synthetic_stream(150_000), true).is_err());
}

/// Validates real tiga_dlx.py output when TIGA_SINGLE_FRAME_SAMPLE names a file
/// (comma-separated for several); skipped otherwise.
#[test]
fn python_built_single_frame_sample() {
    let Ok(paths) = std::env::var("TIGA_SINGLE_FRAME_SAMPLE") else { return };
    for path in paths.split(',') {
        let s = fs::read(path).unwrap();
        let chunks = build_chunks(&s, true).unwrap();
        assert!(validate_stream(&s, false).is_err());
        println!(
            "{path}: {} bytes, {} writes, {} blocks, CONTROL1 len bytes {:02x?}",
            s.len(),
            chunks.len(),
            chunks.iter().filter(|c| c.ends_4k_block).count(),
            &control1_for(s.len())[18..21]
        );
    }
}

// ---- Device selection (issue #14) ----

const GOOD: &str = "07:01:88:B3:66:1B";
const BAD: &str = "1B:A3:A9:10:13:5D";

/// A scan on a virtual clock. Each device appears at a given time; probing takes 1 s.
struct FakeRadio {
    devices: Vec<(Duration, &'static str, &'static str)>, // appears_at, address, name
    qualified: Vec<&'static str>,
    unreachable: Vec<&'static str>,
    clock: Duration,
    scan_started: Duration,
    rescans: u32,
    scanning: bool,
    scan_stops: u32,
    probed: Vec<String>,
}

impl FakeRadio {
    fn new(devices: &[(u64, &'static str)], qualified: &[&'static str]) -> Self {
        FakeRadio {
            devices: devices.iter().map(|&(ms, a)| (Duration::from_millis(ms), a, "Zoom75 Tiga")).collect(),
            qualified: qualified.to_vec(),
            unreachable: Vec::new(),
            clock: Duration::ZERO,
            scan_started: Duration::ZERO,
            rescans: 0,
            scanning: true,
            scan_stops: 0,
            probed: Vec::new(),
        }
    }
}

impl Radio for FakeRadio {
    type Dev = &'static str;
    type Link = ();

    async fn visible(&mut self) -> Result<Vec<Seen<&'static str>>, String> {
        Ok(self
            .devices
            .iter()
            .filter(|d| d.0 <= self.clock)
            .map(|&(_, address, name)| Seen { address: address.into(), name: name.into(), rssi: Some(-60), dev: address })
            .collect())
    }

    async fn probe(&mut self, dev: &&'static str) -> Probe<()> {
        self.probed.push(dev.to_string());
        self.clock += Duration::from_secs(1);
        if self.unreachable.contains(dev) {
            Probe::Unreachable("connect failed".into())
        } else if self.qualified.contains(dev) {
            Probe::Qualified(())
        } else {
            Probe::Unqualified("connected, but no media service".into())
        }
    }

    fn elapsed(&self) -> Duration {
        self.clock - self.scan_started
    }

    async fn pause(&mut self, d: Duration) {
        self.clock += d;
    }

    async fn rescan(&mut self) {
        self.rescans += 1;
        self.scan_started = self.clock;
        self.scanning = true;
    }

    async fn stop_scan(&mut self) {
        self.scan_stops += 1;
        self.scanning = false;
    }
}

/// The names seen on hardware 2026-10-15: GOOD is the display, BAD the BT1 typing slot.
fn real_names(devices: &[(u64, &'static str)], qualified: &[&'static str]) -> FakeRadio {
    let mut r = FakeRadio::new(&[], qualified);
    for &(ms, a) in devices {
        let name = if a == BAD { "Zoom75 Tiga BT1" } else { "Zoom75 Tiga" };
        r.devices.push((Duration::from_millis(ms), a, name));
    }
    r
}

fn select(radio: &mut FakeRadio, cached: Option<&str>) -> Result<String, Failed> {
    futures::executor::block_on(select_device(radio, cached)).map(|c| c.address)
}

#[test]
fn scan_stops_once_a_device_qualifies() {
    // Selected straight from the cache.
    let mut r = FakeRadio::new(&[(0, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, Some(GOOD)).unwrap(), GOOD);
    assert_eq!((r.scanning, r.scan_stops), (false, 1));
    // A rejected candidate does not stop it; the one that qualifies does, once.
    let mut r = FakeRadio::new(&[(0, BAD), (300, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, None).unwrap(), GOOD);
    assert_eq!((r.scanning, r.scan_stops), (false, 1));
    // Found only after a rescan: still scanning while it is needed, stopped after.
    let mut r = FakeRadio::new(&[(0, BAD), (8000, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, None).unwrap(), GOOD);
    assert_eq!((r.rescans, r.scanning, r.scan_stops), (1, false, 1));
}

#[test]
fn scan_keeps_running_when_nothing_qualifies() {
    // The run then ends with 10 or 11; the next run's scan starts fresh.
    let mut r = FakeRadio::new(&[(0, BAD)], &[GOOD]);
    let _ = select(&mut r, None).unwrap_err();
    assert_eq!((r.scanning, r.scan_stops), (true, 0));
    let mut r = FakeRadio::new(&[], &[]);
    let _ = select(&mut r, None).unwrap_err();
    assert_eq!(r.scan_stops, 0);
}

#[test]
fn wrong_device_first_is_skipped() {
    let mut r = FakeRadio::new(&[(0, BAD), (300, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, None).unwrap(), GOOD);
    assert_eq!(r.probed, [BAD, GOOD]);
    // Nothing is tried before the window closes, so every candidate gets logged.
    assert!(r.clock >= SCAN_WINDOW);
}

#[test]
fn typing_slot_names() {
    for n in ["Zoom75 Tiga BT1", "Zoom75 Tiga BT2", "Zoom75 Tiga bt12"] {
        assert!(is_typing_slot_name(n), "{n}");
    }
    for n in ["Zoom75 Tiga", "Zoom75 Tiga BT", "Zoom75 Tiga BTx", "BT1", "Zoom75 Tiga 2.4G", ""] {
        assert!(!is_typing_slot_name(n), "{n}");
    }
}

#[test]
fn exact_name_beats_typing_slot_seen_first() {
    // 23:06 on hardware: BT1 seen first cost an 11 s failed connect.
    let mut r = real_names(&[(0, BAD), (1000, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, None).unwrap(), GOOD);
    assert_eq!(r.probed, [GOOD]);
    // If the display name does not qualify, the typing slot is still tried, second.
    let mut r = real_names(&[(0, BAD), (1000, GOOD)], &[BAD]);
    assert_eq!(select(&mut r, None).unwrap(), BAD);
    assert_eq!(r.probed, [GOOD, BAD]);
}

#[test]
fn other_zoom75_name_comes_between() {
    let odd = "AA:BB:CC:DD:EE:FF";
    let mut r = real_names(&[(0, BAD), (100, GOOD)], &[]);
    r.devices.push((Duration::from_millis(50), odd, "ZOOM75 TIGA-EU"));
    let _ = select(&mut r, None).unwrap_err();
    assert_eq!(r.probed, [GOOD, odd, BAD]);
}

#[test]
fn only_a_typing_slot_is_still_probed() {
    let mut r = real_names(&[(0, BAD)], &[BAD]);
    assert_eq!(select(&mut r, None).unwrap(), BAD);
    assert_eq!(r.probed, [BAD]);
}

#[test]
fn cache_beats_the_exact_name() {
    // Even a cached typing slot is tried before the display name.
    let mut r = real_names(&[(0, GOOD), (300, BAD)], &[BAD]);
    assert_eq!(select(&mut r, Some(BAD)).unwrap(), BAD);
    assert_eq!(r.probed, [BAD]);
    assert!(r.clock < SCAN_WINDOW);
}

#[test]
fn only_the_wrong_device_gives_11() {
    let mut r = FakeRadio::new(&[(0, BAD)], &[GOOD]);
    let f = select(&mut r, None).unwrap_err();
    assert_eq!(f.kind, Failure::Gatt);
    assert!(f.detail.contains("tried 1 candidate(s) over 2 scan(s)"), "{}", f.detail);
    assert!(f.detail.contains(BAD), "{}", f.detail);
    assert_eq!(r.probed, [BAD]); // tried once, not again on later polls or the rescan
    assert_eq!(r.rescans, 1);
}

#[test]
fn rescan_finds_a_device_that_advertised_late() {
    // The good keyboard only shows up after the first window has closed.
    let mut r = FakeRadio::new(&[(0, BAD), (8000, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, None).unwrap(), GOOD);
    assert_eq!(r.probed, [BAD, GOOD]);
    assert_eq!(r.rescans, 1);
}

#[test]
fn neither_of_two_qualifies() {
    let other = "AA:BB:CC:DD:EE:FF";
    let mut r = FakeRadio::new(&[(0, BAD), (1000, other)], &[]);
    let f = select(&mut r, None).unwrap_err();
    assert_eq!(f.kind, Failure::Gatt);
    assert!(f.detail.contains("tried 2 candidate(s)"), "{}", f.detail);
    assert_eq!(r.probed, [BAD, other]);
}

#[test]
fn cached_device_is_tried_first_without_waiting() {
    let mut r = FakeRadio::new(&[(0, BAD), (500, GOOD)], &[GOOD]);
    // Cached address in a different case still matches.
    assert_eq!(select(&mut r, Some(&GOOD.to_ascii_lowercase())).unwrap(), GOOD);
    assert_eq!(r.probed, [GOOD]);
    assert!(r.clock < SCAN_WINDOW, "waited for the whole window: {:?}", r.clock);
}

#[test]
fn cached_device_gone_falls_back_to_full_scan() {
    let gone = "11:22:33:44:55:66";
    let mut r = FakeRadio::new(&[(0, BAD), (200, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, Some(gone)).unwrap(), GOOD);
    assert_eq!(r.probed, [BAD, GOOD]);
}

#[test]
fn cached_device_no_longer_qualifying_falls_back() {
    let mut r = FakeRadio::new(&[(0, BAD), (200, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, Some(BAD)).unwrap(), GOOD);
    assert_eq!(r.probed, [BAD, GOOD]);
}

#[test]
fn cached_device_counts_even_without_a_name() {
    let mut r = FakeRadio::new(&[], &[GOOD]);
    r.devices.push((Duration::ZERO, GOOD, ""));
    r.devices.push((Duration::ZERO, "99:99:99:99:99:99", "Some Mouse"));
    assert_eq!(select(&mut r, Some(GOOD)).unwrap(), GOOD);
    assert_eq!(r.probed, [GOOD]);
}

#[test]
fn nothing_found_is_10() {
    let mut r = FakeRadio::new(&[], &[]);
    r.devices.push((Duration::ZERO, "99:99:99:99:99:99", "Some Mouse"));
    let f = select(&mut r, None).unwrap_err();
    assert_eq!(f.kind, Failure::Unreachable);
    assert!(f.detail.contains("not found"), "{}", f.detail);
    assert!(r.probed.is_empty());
    // Nothing to reject, so no rescan: the supervisor's retry covers this case.
    assert_eq!(r.rescans, 0);
    // But it kept looking for the longer limit, not just one window.
    assert!(r.clock >= NOTHING_SEEN_LIMIT, "{:?}", r.clock);
    assert!(f.detail.contains("15 s scan"), "{}", f.detail);
}

#[test]
fn keyboard_appearing_late_is_found_in_the_first_scan() {
    // After a restart the keyboard may take a while to advertise again.
    let mut r = real_names(&[(9000, BAD), (9000, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, None).unwrap(), GOOD);
    assert_eq!(r.probed, [GOOD]);
    assert_eq!(r.rescans, 0);
    assert!(r.clock < Duration::from_secs(11), "{:?}", r.clock);
    // Same with the cache.
    let mut r = real_names(&[(12_000, GOOD)], &[GOOD]);
    assert_eq!(select(&mut r, Some(GOOD)).unwrap(), GOOD);
}

#[test]
fn rejected_candidates_do_not_get_the_longer_wait() {
    // One window, one rescan window, done: the extension is for "nothing seen" only.
    let mut r = real_names(&[(0, BAD)], &[GOOD]);
    let _ = select(&mut r, None).unwrap_err();
    assert!(r.clock < Duration::from_secs(12), "{:?}", r.clock);
}

#[test]
fn unreachable_candidate_makes_it_10_not_11() {
    let mut r = FakeRadio::new(&[(0, BAD), (0, GOOD)], &[GOOD]);
    r.unreachable.push(GOOD);
    let f = select(&mut r, None).unwrap_err();
    assert_eq!(f.kind, Failure::Unreachable);
    assert!(f.detail.contains("tried 2 candidate(s)"), "{}", f.detail);
}

#[test]
fn media_link_requires_service_and_all_four() {
    use btleplug::api::{CharPropFlags, Characteristic, Service};
    use std::collections::BTreeSet;
    let chr = |uuid| Characteristic {
        uuid,
        service_uuid: SERVICE_MEDIA,
        properties: CharPropFlags::empty(),
        descriptors: BTreeSet::new(),
    };
    let service = |uuid, chars: &[Uuid]| Service {
        uuid,
        primary: true,
        characteristics: chars.iter().map(|&u| chr(u)).collect(),
    };
    let all = [CHAR_CONTROL, CHAR_NOTIFY, CHAR_BULK, CHAR_NOTIFY2];

    let ok: BTreeSet<_> = [service(SERVICE_MEDIA, &all)].into();
    assert!(media_link(&ok).is_ok());

    let none: BTreeSet<Service> = BTreeSet::new();
    let why = media_link(&none).unwrap_err();
    assert!(why.contains("no media service") && why.contains("exposes no services"), "{why}");

    // A rejected device names what it does expose.
    let gap = Uuid::from_u128(0x00001800_0000_1000_8000_00805f9b34fb);
    let name_chr = Uuid::from_u128(0x00002a00_0000_1000_8000_00805f9b34fb);
    let other: BTreeSet<_> = [service(gap, &[name_chr])].into();
    let why = media_link(&other).unwrap_err();
    assert!(why.contains("exposes 1 service(s): 00001800-0000-1000-8000-00805f9b34fb [00002a00-"), "{why}");

    // The four characteristics under some other service do not count.
    let elsewhere: BTreeSet<_> = [service(Uuid::from_u128(0x180a), &all)].into();
    assert!(media_link(&elsewhere).unwrap_err().contains("no media service"));

    let partial: BTreeSet<_> = [service(SERVICE_MEDIA, &all[1..])].into();
    assert!(media_link(&partial).unwrap_err().contains("missing 1f400001"));
}

#[test]
fn device_cache_round_trip() {
    let dir = std::env::temp_dir().join(format!("tiga_device_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("s.bin");
    assert_eq!(read_cached_device(&stream), None);
    write_cached_device(&stream, GOOD, "Zoom75 Tiga");
    assert_eq!(read_cached_device(&stream).as_deref(), Some(GOOD));
    fs::write(sidecar_path(&stream, ".device"), "address=\n").unwrap();
    assert_eq!(read_cached_device(&stream), None);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn envelope_length_must_match() {
    let mut s = synthetic_stream(SINGLE_FRAME_STREAM_LENS[0]);
    s[6] ^= 1;
    assert!(validate_stream(&s, true).is_err());
}

#[test]
fn single_frame_chunking() {
    let s = synthetic_stream(110_565);
    let chunks = build_chunks(&s, true).unwrap();
    let stream_len: usize = chunks.iter().map(|c| c.value.len() - 1).sum();
    assert_eq!(stream_len, 110_565);
    assert_eq!(chunks.iter().filter(|c| c.ends_4k_block).count(), 26);
    assert!(chunks.iter().all(|c| c.value.len() <= 182));
    let c1 = control1_for(stream_len);
    assert_eq!(&c1[18..21], &[0x01, 0xaf, 0xcc]); // file length 110,540
    assert_eq!(&c1[..18], &CONTROL1[..18]);
    assert_eq!(&c1[21..], &CONTROL1[21..]);
}

// ---- The write sequence of one upload, pinned ----
//
// Everything the keyboard receives for the template stream, in order, through the real
// upload_stream. The expected values don't come from this code: the CRC-32 of the
// template is the one in the hardware logs (`UPLOAD HELD ... crc32=57e8e0cf`, Phase 8),
// the writes at each block end are those logged by real uploads (`block  5/53 ACKed
// writes= 115/1239`), and the sequence CRC-32 was computed separately in Python from
// the chunking rule (PROJECT_HISTORY 3.2, 3.3).

const PINNED_CONTROL1: &str = "88 00 00 00 00 00 10 00 09 03 04 00 08 00 00 ff fc 00 03 5d d0 05 00 00";
const PINNED_CONTROL2: &str = "88 00 00 00 00 00 0a 00 08 03 00 ff ff ff 00 ff ff ff";
/// CRC-32 over `<1 control or 3 bulk> <u16 BE length> <bytes>` for every write, in order.
const PINNED_SEQUENCE_CRC32: u32 = 0x14af_2a9f;
const PINNED_TEMPLATE_CRC32: u32 = 0x57e8_e0cf;

fn pinned_bytes(hex: &str) -> Vec<u8> {
    hex.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

fn pinned_characteristic(uuid: Uuid) -> btleplug::api::Characteristic {
    btleplug::api::Characteristic {
        uuid,
        service_uuid: SERVICE_MEDIA,
        properties: btleplug::api::CharPropFlags::empty(),
        descriptors: std::collections::BTreeSet::new(),
    }
}

fn pinned_template() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("extracted")
            .join("session_01_bulk_stream.redgreen-backup.bin"),
    )
    .unwrap()
}

/// Records every write and answers the way the keyboard does: the ACK pairs, a PROGRESS
/// at each 4096-byte boundary of the DLX file, and COMPLETE after the last byte.
struct FakeKeyboard {
    writes: std::cell::RefCell<Vec<(Uuid, Vec<u8>, WriteType)>>,
    answers: futures::channel::mpsc::UnboundedSender<btleplug::api::ValueNotification>,
    stream_len: usize,
    bytes_sent: std::cell::Cell<usize>,
    /// How many writes had gone out at each PROGRESS.
    progress_at: std::cell::RefCell<Vec<usize>>,
    /// No answer to CONTROL1 at all.
    ignore_control1: bool,
    /// No PROGRESS after this many.
    progress_limit: usize,
}

impl FakeKeyboard {
    fn new(
        answers: futures::channel::mpsc::UnboundedSender<btleplug::api::ValueNotification>,
        stream_len: usize,
    ) -> Self {
        FakeKeyboard {
            writes: Default::default(),
            answers,
            stream_len,
            bytes_sent: Default::default(),
            progress_at: Default::default(),
            ignore_control1: false,
            progress_limit: usize::MAX,
        }
    }

    fn answer(&self, value: &[u8]) {
        self.answers.unbounded_send(notification(value)).unwrap();
    }
}

impl GattWriter for FakeKeyboard {
    async fn write(
        &self,
        characteristic: &btleplug::api::Characteristic,
        data: &[u8],
        write_type: WriteType,
    ) -> btleplug::Result<()> {
        self.writes.borrow_mut().push((characteristic.uuid, data.to_vec(), write_type));
        if characteristic.uuid == CHAR_CONTROL {
            if data.len() == 24 {
                if !self.ignore_control1 {
                    self.answer(C1_ACK_A);
                    self.answer(C1_ACK_B);
                }
            } else {
                self.answer(C2_ACK_A);
                self.answer(C2_ACK_B);
            }
            return Ok(());
        }
        let sent = self.bytes_sent.get() + data.len() - 1;
        self.bytes_sent.set(sent);
        if sent >= FILE_START_IN_STREAM
            && (sent - FILE_START_IN_STREAM) % 4096 == 0
            && self.progress_at.borrow().len() < self.progress_limit
        {
            let writes = self.writes.borrow().len() - 2;
            self.progress_at.borrow_mut().push(writes);
            self.answer(PROGRESS);
        }
        if sent == self.stream_len {
            self.answer(COMPLETE);
        }
        Ok(())
    }
}

async fn pinned_upload(
    keyboard: &FakeKeyboard,
    answers: &mut futures::channel::mpsc::UnboundedReceiver<btleplug::api::ValueNotification>,
    chunks: &[Chunk],
    block_timeout_ms: u64,
    stream_path: &Path,
    timings: &mut UploadTimings,
) -> Result<(), Failed> {
    upload_stream(
        keyboard,
        &pinned_characteristic(CHAR_CONTROL),
        &pinned_characteristic(CHAR_BULK),
        answers,
        chunks,
        0,
        block_timeout_ms,
        stream_path,
        "r-f0001",
        true,
        timings,
    )
    .await
}

#[tokio::test]
async fn template_upload_write_sequence_is_pinned() {
    let dir = std::env::temp_dir().join(format!("tiga_pin_sequence_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream_path = dir.join("s.bin");
    let template = pinned_template();
    assert_eq!(crc32(&template), PINNED_TEMPLATE_CRC32);
    let chunks = build_chunks(&template, false).unwrap();

    let (sender, mut answers) = futures::channel::mpsc::unbounded();
    let keyboard = FakeKeyboard::new(sender, template.len());
    let mut timings = UploadTimings::default();
    pinned_upload(&keyboard, &mut answers, &chunks, BLOCK_TIMEOUT_MS, &stream_path, &mut timings)
        .await
        .unwrap();
    let writes = keyboard.writes.into_inner();

    // CONTROL1, CONTROL2, then 1239 bulk writes, none of them with a response.
    assert_eq!(writes.len(), 2 + 1239);
    assert!(writes.iter().all(|w| w.2 == WriteType::WithoutResponse));
    assert_eq!((writes[0].0, writes[0].1.clone()), (CHAR_CONTROL, pinned_bytes(PINNED_CONTROL1)));
    assert_eq!((writes[1].0, writes[1].1.clone()), (CHAR_CONTROL, pinned_bytes(PINNED_CONTROL2)));
    assert!(writes[2..].iter().all(|w| w.0 == CHAR_BULK));

    // Each bulk write is a slice of the stream plus its XOR, and together they are the
    // stream, byte for byte and in order.
    let mut sent = Vec::new();
    for (_, value, _) in &writes[2..] {
        let (data, checksum) = value.split_at(value.len() - 1);
        assert_eq!(checksum[0], data.iter().fold(0u8, |acc, b| acc ^ b));
        sent.extend_from_slice(data);
    }
    assert_eq!(sent, template);

    // The sizes: 181 bytes + XOR, cut short at each 4096-byte boundary.
    let count = |len: usize| writes[2..].iter().filter(|w| w.1.len() == len).count();
    assert_eq!((count(182), count(115), count(140), count(98)), (1185, 52, 1, 1));
    assert_eq!((writes[2].1.len(), writes[writes.len() - 1].1.len()), (182, 98));

    // A PROGRESS after every 23rd write, as logged on hardware.
    let progress_at = keyboard.progress_at.into_inner();
    assert_eq!(progress_at.len(), 53);
    assert_eq!((progress_at[4], progress_at[9], progress_at[52]), (115, 230, 1219));
    assert!(progress_at.iter().enumerate().all(|(i, &w)| w == 23 * (i + 1)));

    // The whole sequence in one number.
    let mut all = Vec::new();
    for (uuid, value, _) in &writes {
        all.push(if *uuid == CHAR_CONTROL { 1 } else { 3 });
        all.extend_from_slice(&(value.len() as u16).to_be_bytes());
        all.extend_from_slice(value);
    }
    assert_eq!(crc32(&all), PINNED_SEQUENCE_CRC32);

    assert_eq!(timings.block_ms.len(), 53);
    assert!(timings.control1_ack_ms.is_some() && timings.control2_ack_ms.is_some());
    assert!(read_session_marker(&stream_path).is_none(), "COMPLETE closes the session");
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_stalled_block_stops_the_writes_and_keeps_the_session_marker() {
    let dir = std::env::temp_dir().join(format!("tiga_pin_stall_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream_path = dir.join("s.bin");
    let template = pinned_template();
    let chunks = build_chunks(&template, false).unwrap();

    let (sender, mut answers) = futures::channel::mpsc::unbounded();
    let mut keyboard = FakeKeyboard::new(sender, template.len());
    keyboard.progress_limit = 4;
    let mut timings = UploadTimings::default();
    let f = pinned_upload(&keyboard, &mut answers, &chunks, 50, &stream_path, &mut timings)
        .await
        .unwrap_err();

    // Block 5 was written in full, and nothing after it.
    assert_eq!(keyboard.writes.borrow().len(), 2 + 115);
    assert_eq!((f.kind, f.kind.code(), f.last_acked_block), (Failure::Interrupted, 20, Some(4)));
    assert!(f.progress_timeout);
    assert_eq!(f.detail, "timed out waiting for PROGRESS after 4K block 5/53 (last ACKed block 4/53)");
    assert_eq!(f.frame_id.as_deref(), Some("r-f0001"));
    assert_eq!(stall_signature(&timings.block_ms, f.progress_timeout), Signature::Sudden);
    let marker = read_session_marker(&stream_path).unwrap();
    assert_eq!(sidecar_value(&marker, "last_acked_block"), Some("4"));
    assert_eq!(sidecar_value(&marker, "total_blocks"), Some("53"));
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn no_answer_to_control1_is_unreachable_and_never_sends_control2() {
    let dir = std::env::temp_dir().join(format!("tiga_pin_control1_test_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream_path = dir.join("s.bin");
    let template = pinned_template();
    let chunks = build_chunks(&template, false).unwrap();

    let (sender, mut answers) = futures::channel::mpsc::unbounded();
    let mut keyboard = FakeKeyboard::new(sender, template.len());
    keyboard.ignore_control1 = true;
    let mut timings = UploadTimings::default();
    let started = Instant::now();
    let f = pinned_upload(&keyboard, &mut answers, &chunks, BLOCK_TIMEOUT_MS, &stream_path, &mut timings)
        .await
        .unwrap_err();

    assert_eq!(keyboard.writes.borrow().len(), 1);
    assert_eq!((f.kind, f.kind.code(), f.last_acked_block), (Failure::Unreachable, 10, None));
    assert_eq!(f.detail, "control ACK pair timed out");
    assert!(started.elapsed() >= Duration::from_millis(1500), "{:?}", started.elapsed());
    assert!(read_session_marker(&stream_path).is_none(), "no session before CONTROL2");
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(windows)]
#[test]
fn a_2ms_pause_lasts_about_2ms_after_the_timer_request() {
    // Without the request, tokio's 2 ms sleep lasted a 15.6 ms tick when the uploader was
    // started from a shell: 420 ms blocks instead of 157 ms (2026-11-11, timing.rs).
    assert!(request_fine_timer(), "timeBeginPeriod(1) refused");
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let mut pauses: Vec<Duration> = rt.block_on(async {
        let mut v = Vec::new();
        for _ in 0..40 {
            let t = Instant::now();
            tokio::time::sleep(Duration::from_millis(2)).await;
            v.push(t.elapsed());
        }
        v
    });
    pauses.sort();
    let median = pauses[pauses.len() / 2];
    assert!(median < Duration::from_millis(9), "a 2 ms pause took {median:?} (median of 40); a 15.6 ms tick means the request had no effect");
}
