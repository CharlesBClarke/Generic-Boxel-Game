//! A frame rate recorder, for comparing branches and settings without writing numbers down.
//! The `record_perf` key measures frames for a few seconds and appends one line to
//! `$XDG_DATA_HOME/boxel/perf.txt` (usually `~/.local/share/boxel/perf.txt`): average fps, the
//! slowest frames, what was drawn, where from, and on which branch. Each recording also keeps
//! where the player stood and looked, and the `perf_spot` key goes back there, in this run or
//! a later one, so two builds can be measured from the same view. A line also keeps the window's
//! size, the sun, and the shading and shadow values that differ from their defaults. For
//! measuring without pressing keys, see `bench`. Also the HUD's steady fps readout, [`FpsMeter`].

use std::path::PathBuf;
use std::time::Instant;

use api::{I64Vec3, Tunables, Vec3 as ApiVec3, WorldPos};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use protocol::Command;

use crate::controls::Controls;
use crate::shading::ShadingTuning;
use crate::shadows::{ShadowTuning, Sun};
use crate::view::{ViewStats, ViewTuning};
use crate::world::{ClientWorld, Link};
use crate::{FrameCap, Look, PendingToggles};

/// How long a recording measures, in seconds.
const SECONDS: f32 = 5.0;
/// How long the note about a recording stays on screen, in seconds.
const NOTE_SECONDS: f32 = 6.0;

/// Extra words for each recorded line, set by whatever the measurement should mention, such
/// as which shading parts are switched off.
#[derive(Resource, Default)]
pub struct PerfTags(pub String);

#[derive(Resource, Default)]
pub(crate) struct PerfRecorder {
    /// Frame times so far, while recording.
    frames: Option<Vec<f32>>,
    started: Option<Instant>,
    /// What to show on screen, and until when.
    note: String,
    note_until: Option<Instant>,
}

/// Marks the recorder's text on screen.
#[derive(Component)]
pub(crate) struct PerfText;

pub(crate) fn setup_perf_text(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextColor(Color::srgb(1.0, 0.9, 0.3)),
        Node { position_type: PositionType::Absolute, top: px(8), right: px(8), ..default() },
        PerfText,
    ));
}

/// Where the player stands and looks, as kept with a recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub cell: I64Vec3,
    pub offset: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
}

impl Spot {
    /// As one line of the spot file: `cell x y z offset x y z yaw pitch flying`.
    pub fn to_line(self) -> String {
        let [ox, oy, oz] = self.offset;
        format!("{} {} {} {ox} {oy} {oz} {} {} {}", self.cell.x, self.cell.y, self.cell.z, self.yaw, self.pitch, self.flying)
    }

    pub fn from_line(line: &str) -> Option<Self> {
        let words: Vec<&str> = line.split_whitespace().collect();
        let [x, y, z, ox, oy, oz, yaw, pitch, flying] = words[..] else {
            return None;
        };
        Some(Self {
            cell: I64Vec3::new(x.parse().ok()?, y.parse().ok()?, z.parse().ok()?),
            offset: [ox.parse().ok()?, oy.parse().ok()?, oz.parse().ok()?],
            yaw: yaw.parse().ok()?,
            pitch: pitch.parse().ok()?,
            flying: flying.parse().ok()?,
        })
    }
}

/// Average fps and the 1% low (the fps of the slowest hundredth of frames) from frame times
/// in seconds.
pub fn summarize(frames: &[f32]) -> (f32, f32) {
    let total: f32 = frames.iter().sum();
    if frames.is_empty() || total <= 0.0 {
        return (0.0, 0.0);
    }
    let mut sorted = frames.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let slowest = &sorted[..sorted.len().div_ceil(100)];
    let low = slowest.len() as f32 / slowest.iter().sum::<f32>();
    (frames.len() as f32 / total, low)
}

fn data_dir() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")))?;
    Some(data.join("boxel"))
}

/// The git branch and commit the game runs from, if it runs in a checkout.
fn branch() -> String {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let name = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "?".into());
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "?".into());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    format!("{name} ({commit}{})", if dirty { ", edited" } else { "" })
}

fn now() -> String {
    std::process::Command::new("date")
        .arg("+%F %T")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "?".into())
}

/// Starts a recording or goes back to the recorded spot on their keys; counts frames while
/// recording, and writes the line when it's done.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_perf(
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    time: Res<Time<Real>>,
    mut recorder: ResMut<PerfRecorder>,
    mut world: ResMut<ClientWorld>,
    mut link: ResMut<Link>,
    (mut look, mut toggles, cap): (ResMut<Look>, ResMut<PendingToggles>, Res<FrameCap>),
    (view, drawn, tags): (Res<ViewTuning>, Res<ViewStats>, Res<PerfTags>),
    window: Single<&Window, With<PrimaryWindow>>,
    (sun, shading, shadows): (Res<Sun>, Res<ShadingTuning>, Res<ShadowTuning>),
    mut text: Single<&mut Text, With<PerfText>>,
) {
    let Some(player) = world.player() else {
        return;
    };
    let spot = Spot {
        cell: player.pos().cell(),
        offset: player.pos().offset().to_array(),
        yaw: look.yaw,
        pitch: look.pitch,
        flying: player.flying(),
    };
    let note = |recorder: &mut PerfRecorder, message: String| {
        recorder.note = message;
        recorder.note_until = Some(Instant::now() + std::time::Duration::from_secs_f32(NOTE_SECONDS));
    };

    if keys.just_pressed(controls.perf_spot) {
        let saved = data_dir().and_then(|d| std::fs::read_to_string(d.join("perf_spot.txt")).ok());
        match saved.as_deref().and_then(Spot::from_line) {
            Some(to) => {
                let pos = WorldPos::new(to.cell, ApiVec3::from_array(to.offset));
                link.command(Command::TeleportPlayer(pos), &mut world);
                (look.yaw, look.pitch) = (to.yaw, to.pitch);
                if to.flying != spot.flying {
                    toggles.toggle_fly = !toggles.toggle_fly;
                }
                note(&mut recorder, "back at the recorded spot".into());
            }
            None => note(&mut recorder, "no recorded spot yet".into()),
        }
    }

    if keys.just_pressed(controls.record_perf) && recorder.frames.is_none() {
        recorder.frames = Some(Vec::new());
        recorder.started = Some(Instant::now());
    }
    if let Some(frames) = &mut recorder.frames {
        frames.push(time.delta_secs());
        let elapsed = recorder.started.map_or(0.0, |s| s.elapsed().as_secs_f32());
        // The frame the key was pressed on ran before recording started.
        if elapsed >= SECONDS {
            let frames = recorder.frames.take().unwrap_or_default();
            let (average, low) = summarize(&frames[1..]);
            let line = format!(
                "{} | {} | {} build | avg {average:.1} fps | 1% low {low:.1} fps | {:.1} ms | cap {} | {:.2}M triangles | {} chunks drawn | render {:.0} m | at {} | {} | {}x{} | sun {:.0} {:.0} | changed: {}",
                now(),
                branch(),
                if cfg!(debug_assertions) { "debug" } else { "release" },
                1000.0 / average.max(0.001),
                cap.mode.label(),
                drawn.triangles as f64 / 1e6,
                drawn.drawn,
                view.render_distance,
                spot.to_line(),
                if tags.0.is_empty() { "-" } else { &tags.0 },
                window.physical_width(),
                window.physical_height(),
                sun.sky.sun_azimuth,
                sun.sky.sun_elevation,
                changed_values(&[("shading", &*shading, &ShadingTuning::default()), ("shadows", &*shadows, &ShadowTuning::default())]),
            );
            let written = data_dir().and_then(|dir| {
                std::fs::create_dir_all(&dir).ok()?;
                std::fs::write(dir.join("perf_spot.txt"), spot.to_line()).ok()?;
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("perf.txt")).ok()?;
                writeln!(file, "{line}").ok()?;
                Some(dir.join("perf.txt"))
            });
            info!("perf: {line}");
            let vsync = if matches!(cap.mode, crate::CapMode::Vsync) { "\n(vsync holds fps at the screen's rate: frame cap to none to measure)" } else { "" };
            let saved = written.map_or("couldn't write the perf file".into(), |p| format!("saved to {}", p.display()));
            note(&mut recorder, format!("recorded: {average:.1} fps, 1% low {low:.1}\n{saved}{vsync}"));
        }
    }

    let shown = if let Some(started) = recorder.started.filter(|_| recorder.frames.is_some()) {
        format!("recording frame rate… {:.0} s", (SECONDS - started.elapsed().as_secs_f32()).max(0.0).ceil())
    } else if recorder.note_until.is_some_and(|t| Instant::now() < t) {
        recorder.note.clone()
    } else {
        String::new()
    };
    if text.0 != shown {
        text.0 = shown;
    }
}

/// `section.name=value` for each value that differs from its default, or `-`.
pub fn changed_values(sections: &[(&str, &dyn Tunables, &dyn Tunables)]) -> String {
    let mut out = Vec::new();
    for (section, now, default) in sections {
        let defaults = default.tunables();
        for t in now.tunables() {
            if defaults.iter().find(|d| d.name == t.name).is_none_or(|d| d.value != t.value) {
                out.push(format!("{section}.{}={}", t.name, t.value));
            }
        }
    }
    if out.is_empty() { "-".into() } else { out.join(" ") }
}

/// The HUD's frame rate: frames and their time added up, and shown as an average that
/// refreshes every [`FpsMeter::EVERY`] seconds, so it holds still long enough to read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FpsMeter {
    frames: u32,
    seconds: f64,
    /// The last average: frames a second and milliseconds a frame. None until the first.
    shown: Option<(f64, f64)>,
}

impl FpsMeter {
    pub const EVERY: f64 = 0.5;

    /// Counts a frame that took `seconds`, and gives what to show.
    pub fn frame(&mut self, seconds: f64) -> Option<(f64, f64)> {
        self.frames += 1;
        self.seconds += seconds.max(0.0);
        if self.seconds >= Self::EVERY {
            self.shown = Some((self.frames as f64 / self.seconds, self.seconds * 1000.0 / self.frames as f64));
            *self = Self { shown: self.shown, ..Self::default() };
        }
        self.shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_frames_show_their_exact_rate() {
        let mut meter = FpsMeter::default();
        let mut shown = None;
        for _ in 0..100 {
            shown = meter.frame(1.0 / 125.0);
        }
        let (fps, ms) = shown.unwrap();
        assert!((fps - 125.0).abs() < 1e-6 && (ms - 8.0).abs() < 1e-6, "{fps} {ms}");
    }

    #[test]
    fn the_number_holds_between_refreshes() {
        let mut meter = FpsMeter::default();
        assert_eq!(meter.frame(0.01), None, "nothing to show before the first half second");
        let first = loop {
            if let Some(shown) = meter.frame(0.01) {
                break shown;
            }
        };
        assert!((first.0 - 100.0).abs() < 1e-6, "{first:?}");
        // A slow frame and then fast ones: the shown number waits for the next refresh.
        assert_eq!(meter.frame(0.1), Some(first));
        assert_eq!(meter.frame(0.001), Some(first));
        // Then the average takes in all of them.
        let mut next = Some(first);
        for _ in 0..400 {
            next = meter.frame(0.001);
            if next != Some(first) {
                break;
            }
        }
        let (fps, _) = next.unwrap();
        assert!(fps < 1000.0 && fps > 100.0, "the slow frame pulls the average down: {fps}");
    }

    #[test]
    fn a_spot_round_trips_through_its_line() {
        let spot = Spot { cell: I64Vec3::new(-12, 4096, 77), offset: [0.25, -0.5, 0.125], yaw: 1.5, pitch: -0.25, flying: true };
        assert_eq!(Spot::from_line(&spot.to_line()), Some(spot));
        assert_eq!(Spot::from_line("1 2 3"), None);
    }

    #[test]
    fn only_values_off_their_defaults_are_listed() {
        let mut shadows = ShadowTuning::default();
        let shading = ShadingTuning::default();
        assert_eq!(changed_values(&[("shading", &shading, &ShadingTuning::default())]), "-");
        assert!(shadows.set_tunable("map_size", 1024.0));
        assert_eq!(changed_values(&[("shading", &shading, &ShadingTuning::default()), ("shadows", &shadows, &ShadowTuning::default())]), "shadows.map_size=1024");
    }

    #[test]
    fn the_average_and_the_slowest_frames() {
        // 99 frames at 10 ms and one at 50 ms: 1 s for 100 frames.
        let mut frames = vec![0.01; 99];
        frames.push(0.05);
        let (average, low) = summarize(&frames);
        assert!((average - 100.0 / 1.04).abs() < 0.01, "{average}");
        assert!((low - 20.0).abs() < 0.01, "the slowest hundredth is the one 50 ms frame: {low}");
        assert_eq!(summarize(&[]), (0.0, 0.0));
    }
}
