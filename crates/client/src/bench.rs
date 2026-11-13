//! The bench: measures the frame rate at fixed spots under one set of settings, unattended,
//! then quits. Run it with `tools/bench.sh`, which builds the game, starts it once per settings
//! set and compares builds; it does nothing unless `BOXEL_BENCH` names a bench file.
//!
//! - `BOXEL_BENCH`: the bench file (see `tools/bench/bench.txt`): its `[bench]` timings,
//!   `[base]` settings every set gets, `[spot name]` places to stand and `[set name]` settings.
//! - `BOXEL_BENCH_SET`: which set to measure (`shipped` if unset).
//! - `BOXEL_BENCH_OUT`: the results file each spot's line is added to.
//! - `BOXEL_BENCH_BUILD`: what to call this build in the results.
//! - `BOXEL_BENCH_SHOTS`: if set, a folder for a screenshot of each spot, `<set>-<spot>.png`,
//!   taken once the view is steady and before measuring (the bench settles again after it).
//! - `BOXEL_TUNING` must point at a throwaway file: the bench writes the set's values there for
//!   the tuning panel to load, so it never touches the player's own tuning.
//!
//! A setting is `section.name = value`. A name `switches` knows (`shadows.maps`) is a switch,
//! on unless the value is 0; anything else is a tuning value, written to the tuning file under
//! `[section]`. A build that doesn't know a value warns as it loads the file, and `bench.sh`
//! lists those warnings with the results.
//!
//! Kept to what the client has had since the perf recorder (`f9fbc16`), so `tools/bench/
//! backport.sh` can copy it into older commits; only `switches` changes with the commit.

pub(crate) mod switches;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use api::{Vec3 as ApiVec3, WorldPos};
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PresentMode, PrimaryWindow};
use protocol::Command;

use crate::dev::DevMode;
use crate::perf::{Spot, summarize};
use crate::view::{ViewStats, ViewTuning};
use crate::world::{ClientWorld, Link};
use crate::{CapMode, FrameCap, Look, PendingToggles};

/// A frame slower than this, in seconds, counts as a stall: the view isn't steady yet.
const STALL: f32 = 0.25;

/// Starts the bench when `BOXEL_BENCH` is set; otherwise adds nothing.
pub struct BenchPlugin;

impl Plugin for BenchPlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = std::env::var_os("BOXEL_BENCH").map(PathBuf::from) else {
            return;
        };
        let set = std::env::var("BOXEL_BENCH_SET").unwrap_or_else(|_| "shipped".into());
        let fail = |why: String| -> ! {
            eprintln!("bench: {why}");
            std::process::exit(2);
        };
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| fail(format!("reading {}: {e}", path.display())));
        let file = BenchFile::parse(&text).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
        let Some(settings) = file.settings_for(&set) else {
            fail(format!("{} has no [set {set}]", path.display()));
        };
        if file.spots.is_empty() {
            fail(format!("{} has no [spot …]", path.display()));
        }
        let Some(tuning) = std::env::var_os("BOXEL_TUNING").map(PathBuf::from) else {
            fail("BOXEL_TUNING must name a throwaway tuning file for the bench to write".into());
        };
        let out = std::env::var_os("BOXEL_BENCH_OUT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("bench-results.txt"));
        let build = std::env::var("BOXEL_BENCH_BUILD").unwrap_or_else(|_| "?".into());
        let shots = std::env::var_os("BOXEL_BENCH_SHOTS").map(PathBuf::from);
        if let Some(dir) = &shots {
            let _ = std::fs::create_dir_all(dir);
        }
        app.insert_resource(Bench {
            file,
            set,
            settings,
            tuning,
            out,
            build,
            shots,
            shot: false,
            gpu: GpuClock::find(),
            phase: Phase::Starting,
            spot: 0,
            since: Instant::now(),
            last_stall: Instant::now(),
            last_fly: None,
            loading: false,
            frames: Vec::new(),
        })
        .add_systems(Startup, start_bench)
        .add_systems(Update, (watch_loading, drive_bench).chain().after(crate::mouse_look).before(crate::update_camera));
    }
}

/// What a bench file asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct BenchFile {
    /// Seconds each spot is measured for.
    pub seconds: f32,
    /// Seconds without a stall before measuring starts.
    pub settle: f32,
    /// Seconds a spot may take to settle before it's measured anyway.
    pub timeout: f32,
    /// The window's size in pixels.
    pub window: (u32, u32),
    /// Settings every set gets, before its own.
    pub base: Vec<Setting>,
    pub spots: Vec<(String, Spot)>,
    pub sets: Vec<(String, Vec<Setting>)>,
}

/// One `section.name = value` line.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub section: String,
    pub name: String,
    pub value: f32,
}

impl Setting {
    pub fn key(&self) -> String {
        format!("{}.{}", self.section, self.name)
    }
}

impl BenchFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut file = BenchFile { seconds: 5.0, settle: 2.0, timeout: 90.0, window: (1080, 1080), base: Vec::new(), spots: Vec::new(), sets: Vec::new() };
        let mut section = String::new();
        for (number, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            let at = |why: &str| format!("line {}: {why}", number + 1);
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_owned();
                if let Some(set) = section.strip_prefix("set ") {
                    if file.sets.iter().any(|(n, _)| n == set.trim()) {
                        return Err(at(&format!("[set {}] twice", set.trim())));
                    }
                    file.sets.push((set.trim().to_owned(), Vec::new()));
                } else if section != "bench" && section != "base" && !section.starts_with("spot ") {
                    return Err(at(&format!("unknown section [{section}]: expected [bench], [base], [spot name] or [set name]")));
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else {
                return Err(at("expected `name = value` or `[section]`"));
            };
            let number = |v: &str| v.parse::<f32>().map_err(|_| at(&format!("expected a number for {key}, got {v:?}")));
            if section == "bench" {
                match key {
                    "seconds" => file.seconds = number(value)?,
                    "settle" => file.settle = number(value)?,
                    "timeout" => file.timeout = number(value)?,
                    "window" => {
                        let size: Vec<u32> = value.split_whitespace().filter_map(|w| w.parse().ok()).collect();
                        let [w, h] = size[..] else {
                            return Err(at("window is `width height` in pixels"));
                        };
                        file.window = (w, h);
                    }
                    _ => return Err(at(&format!("[bench] has no {key}"))),
                }
            } else if let Some(spot) = section.strip_prefix("spot ") {
                if key != "at" {
                    return Err(at("a spot has one line, `at = ` and the spot from an F9 line"));
                }
                let spot_at = Spot::from_line(value).ok_or_else(|| at("expected `cell x y z offset x y z yaw pitch flying`"))?;
                file.spots.push((spot.trim().to_owned(), spot_at));
            } else {
                // Sections such as `terrain base:hills` have spaces and colons but no dots.
                let Some((sec, name)) = key.rsplit_once('.') else {
                    return Err(at(&format!("expected `section.name = value`, got {key:?}")));
                };
                let setting = Setting { section: sec.trim().to_owned(), name: name.trim().to_owned(), value: number(value)? };
                match file.sets.last_mut() {
                    Some((_, settings)) if section.starts_with("set ") => settings.push(setting),
                    _ if section == "base" => file.base.push(setting),
                    _ => return Err(at("a setting outside [base] or a [set name]")),
                }
            }
        }
        Ok(file)
    }

    /// Everything `set` asks for, base first, or `None` if there is no such set.
    pub fn settings_for(&self, set: &str) -> Option<Vec<Setting>> {
        let (_, own) = self.sets.iter().find(|(name, _)| name == set)?;
        Some(self.base.iter().chain(own).cloned().collect())
    }

    /// Just what the set itself changes, for the results line.
    pub fn describe(&self, set: &str) -> String {
        let own = self.sets.iter().find(|(name, _)| name == set).map(|(_, s)| s.as_slice()).unwrap_or_default();
        if own.is_empty() {
            return "-".into();
        }
        own.iter().map(|s| format!("{}={}", s.key(), s.value)).collect::<Vec<_>>().join(" ")
    }
}

/// Tuning values as a tuning file, one section each; a later value for a name wins.
pub fn tuning_text(settings: &[Setting]) -> String {
    let mut sections: Vec<(String, BTreeMap<String, f32>)> = Vec::new();
    for s in settings {
        let i = match sections.iter().position(|(name, _)| *name == s.section) {
            Some(i) => i,
            None => {
                sections.push((s.section.clone(), BTreeMap::new()));
                sections.len() - 1
            }
        };
        sections[i].1.insert(s.name.clone(), s.value);
    }
    let mut text = String::from("# Written by the bench; thrown away after the run.\n");
    for (section, values) in sections {
        text += &format!("\n[{section}]\n");
        for (name, value) in values {
            text += &format!("{name} = {value}\n");
        }
    }
    text
}

#[derive(Resource)]
struct Bench {
    file: BenchFile,
    set: String,
    settings: Vec<Setting>,
    tuning: PathBuf,
    out: PathBuf,
    build: String,
    /// Where screenshots go, if anywhere.
    shots: Option<PathBuf>,
    /// Whether this spot's screenshot has been taken.
    shot: bool,
    gpu: GpuClock,
    phase: Phase,
    /// The spot being settled at or measured.
    spot: usize,
    /// When the phase started.
    since: Instant,
    last_stall: Instant,
    /// When flying was last switched on, so it isn't switched again before the server answers.
    last_fly: Option<Instant>,
    /// The world around the spot is still loading (see `switches::loading`): counts as a stall,
    /// so measuring waits until it's all there.
    loading: bool,
    frames: Vec<f32>,
}

/// Notes whether the world is still loading, for the bench to wait.
fn watch_loading(world: &mut World) {
    let loading = switches::loading(world);
    if let Some(mut bench) = world.get_resource_mut::<Bench>() {
        bench.loading = loading;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Waiting for the player.
    Starting,
    /// At a spot, waiting for frames to be steady.
    Settling,
    Measuring,
    Done,
}

/// Sets the switches, writes the tuning values for the panel to load on its first frame, and
/// sets the window and frame cap up for measuring.
fn start_bench(world: &mut World) {
    let settings = world.resource::<Bench>().settings.clone();
    let mut tuning = Vec::new();
    for s in settings {
        if !switches::set(world, &s.key(), s.value != 0.0) {
            tuning.push(s);
        }
    }
    let path = world.resource::<Bench>().tuning.clone();
    if let Err(e) = std::fs::write(&path, tuning_text(&tuning)) {
        eprintln!("bench: writing {}: {e}", path.display());
        std::process::exit(2);
    }
    let (w, h) = world.resource::<Bench>().file.window;
    let mut windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    if let Ok(mut window) = windows.single_mut(world) {
        window.resolution.set_physical_resolution(w, h);
        window.present_mode = PresentMode::AutoNoVsync;
    }
    world.resource_mut::<FrameCap>().mode = CapMode::Uncapped;
    world.resource_mut::<DevMode>().show_text = false;
    let bench = world.resource::<Bench>();
    info!("bench: set {} ({}), {} spots, {} s each", bench.set, bench.file.describe(&bench.set), bench.file.spots.len(), bench.file.seconds);
}

#[allow(clippy::too_many_arguments)]
fn drive_bench(
    time: Res<Time<Real>>,
    mut bench: ResMut<Bench>,
    mut world: ResMut<ClientWorld>,
    mut link: ResMut<Link>,
    mut look: ResMut<Look>,
    mut toggles: ResMut<PendingToggles>,
    (view, drawn): (Res<ViewTuning>, Res<ViewStats>),
    window: Single<&Window, With<PrimaryWindow>>,
    adapter: Option<Res<RenderAdapterInfo>>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
) {
    let Some(player) = world.player() else {
        return;
    };
    let flying = player.flying();
    let frame = time.delta_secs();
    let now = Instant::now();
    if frame > STALL || bench.loading {
        bench.last_stall = now;
    }
    let bench = &mut *bench;
    let go_to = |bench: &mut Bench, world: &mut ClientWorld, link: &mut Link, spot: usize| {
        let (_, at) = bench.file.spots[spot];
        link.command(Command::TeleportPlayer(WorldPos::new(at.cell, ApiVec3::from_array(at.offset))), world);
        bench.spot = spot;
        bench.shot = false;
        bench.phase = Phase::Settling;
        bench.since = Instant::now();
    };
    match bench.phase {
        Phase::Starting => go_to(bench, &mut world, &mut link, 0),
        Phase::Settling => {
            let steady = now.duration_since(bench.since.max(bench.last_stall)).as_secs_f32() >= bench.file.settle;
            let late = now.duration_since(bench.since).as_secs_f32() >= bench.file.timeout;
            if let Some(dir) = bench.shots.as_ref().filter(|_| steady && flying && !bench.shot) {
                let file = dir.join(format!("{}-{}.png", bench.set, bench.file.spots[bench.spot].0));
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(file));
                bench.shot = true;
                bench.since = now;
            } else if (steady && flying) || late {
                bench.phase = Phase::Measuring;
                bench.since = now;
                bench.frames.clear();
                bench.gpu.samples.clear();
            }
        }
        Phase::Measuring => {
            bench.frames.push(frame);
            bench.gpu.sample(now);
            if now.duration_since(bench.since).as_secs_f32() >= bench.file.seconds {
                let (average, low) = summarize(&bench.frames[1..]);
                let (spot_name, _) = &bench.file.spots[bench.spot];
                let stalls = bench.frames.iter().filter(|&&f| f > STALL).count();
                let line = format!(
                    "{} | build {} | {} build | set {} | spot {spot_name} | avg {average:.1} fps | 1% low {low:.1} fps | {:.2} ms | {}x{} | {} | {:.2}M triangles | {} chunks drawn | render {:.0} m | {}{}{}",
                    now_text(),
                    bench.build,
                    if cfg!(debug_assertions) { "debug" } else { "release" },
                    bench.set,
                    1000.0 / average.max(0.001),
                    window.physical_width(),
                    window.physical_height(),
                    adapter.as_ref().map_or("?".into(), |a| a.name.clone()),
                    drawn.triangles as f64 / 1e6,
                    drawn.drawn,
                    view.render_distance,
                    bench.file.describe(&bench.set),
                    bench.gpu.describe(),
                    if stalls > 0 { format!(" | {stalls} stalls") } else { String::new() },
                );
                info!("bench: {line}");
                append(&bench.out, &line);
                if bench.spot + 1 < bench.file.spots.len() {
                    let next = bench.spot + 1;
                    go_to(bench, &mut world, &mut link, next);
                } else {
                    bench.phase = Phase::Done;
                    exit.write(AppExit::Success);
                }
            }
        }
        Phase::Done => {}
    }
    // Held every frame: the mouse doesn't move the view, and the player stays flying.
    if matches!(bench.phase, Phase::Settling | Phase::Measuring) {
        let (_, at) = bench.file.spots[bench.spot];
        (look.yaw, look.pitch) = (at.yaw, at.pitch);
        let waited = bench.last_fly.is_none_or(|t| now.duration_since(t).as_secs_f32() > 0.5);
        if !flying && !toggles.toggle_fly && waited {
            toggles.toggle_fly = true;
            bench.last_fly = Some(now);
        }
    }
}

/// The GPU's clock while measuring, where the system says (Intel on Linux): a GPU held under
/// its top speed, by heat or a power budget shared with the CPU, makes a run look slower than
/// the build is.
struct GpuClock {
    /// The files holding the clock in MHz and whether the GPU is being throttled.
    files: Option<(PathBuf, PathBuf)>,
    samples: Vec<(f32, bool)>,
    last: Option<Instant>,
}

impl GpuClock {
    fn find() -> Self {
        let files = (0..4).map(|card| PathBuf::from(format!("/sys/class/drm/card{card}/gt/gt0"))).find_map(|gt| {
            let (clock, throttled) = (gt.join("rps_act_freq_mhz"), gt.join("throttle_reason_status"));
            (clock.exists() && throttled.exists()).then_some((clock, throttled))
        });
        Self { files, samples: Vec::new(), last: None }
    }

    /// Reads the clock, at most four times a second.
    fn sample(&mut self, now: Instant) {
        let Some((clock, throttled)) = &self.files else {
            return;
        };
        if self.last.is_some_and(|t| now.duration_since(t).as_secs_f32() < 0.25) {
            return;
        }
        self.last = Some(now);
        let read = |path: &PathBuf| std::fs::read_to_string(path).ok().and_then(|s| s.trim().parse::<f32>().ok());
        if let (Some(mhz), Some(flag)) = (read(clock), read(throttled)) {
            self.samples.push((mhz, flag != 0.0));
        }
    }

    /// A field for the results line, or nothing where the clock can't be read.
    fn describe(&self) -> String {
        gpu_field(&self.samples)
    }
}

/// ` | gpu clock <average> MHz, throttled <share>%`, or nothing without samples.
fn gpu_field(samples: &[(f32, bool)]) -> String {
    if samples.is_empty() {
        return String::new();
    }
    let n = samples.len() as f32;
    let mhz = samples.iter().map(|s| s.0).sum::<f32>() / n;
    let throttled = samples.iter().filter(|s| s.1).count() as f32 / n;
    format!(" | gpu clock {mhz:.0} MHz, throttled {:.0}%", throttled * 100.0)
}

/// Adds `line` to the results file, making its folder if need be.
fn append(path: &std::path::Path, line: &str) {
    use std::io::Write;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    let written = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = written {
        error!("bench: writing {}: {e}", path.display());
    }
}

fn now_text() -> String {
    std::process::Command::new("date")
        .arg("+%F %T")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "
# a comment
[bench]
seconds = 3
window = 1280 720

[base]
world.terrain_width = 11
terrain base:hills.seed = 120   # the user's seed

[spot hill]
at = -70 4084 60 0.25 0.5 0 6.1 -0.6 true

[set shipped]

[set no_shadows]
shadows.maps = 0
shadows.far = 0
terrain base:hills.seed = 7
";

    #[test]
    fn a_bench_file_reads_back() {
        let file = BenchFile::parse(FILE).unwrap();
        assert_eq!((file.seconds, file.settle, file.window), (3.0, 2.0, (1280, 720)));
        assert_eq!(file.spots.len(), 1);
        assert_eq!(file.spots[0].0, "hill");
        assert_eq!(file.spots[0].1.cell, api::I64Vec3::new(-70, 4084, 60));
        assert!(file.spots[0].1.flying);
        let names: Vec<&str> = file.sets.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["shipped", "no_shadows"]);
        let seed = &file.base[1];
        assert_eq!((seed.section.as_str(), seed.name.as_str(), seed.value), ("terrain base:hills", "seed", 120.0));
    }

    #[test]
    fn a_set_gets_the_base_first_and_says_only_what_it_changes() {
        let file = BenchFile::parse(FILE).unwrap();
        let keys: Vec<String> = file.settings_for("no_shadows").unwrap().iter().map(Setting::key).collect();
        assert_eq!(keys, ["world.terrain_width", "terrain base:hills.seed", "shadows.maps", "shadows.far", "terrain base:hills.seed"]);
        assert_eq!(file.settings_for("shipped").unwrap().len(), 2);
        assert!(file.settings_for("nope").is_none());
        assert_eq!(file.describe("shipped"), "-");
        assert_eq!(file.describe("no_shadows"), "shadows.maps=0 shadows.far=0 terrain base:hills.seed=7");
    }

    #[test]
    fn tuning_values_become_a_tuning_file_where_the_set_wins() {
        let file = BenchFile::parse(FILE).unwrap();
        let text = tuning_text(&file.settings_for("no_shadows").unwrap());
        let (saved, problems) = crate::tuning_file::SavedTuning::parse(&text);
        assert!(problems.is_empty(), "{problems:?}");
        let mut seen = Vec::new();
        let mut hills = Recorder(&mut seen);
        let mut problems = Vec::new();
        saved.apply("terrain base:hills", &mut hills, &mut problems);
        assert_eq!(seen, [("seed".to_owned(), 7.0)], "the set's seed replaces the base's");
    }

    #[test]
    fn the_gpu_clock_is_averaged_with_how_often_it_was_throttled() {
        assert_eq!(gpu_field(&[]), "");
        assert_eq!(gpu_field(&[(1100.0, false), (900.0, true), (1000.0, false), (1000.0, false)]), " | gpu clock 1000 MHz, throttled 25%");
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        for (text, says) in [
            ("[set a]\n[set a]", "line 2: [set a] twice"),
            ("[bench]\nwindow = 1920", "line 2: window is"),
            ("[spot x]\nat = 1 2 3", "line 2: expected `cell"),
            ("[set a]\nshadows = 1", "line 2: expected `section.name"),
            ("[set a]\nshadows.maps = off", "line 2: expected a number"),
            ("shadows.maps = 0", "line 1: a setting outside"),
            ("[sets a]", "line 1: unknown section"),
        ] {
            let error = BenchFile::parse(text).unwrap_err();
            assert!(error.starts_with(says), "{text:?}: {error}");
        }
    }

    /// Records the values a tuning file sets.
    struct Recorder<'a>(&'a mut Vec<(String, f32)>);

    impl api::Tunables for Recorder<'_> {
        fn tunables(&self) -> Vec<api::Tunable> {
            vec![api::Tunable::new("seed", 0.0, 0.0, 1000.0)]
        }

        fn set_tunable(&mut self, name: &str, value: f32) -> bool {
            self.0.push((name.to_owned(), value));
            true
        }
    }
}
