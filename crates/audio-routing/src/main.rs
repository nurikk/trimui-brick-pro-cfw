use std::{env, fs, path::PathBuf, process};

use audio_routing::{
    alsa_buffer, default_alsa_buffer, diagnostics_from_state, load_route_or_default, save_route,
    valid_sample_rate_hz, AudioDiagnostics, FixtureAudioAdapter, LiveRouteManager, RouteManager,
    Runtime, Sink, SystemAudioAdapter,
};

fn main() {
    let result = execute(env::args().skip(1).collect());
    match result {
        Ok(Some(output)) => println!("{output}"),
        Ok(None) => println!("audio routing journey passed: live adapter lifecycle, ALSA profiles, hotplug, amp idle"),
        Err(error) => {
            eprintln!("brickpro-audio-routing: {error}");
            process::exit(1);
        }
    }
}

fn execute(args: Vec<String>) -> Result<Option<String>, String> {
    match args.first().map(String::as_str) {
        Some("journey") if args.len() == 1 => journey().map(|()| None),
        Some("diagnostics") if args.len() == 3 && args[1] == "--state" => {
            serde_json::to_string(&diagnostics_from_state(&PathBuf::from(&args[2])))
                .map(Some)
                .map_err(|_| "serialization-failed".into())
        }
        Some("system") => system(&args[1..]).map(|()| None),
        _ => Err("usage: brickpro-audio-routing journey | diagnostics --state PATH | system --state PATH --alsa-card N --speaker-control NAME --jack-control NAME --speaker-amp PATH --jack-present PATH ACTION".into()),
    }
}

fn system(args: &[String]) -> Result<(), String> {
    let mut state = None;
    let mut card = None;
    let mut speaker_control = None;
    let mut jack_control = None;
    let mut speaker_amp = None;
    let mut jack_present = None;
    let mut index = 0;
    while index + 1 < args.len() && args[index].starts_with("--") {
        let value = args[index + 1].clone();
        match args[index].as_str() {
            "--state" => state = Some(PathBuf::from(value)),
            "--alsa-card" => {
                card = Some(value.parse().map_err(|_| "alsa-card-invalid".to_string())?)
            }
            "--speaker-control" => speaker_control = Some(value),
            "--jack-control" => jack_control = Some(value),
            "--speaker-amp" => speaker_amp = Some(PathBuf::from(value)),
            "--jack-present" => jack_present = Some(PathBuf::from(value)),
            _ => return Err("audio-runtime-option-invalid".into()),
        }
        index += 2;
    }
    let action = args.get(index..).ok_or("audio-runtime-action-invalid")?;
    let state = state.ok_or("audio-state-required")?;
    let route = load_route_or_default(&state)?;
    let adapter = SystemAudioAdapter::new(
        card.ok_or("alsa-card-required")?,
        speaker_control.ok_or("speaker-control-required")?,
        jack_control.ok_or("jack-control-required")?,
        speaker_amp.ok_or("speaker-amp-required")?,
        jack_present.ok_or("jack-present-required")?,
    );
    let mut live = LiveRouteManager::new(route, adapter);
    live.poll_hotplug()?;
    match action {
        [command] if command == "poll" => {}
        [command, sink] if command == "select" => {
            let sink = parse_sink(sink)?;
            live.mutate(|route| {
                route.select_sink(sink);
            })?
        }
        [command, sink, volume] if command == "volume" => {
            let sink = parse_sink(sink)?;
            let volume = volume.parse().map_err(|_| "volume-invalid".to_string())?;
            live.mutate(|route| route.set_volume(sink, volume))?
        }
        [command, rate] if command == "stream-start" => {
            let rate = rate
                .parse()
                .map_err(|_| "sample-rate-invalid".to_string())?;
            valid_sample_rate_hz(rate)
                .then_some(())
                .ok_or_else(|| "sample-rate-invalid".to_string())?;
            lifecycle(live.transition(|route| route.begin_stream(rate))?)?
        }
        [command] if command == "stream-stop" => live.mutate(RouteManager::end_stream)?,
        [command] if command == "session-start" => {
            lifecycle(live.transition(RouteManager::begin_session)?)?
        }
        [command] if command == "session-end" => {
            lifecycle(live.transition(RouteManager::end_session)?)?
        }
        [command] if command == "suspend" => lifecycle(live.transition(RouteManager::suspend)?)?,
        [command] if command == "wake" => lifecycle(live.transition(RouteManager::wake)?)?,
        _ => return Err("audio-runtime-action-invalid".into()),
    }
    save_route(&state, &live.into_route())
}

fn lifecycle(applied: bool) -> Result<(), String> {
    applied
        .then_some(())
        .ok_or_else(|| "audio-lifecycle-transition-invalid".into())
}

fn parse_sink(value: &str) -> Result<Sink, String> {
    match value {
        "speaker" => Ok(Sink::Speaker),
        "jack" => Ok(Sink::Jack),
        _ => Err("sink-invalid".into()),
    }
}

fn journey() -> Result<(), String> {
    if default_alsa_buffer().buffer_frames != 1024
        || alsa_buffer(Runtime::RetroArch).period_frames != 384
        || alsa_buffer(Runtime::DraStic).buffer_frames != 2048
        || alsa_buffer(Runtime::Ppsspp).period_frames != 512
        || alsa_buffer(Runtime::Flycast).buffer_frames != 4096
        || alsa_buffer(Runtime::PortMaster).period_frames != 384
    {
        return Err("ALSA profiles are not centralized".into());
    }

    let state = env::temp_dir().join(format!("brickpro-audio-routing-{}", process::id()));
    let _ = fs::remove_file(&state);
    if !matches!(
        load_route_or_default(&state)?.diagnostics(),
        AudioDiagnostics::Available {
            sample_rate_hz: 48_000,
            ..
        }
    ) {
        return Err("missing audio state did not initialize defaults".into());
    }
    let corrupt = br#"{"schema":"brickpro-audio-route/v1","route":{"available":["speaker"],"currentSink":"speaker","requestedSink":"speaker","volumes":{"jack":50,"speaker":50},"streamActive":false,"sampleRateHz":192001,"underrunCount":0,"speakerAmpEnabled":false,"sessionSnapshot":null,"systemSuspendSnapshot":null}}"#;
    fs::write(&state, corrupt).map_err(|_| "audio-state-write-failed".to_string())?;
    if load_route_or_default(&state).is_ok()
        || fs::read(&state).map_err(|_| "audio-state-unavailable".to_string())? != corrupt
    {
        return Err("invalid audio state was applied or overwritten".into());
    }
    fs::remove_file(&state).map_err(|_| "audio-state-write-failed".to_string())?;

    let mut adapter = FixtureAudioAdapter::default();
    adapter.set_sink_present(Sink::Speaker, true);
    adapter.set_sink_present(Sink::Jack, true);
    let mut live = LiveRouteManager::new(RouteManager::default(), adapter);
    live.poll_hotplug()?;
    live.mutate(|route| {
        route.set_volume(Sink::Speaker, 33);
        route.set_volume(Sink::Jack, 71);
        route.select_sink(Sink::Jack);
        if !route.begin_stream(48_000) {
            route.record_underrun();
        }
    })?;
    expect(
        live.route(),
        Sink::Jack,
        48_000,
        false,
        "jack stream did not route deterministically",
    )?;
    for rate in [7_999, 192_001] {
        if live.transition(|route| route.begin_stream(rate))? {
            return Err("out-of-range sample rate accepted".into());
        }
        expect(
            live.route(),
            Sink::Jack,
            48_000,
            false,
            "invalid sample rate changed audio state",
        )?;
    }
    live.mutate(RouteManager::record_underrun)?;
    live.adapter_mut().set_sink_present(Sink::Jack, false);
    live.poll_hotplug()?;
    expect(
        live.route(),
        Sink::Speaker,
        48_000,
        true,
        "missing jack did not visibly fall back to speaker",
    )?;
    if live.route().volume(Sink::Speaker) != 33 || live.route().volume(Sink::Jack) != 71 {
        return Err("per-sink volume changed during fallback".into());
    }
    live.adapter_mut().set_sink_present(Sink::Jack, true);
    live.poll_hotplug()?;
    expect(
        live.route(),
        Sink::Jack,
        48_000,
        false,
        "jack did not recover after hotplug",
    )?;

    for _ in 0..100 {
        live.mutate(|route| {
            if !route.begin_session() || !route.end_session() || route.end_session() {
                route.record_underrun();
            }
        })?;
        expect(
            live.route(),
            Sink::Jack,
            48_000,
            false,
            "launch/exit lost route",
        )?;
    }

    for _ in 0..50 {
        live.mutate(|route| {
            if !route.suspend() || !route.wake() || route.wake() {
                route.record_underrun();
            }
        })?;
        expect(
            live.route(),
            Sink::Jack,
            48_000,
            false,
            "sleep/wake lost route",
        )?;
    }

    live.mutate(|route| {
        if !route.begin_session() || !route.suspend() || !route.wake() || !route.end_session() {
            route.record_underrun();
        }
    })?;
    expect(
        live.route(),
        Sink::Jack,
        48_000,
        false,
        "nested launch/suspend did not restore session snapshot",
    )?;

    live.mutate(|route| {
        if !route.begin_session() || !route.begin_stream(44_100) || !route.end_session() {
            route.record_underrun();
        }
    })?;
    expect(
        live.route(),
        Sink::Jack,
        48_000,
        false,
        "session exit left an inactive snapshot stream running",
    )?;

    live.mutate(|route| {
        route.select_sink(Sink::Speaker);
        route.set_volume(Sink::Speaker, 0);
        if !route.begin_stream(48_000) {
            route.record_underrun();
        }
    })?;
    expect(
        live.route(),
        Sink::Speaker,
        48_000,
        true,
        "volume-zero stream clipped speaker startup",
    )?;
    live.mutate(RouteManager::end_stream)?;
    expect(
        live.route(),
        Sink::Speaker,
        48_000,
        false,
        "idle speaker amp remained enabled",
    )?;
    match live.route().diagnostics() {
        AudioDiagnostics::Available {
            underrun_count: 1, ..
        } => Ok(()),
        _ => Err("diagnostics omitted underrun count".into()),
    }
}

fn expect(
    route: &RouteManager,
    sink: Sink,
    sample_rate_hz: u32,
    speaker_amp_enabled: bool,
    message: &str,
) -> Result<(), String> {
    match route.diagnostics() {
        AudioDiagnostics::Available {
            active_sink,
            sample_rate_hz: rate,
            speaker_amp_enabled: amp,
            ..
        } if active_sink == sink && rate == sample_rate_hz && amp == speaker_amp_enabled => Ok(()),
        _ => Err(message.into()),
    }
}
