//! swapdock — launch film v2, 30 seconds, benefit-first.
//!
//! Copy beats: ship at 2pm, no maintenance window, one command, rollback,
//! and one box instead of a control plane. No journal internals.
use fframes::{
    AnimateRuntimeInput, AudioMap, AudioTrack, AudioTimestamp::{Eof, Second}, Color, Duration,
    FFramesContext, FontQuery, Frame, Scene, Scenes, Svgr, Transform, Video,
    animation::{AnimationRuntime, Easing},
    include_media_dir,
};
use std::sync::LazyLock;

include_media_dir!(pub struct LaunchVideoMedia, "media", stereo);

pub const WIDTH: usize = 1920;
pub const HEIGHT: usize = 1080;
pub const FPS: usize = 30;
pub const TOTAL: f32 = 30.0;

const BG: &str = "#08090A";
const CARD: &str = "#0E0F12";
const CARD2: &str = "#15161A";
const INK: &str = "#F3F2EF";
const MUTED: &str = "#9B9CA4";
const DIM: &str = "#6F7079";
const LINE: &str = "#27282E";
const ACCENT: &str = "#8B8BFF";
const LIME: &str = "#B6F09C";
const RED: &str = "#F2616B";
const AMBER: &str = "#FFD166";

const SANS: &str = "Geist";
const MONO: &str = "IBM Plex Mono";
const W_SEMI: u16 = 600;
const W_MED: u16 = 500;
const W_REG: u16 = 400;

static EASE_OUT: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(0.55, &Easing::EaseOut));
static EASE_OUT_SLOW: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(0.9, &Easing::EaseOut));
static EASE_IN: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(0.24, &Easing::EaseIn));
static EASE_INOUT: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(0.75, &Easing::EaseInOut));
static SPRING: LazyLock<AnimationRuntime> = LazyLock::new(|| {
    AnimationRuntime::new(
        2.0,
        &Easing::Spring {
            mass: 1.0,
            stiffness: 210.0,
            damping: 23.0,
        },
    )
});
static DRAIN: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(2.5, &Easing::EaseInOut));

fn p(frame: &Frame, start: f32) -> f32 {
    frame.animate_runtime(AnimateRuntimeInput {
        on_second: start,
        from: 0.0,
        to: 1.0,
        animation_runtime: &EASE_OUT,
    })
}

fn p_slow(frame: &Frame, start: f32) -> f32 {
    frame.animate_runtime(AnimateRuntimeInput {
        on_second: start,
        from: 0.0,
        to: 1.0,
        animation_runtime: &EASE_OUT_SLOW,
    })
}

fn rise(frame: &Frame, start: f32) -> f32 {
    frame.animate_runtime(AnimateRuntimeInput {
        on_second: start,
        from: 40.0,
        to: 0.0,
        animation_runtime: &SPRING,
    })
}

fn out(frame: &Frame, start: f32) -> f32 {
    frame.animate_runtime(AnimateRuntimeInput {
        on_second: start,
        from: 1.0,
        to: 0.0,
        animation_runtime: &EASE_IN,
    })
}

fn draw(frame: &Frame, start: f32, len: f32) -> f32 {
    frame.animate_runtime(AnimateRuntimeInput {
        on_second: start,
        from: len,
        to: 0.0,
        animation_runtime: &EASE_INOUT,
    })
}

fn drain_p(frame: &Frame, start: f32) -> f32 {
    frame.animate_runtime(AnimateRuntimeInput {
        on_second: start,
        from: 0.0,
        to: 1.0,
        animation_runtime: &DRAIN,
    })
}

fn fit_size<'a>(
    frame: &mut Frame,
    ctx: &FFramesContext<'a, '_>,
    text: &'a str,
    preferred: usize,
    max_width: usize,
) -> usize {
    let mut size = preferred;
    loop {
        let font = FontQuery {
            family: SANS,
            size,
            weight: W_SEMI,
            ..Default::default()
        };
        match frame.text_width(ctx, font, text) {
            Some(w) if w > max_width && size > 30 => size -= 4,
            _ => return size,
        }
    }
}

fn logo(x: f32, y: f32, scale: f32, opacity: f32) -> Svgr<'static> {
    fframes::svgr!(
        <g transform={format!("translate({x} {y}) scale({scale})")} opacity={opacity}>
            <rect width="22" height="30" rx="7" fill={ACCENT} />
            <rect x="30" width="22" height="30" rx="7" fill={LIME} />
            <path d="M22 15h8" stroke={INK} stroke-opacity="0.4" stroke-width="2" />
        </g>
    )
}

fn chrome<'a>(no: &'a str, name: &'a str) -> Svgr<'a> {
    fframes::svgr!(
        <g>
            {logo(160.0, 78.0, 1.0, 0.9)}
            <text x="232" y="101" font-family={SANS} font-size="25" font-weight={W_SEMI} fill={INK} letter-spacing="-0.5">
                "swapdock"
            </text>
            <text x="1760" y="100" text-anchor="end" font-family={MONO} font-size="17" fill={DIM} letter-spacing="1.5">
                {no} "  /  " {name}
            </text>
        </g>
    )
}

fn title<'a>(
    frame: &mut Frame,
    ctx: &FFramesContext<'a, '_>,
    text: &'a str,
    x: f32,
    y: f32,
    preferred: usize,
    max_width: usize,
    fill: &'a str,
    start: f32,
) -> Svgr<'a> {
    let size = fit_size(frame, ctx, text, preferred, max_width);
    let opacity = p(frame, start);
    let dy = rise(frame, start);
    fframes::svgr!(
        <g transform={Transform::translate(0.0, dy)} opacity={opacity}>
            <text x={x} y={y} font-family={SANS} font-size={size} font-weight={W_SEMI} fill={fill} letter-spacing="-2.4">
                {text}
            </text>
        </g>
    )
}

fn sub<'a>(frame: &Frame, text: &'a str, x: f32, y: f32, start: f32, fill: &'a str, size: f32) -> Svgr<'a> {
    let opacity = p(frame, start);
    let dy = rise(frame, start);
    fframes::svgr!(
        <g transform={Transform::translate(0.0, dy)} opacity={opacity}>
            <text x={x} y={y} font-family={SANS} font-size={size} font-weight={W_REG} fill={fill}>
                {text}
            </text>
        </g>
    )
}

fn mono<'a>(text: &'a str, x: f32, y: f32, size: f32, fill: &'a str, anchor: &'a str) -> Svgr<'a> {
    fframes::svgr!(
        <text x={x} y={y} font-family={MONO} font-size={size} font-weight={W_MED} fill={fill} text-anchor={anchor}>
            {text}
        </text>
    )
}

fn card<'a>(x: f32, y: f32, w: f32, h: f32, stroke: &'a str, fill: &'a str, opacity: f32) -> Svgr<'a> {
    fframes::svgr!(
        <rect x={x} y={y} width={w} height={h} rx="18" fill={fill} stroke={stroke} stroke-width="1.4" opacity={opacity} />
    )
}

fn pill<'a>(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &'a str,
    fg: &'a str,
    bg: &'a str,
    opacity: f32,
    size: f32,
) -> Svgr<'a> {
    fframes::svgr!(
        <g opacity={opacity}>
            <rect x={x} y={y} width={w} height={h} rx={h * 0.5} fill={bg} stroke={fg} stroke-opacity="0.35" stroke-width="1" />
            <text x={x + w * 0.5} y={y + h * 0.5 + size * 0.34} text-anchor="middle" font-family={MONO} font-size={size} font-weight={W_MED} fill={fg}>
                {label}
            </text>
        </g>
    )
}

fn check<'a>(x: f32, y: f32, size: f32, color: &'a str, progress: f32) -> Svgr<'a> {
    fframes::svgr!(
        <g transform={format!("translate({x} {y}) scale({})", size / 24.0)} opacity={progress}>
            <path d="M4 12.5 L9.5 18 L20 5.5" stroke={color} stroke-width="2.8" fill="none"
                stroke-linecap="round" stroke-linejoin="round" stroke-dasharray="28"
                stroke-dashoffset={28.0 * (1.0 - progress)} />
        </g>
    )
}

// ---------------------------------------------------------------------------
// 01 — Ship in the middle of the day. Nobody notices.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Ship;

impl Scene for Ship {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let s = frame.seconds();
        let head = title(&mut frame, ctx, "SHIP AT 2PM.", 160.0, 360.0, 112, 1450, INK, -0.4);
        let nobody = title(&mut frame, ctx, "Nobody notices.", 160.0, 490.0, 76, 1450, LIME, 1.5);
        let line = draw(&frame, 1.9, 540.0);
        let live = p(&frame, 0.4);
        let release = p(&frame, 0.9);
        let pulse = 0.5 + 0.5 * (s * 2.0 * std::f32::consts::PI).sin();
        let ticker: Vec<Svgr> = (0..6)
            .map(|i| {
                let x = 160.0 + ((s * 80.0 + i as f32 * 280.0) % 1480.0);
                fframes::svgr!(
                    <g>
                        {pill(x, 900.0, 92.0, 34.0, "200", LIME, "#16220F", 0.6, 11.0)}
                    </g>
                )
            })
            .collect();

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("01", "SHIP")}
                <g opacity={p(&frame, -0.3)}>
                    {mono("DEPLOY 128", 160.0, 210.0, 15.0, DIM, "start")}
                </g>
                {head}
                {nobody}
                <g opacity={p(&frame, 1.9)}>
                    <path d="M160 532 H700" stroke={LIME} stroke-width="3" stroke-dasharray="540" stroke-dashoffset={line} />
                </g>

                <g opacity={p(&frame, -0.2)} transform={Transform::translate(0.0, rise(&frame, -0.2))}>
                    {card(1020.0, 280.0, 740.0, 470.0, LINE, CARD, 1.0)}
                    <text x="1060" y="338" font-size="28" font-weight={W_SEMI} fill={INK}>"blog"</text>
                    {mono("blog.example", 1060.0, 372.0, 16.0, DIM, "start")}
                    {pill(1590.0, 306.0, 130.0, 34.0, "live", LIME, "#16220F", live, 12.0)}
                    <circle cx="1712" cy="323" r={5.0 + 2.0 * pulse} fill={LIME} opacity={live} />

                    <text x="1060" y="448" font-size="36" font-weight={W_SEMI} fill={INK}>"A quiet Tuesday"</text>
                    <rect x="1060" y="474" width="560" height="9" rx="4" fill={LINE} />
                    <rect x="1060" y="494" width="520" height="9" rx="4" fill={LINE} />
                    <rect x="1060" y="514" width="380" height="9" rx="4" fill={LINE} />
                    <rect x="1060" y="548" width="600" height="9" rx="4" fill={LINE} opacity="0.6" />
                    <rect x="1060" y="568" width="560" height="9" rx="4" fill={LINE} opacity="0.6" />
                    <rect x="1060" y="588" width="420" height="9" rx="4" fill={LINE} opacity="0.6" />

                    <g opacity={release} transform={Transform::translate(0.0, rise(&frame, 0.9))}>
                        {pill(1060.0, 660.0, 300.0, 48.0, "release 9c1f2ab", LIME, "#16220F", 1.0, 14.0)}
                        {check(1380.0, 673.0, 24.0, LIME, release)}
                    </g>
                </g>

                <g opacity={p(&frame, -0.3)}>
                    {ticker}
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 02 — Downtime is a choice.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Problem;

impl Scene for Problem {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let s = frame.seconds();
        let down = title(&mut frame, ctx, "DOWNTIME", 160.0, 360.0, 132, 1100, RED, -0.4);
        let choice = title(&mut frame, ctx, "IS A CHOICE.", 160.0, 490.0, 110, 1300, INK, -0.3);
        let dont = title(&mut frame, ctx, "Yours doesn't have to.", 160.0, 640.0, 56, 1200, LIME, 2.0);
        let bad = p(&frame, -0.2) * out(&frame, 2.0);
        let good = p(&frame, 2.05);
        let shake = if (0.9..1.15).contains(&s) {
            (s * 120.0).sin() * 4.0
        } else {
            0.0
        };

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("02", "PROBLEM")}
                {down}
                {choice}
                {dont}

                <g transform={Transform::translate(shake, 0.0)} opacity={bad}>
                    {card(1060.0, 300.0, 700.0, 430.0, RED, CARD, 1.0)}
                    {mono("MAINTENANCE WINDOW", 1096.0, 348.0, 14.0, RED, "start")}
                    <text x="1100" y="560" font-size="170" font-weight={W_SEMI} fill={RED} letter-spacing="-6">"502"</text>
                    {mono("Bad Gateway", 1100.0, 610.0, 18.0, DIM, "start")}
                    <rect x="1100" y="640" width="620" height="6" rx="3" fill={RED} opacity="0.35" />
                </g>

                <g opacity={good} transform={Transform::translate(0.0, rise(&frame, 2.05))}>
                    {card(1060.0, 300.0, 700.0, 430.0, LIME, CARD, 1.0)}
                    {mono("SWAP", 1096.0, 348.0, 14.0, LIME, "start")}
                    <text x="1100" y="552" font-size="150" font-weight={W_SEMI} fill={LIME} letter-spacing="-5">"200"</text>
                    {mono("OK", 1100.0, 600.0, 22.0, LIME, "start")}
                    <text x="1100" y="656" font-size="28" font-weight={W_REG} fill={MUTED}>"the site stays up"</text>
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 03 — One command.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Command;

impl Scene for Command {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let s = frame.seconds();
        let head = title(&mut frame, ctx, "One command.", 160.0, 280.0, 104, 1500, INK, -0.4);
        let caption = sub(&frame, "No YAML. No control plane.", 166.0, 342.0, -0.2, MUTED, 34.0);
        let command = "swapdock up blog --release 9c1f2ab";
        let n = (command.chars().count() as f32 * p(&frame, 0.35)) as usize;
        let shown: String = command.chars().take(n).collect();
        let shown_chars = shown.chars().count() as f32;
        let output = p(&frame, 1.1);
        let caret = 0.35 + 0.65 * (s * 3.0).sin().abs();

        let steps = ["BUILD", "START", "HEALTH", "FLIP", "COMMIT"];
        let step_svgs: Vec<Svgr> = steps
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let start = 1.5 + i as f32 * 0.15;
                let op = p(&frame, start);
                let x = 200.0 + i as f32 * 280.0;
                fframes::svgr!(
                    <g opacity={op} transform={Transform::translate(0.0, rise(&frame, start))}>
                        {card(x, 760.0, 240.0, 92.0, if i == 4 { LIME } else { LINE }, CARD, 1.0)}
                        {mono(label, x + 24.0, 802.0, 14.0, if i == 4 { LIME } else { MUTED }, "start")}
                        {check(x + 196.0, 790.0, 24.0, LIME, op)}
                        <path d={format!("M{} 806 H{}", x + 20.0, x + 80.0)} stroke={LINE} stroke-width="1" />
                    </g>
                )
            })
            .collect();

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("03", "ONE COMMAND")}
                {head}
                {caption}

                <g opacity={p(&frame, -0.2)} transform={Transform::translate(0.0, rise(&frame, -0.2))}>
                    {card(200.0, 410.0, 1520.0, 270.0, LINE, CARD, 1.0)}
                    <circle cx="236" cy="446" r="5" fill={RED} opacity="0.8" />
                    <circle cx="256" cy="446" r="5" fill={AMBER} opacity="0.8" />
                    <circle cx="276" cy="446" r="5" fill={LIME} opacity="0.8" />
                    {mono("deploy / blog", 1680.0, 450.0, 13.0, DIM, "end")}

                    <text x="240" y="540" font-family={MONO} font-size="30" font-weight={W_MED} fill={LIME}>"$"</text>
                    <text x="276" y="540" font-family={MONO} font-size="30" font-weight={W_MED} fill={INK}>{shown}</text>
                    <rect x={276.0 + shown_chars * 18.0} y="514" width="3" height="32" fill={LIME} opacity={caret} />

                    <g opacity={output}>
                        {check(240.0, 604.0, 26.0, LIME, output)}
                        {mono("healthy  ·  flipped  ·  committed", 280.0, 624.0, 20.0, MUTED, "start")}
                    </g>
                </g>

                {step_svgs}
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 04 — No maintenance window.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Window;

impl Scene for Window {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(5.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let s = frame.seconds();
        let head = title(&mut frame, ctx, "No maintenance window.", 160.0, 270.0, 76, 1550, INK, -0.4);
        let caption = sub(&frame, "Traffic moves once. The site stays up.", 166.0, 330.0, -0.2, MUTED, 32.0);
        let sw = p_slow(&frame, 1.9);
        let old_dim = 1.0 - 0.62 * sw;
        let gate1 = p(&frame, 0.9);
        let gate2 = p(&frame, 1.5);
        let done = p(&frame, 3.4);
        let dp = drain_p(&frame, 2.0);
        let circ = 2.0 * std::f32::consts::PI * 36.0;
        let remain = (15.0 * (1.0 - dp)).ceil() as i32;

        let old_pills: Vec<Svgr> = (0..4)
            .map(|i| {
                let x = 600.0 + ((s * 230.0 + i as f32 * 250.0) % 980.0);
                fframes::svgr!(
                    <g opacity={(1.0 - sw) * 0.9}>
                        {pill(x, 470.0, 100.0, 40.0, "200", LIME, "#16220F", 1.0, 13.0)}
                    </g>
                )
            })
            .collect();
        let new_pills: Vec<Svgr> = (0..4)
            .map(|i| {
                let x = 600.0 + ((s * 230.0 + i as f32 * 250.0 + 120.0) % 980.0);
                fframes::svgr!(
                    <g opacity={sw * 0.95}>
                        {pill(x, 680.0, 100.0, 40.0, "200", LIME, "#16220F", 1.0, 13.0)}
                    </g>
                )
            })
            .collect();

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("04", "NO WINDOW")}
                {head}
                {caption}

                <g opacity={p(&frame, -0.2)}>
                    {card(180.0, 400.0, 400.0, 140.0, LINE, CARD, old_dim)}
                    <text x="212" y="456" font-size="26" font-weight={W_SEMI} fill={INK} opacity={old_dim}>"old"</text>
                    {mono(":9000", 212.0, 502.0, 18.0, DIM, "start")}
                </g>
                <g opacity={p(&frame, -0.1)}>
                    {card(180.0, 650.0, 400.0, 140.0, LIME, CARD, 1.0)}
                    <text x="212" y="706" font-size="26" font-weight={W_SEMI} fill={INK}>"new"</text>
                    {mono(":9001  ·  9c1f2ab", 212.0, 752.0, 16.0, LIME, "start")}
                </g>

                <path d="M520 470 H1700" stroke={LINE} stroke-width="1" opacity={old_dim} />
                <path d="M520 710 H1700" stroke={LINE} stroke-width="1" />
                {old_pills}
                {new_pills}

                <g opacity={gate1}>
                    {card(1360.0, 330.0, 340.0, 70.0, LINE, CARD, 1.0)}
                    {mono("container health", 1384.0, 372.0, 13.0, MUTED, "start")}
                    {check(1642.0, 348.0, 24.0, LIME, gate1)}
                </g>
                <g opacity={gate2}>
                    {card(1360.0, 414.0, 340.0, 70.0, LINE, CARD, 1.0)}
                    {mono("GET /health", 1384.0, 456.0, 13.0, MUTED, "start")}
                    {check(1642.0, 432.0, 24.0, LIME, gate2)}
                </g>

                <g opacity={done}>
                    <circle cx="1540" cy="790" r="36" fill="none" stroke={LINE} stroke-width="2" />
                    <circle cx="1540" cy="790" r="36" fill="none" stroke={LIME} stroke-width="3"
                        stroke-dasharray={circ} stroke-dashoffset={circ * dp} stroke-linecap="round"
                        transform="rotate(-90 1540 790)" />
                    <text x="1540" y="800" text-anchor="middle" font-family={MONO} font-size="20" font-weight={W_SEMI} fill={INK}>
                        {format!("{remain}")}
                    </text>
                    {mono("drain", 1540.0, 848.0, 12.0, DIM, "middle")}
                </g>

                <g opacity={p(&frame, 3.6)}>
                    {pill(180.0, 850.0, 360.0, 56.0, "still 200 OK", LIME, "#16220F", 1.0, 16.0)}
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 05 — Rollback is one command.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Rollback;

impl Scene for Rollback {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let head = title(&mut frame, ctx, "Rollback is one command.", 160.0, 270.0, 76, 1500, INK, -0.4);
        let command = "swapdock rollback blog";
        let n = (command.chars().count() as f32 * p(&frame, 0.3)) as usize;
        let shown: String = command.chars().take(n).collect();
        let shown_chars = shown.chars().count() as f32;
        let output = p(&frame, 1.2);
        let old = p(&frame, -0.2);
        let bad = 1.0 - 0.62 * p(&frame, 1.6);
        let live = p(&frame, 1.7);
        let undo = draw(&frame, 1.5, 250.0);
        let s = frame.seconds();
        let caret = 0.35 + 0.65 * (s * 3.0).sin().abs();

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("05", "ROLLBACK")}
                {head}

                <g opacity={p(&frame, -0.2)}>
                    {card(200.0, 390.0, 780.0, 150.0, LINE, CARD, 1.0)}
                    <text x="240" y="452" font-family={MONO} font-size="24" fill={LIME}>"$"</text>
                    <text x="274" y="452" font-family={MONO} font-size="24" fill={INK}>{shown}</text>
                    <rect x={274.0 + shown_chars * 14.5} y="430" width="3" height="26" fill={LIME} opacity={caret} />
                    <g opacity={output}>
                        {check(240.0, 504.0, 24.0, LIME, output)}
                        {mono("v41 live again", 278.0, 522.0, 18.0, MUTED, "start")}
                    </g>
                </g>

                <g opacity={old}>
                    {card(1120.0, 330.0, 600.0, 150.0, RED, CARD, 1.0 * bad)}
                    {mono("v42", 1154.0, 380.0, 16.0, RED, "start")}
                    <text x="1154" y="438" font-size="30" font-weight={W_SEMI} fill={INK} opacity={bad}>"bad deploy"</text>
                </g>

                <g opacity={old}>
                    {card(1120.0, 600.0, 600.0, 150.0, LIME, CARD, 1.0)}
                    {mono("v41", 1154.0, 650.0, 16.0, LIME, "start")}
                    <text x="1154" y="708" font-size="30" font-weight={W_SEMI} fill={INK}>"previous release"</text>
                    <g opacity={live}>
                        {pill(1580.0, 632.0, 108.0, 32.0, "live", LIME, "#16220F", 1.0, 12.0)}
                    </g>
                </g>

                <path d="M1420 486 C 1500 520, 1500 570, 1420 596" stroke={LIME} stroke-width="2.4" fill="none"
                    stroke-dasharray="250" stroke-dashoffset={undo} />
                <g opacity={p(&frame, 1.7)}>
                    {check(1500.0, 520.0, 30.0, LIME, live)}
                </g>

                <g opacity={p(&frame, 2.5)}>
                    <text x="160" y="680" font-size="52" font-weight={W_SEMI} fill={LIME} letter-spacing="-1.2">
                        "Bad deploy? Undo it."
                    </text>
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 06 — No Kubernetes.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct NoK8s;

impl Scene for NoK8s {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let head = title(&mut frame, ctx, "No Kubernetes.", 160.0, 270.0, 110, 1400, INK, -0.4);
        let caption = sub(&frame, "One box. One binary. nginx.", 166.0, 334.0, -0.2, MUTED, 34.0);
        let apps = ["blog", "api", "docs"];
        let app_svgs: Vec<Svgr> = apps
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let start = -0.6 + i as f32 * 0.08;
                fframes::svgr!(
                    <g opacity={p(&frame, start)} transform={Transform::translate(0.0, rise(&frame, start))}>
                        <rect x={320.0 + i as f32 * 220.0} y="580.0" width="190.0" height="70.0" rx="12" fill={CARD2} stroke={LINE} />
                        <text x={415.0 + i as f32 * 220.0} y="624.0" text-anchor="middle" font-family={MONO} font-size="19" fill={INK}>
                            {*name}
                        </text>
                    </g>
                )
            })
            .collect();
        let cross = draw(&frame, 1.4, 320.0);
        let stateful = p(&frame, 2.6);

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("06", "ONE BOX")}
                {head}
                {caption}

                <g opacity={p(&frame, -0.2)} transform={Transform::translate(0.0, rise(&frame, -0.2))}>
                    {card(260.0, 420.0, 800.0, 420.0, LIME, CARD, 1.0)}
                    {mono("ONE VPS", 296.0, 464.0, 14.0, LIME, "start")}
                    <rect x="296" y="488" width="728" height="10" rx="5" fill={LIME} opacity="0.25" />
                    <text x="296" y="546" font-size="26" font-weight={W_SEMI} fill={INK}>"nginx"</text>
                    {mono(":80  ·  :443", 296.0, 576.0, 15.0, MUTED, "start")}
                    {app_svgs}
                    {mono("swapdock  ·  one binary", 296.0, 736.0, 14.0, LIME, "start")}
                </g>

                <g opacity={p(&frame, -0.2)}>
                    <path d="M1440 510 L1530 560 L1530 660 L1440 710 L1350 660 L1350 560 Z" fill="none" stroke="#4A4B54" stroke-width="2.5" />
                    <path d="M1440 560 L1440 660 M1370 610 L1510 610" stroke="#4A4B54" stroke-width="1.8" />
                    <text x="1440" y="620" text-anchor="middle" font-family={MONO} font-size="16" fill={MUTED}>"k8s"</text>
                    {mono("control plane", 1440.0, 760.0, 13.0, MUTED, "middle")}
                </g>
                <g opacity={p(&frame, 1.4)}>
                    <path d="M1340 490 L1540 730" stroke={RED} stroke-width="6" stroke-linecap="round" stroke-dasharray="320" stroke-dashoffset={cross} />
                </g>
                <g opacity={p(&frame, 1.8)}>
                    {mono("NOT REQUIRED", 1440.0, 806.0, 14.0, RED, "middle")}
                </g>

                <g opacity={stateful}>
                    {pill(260.0, 880.0, 560.0, 54.0, "SQLite  ·  Postgres  ·  a disk", LIME, "#16220F", 1.0, 14.0)}
                    {mono("stateful apps included", 840.0, 914.0, 13.0, MUTED, "start")}
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 07 — Outro.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Outro;

impl Scene for Outro {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(5.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let mark = p(&frame, -0.3);
        let word = p(&frame, -0.05);
        let title_size = fit_size(&mut frame, ctx, "Deploy on a Tuesday.", 96, 1500);
        let title = p(&frame, 0.8);
        let tag = p(&frame, 2.0);
        let meta = p(&frame, 2.7);

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("07", "END")}

                <g opacity={mark}>
                    {logo(880.0, 240.0, 2.2, 1.0)}
                </g>
                <g opacity={word}>
                    <text x="960" y="400" text-anchor="middle" font-family={SANS} font-size="58" font-weight={W_SEMI} fill={INK} letter-spacing="-1.8">
                        "swapdock"
                    </text>
                </g>

                <g opacity={title} transform={Transform::translate(0.0, rise(&frame, 0.8))}>
                    <text x="960" y="610" text-anchor="middle" font-family={SANS} font-size={title_size} font-weight={W_SEMI} fill={INK} letter-spacing="-2.8">
                        "Deploy on a Tuesday."
                    </text>
                </g>
                <g opacity={tag}>
                    <text x="960" y="692" text-anchor="middle" font-family={SANS} font-size="40" font-weight={W_REG} fill={LIME}>
                        "Make the next deploy boring."
                    </text>
                </g>

                <g opacity={meta}>
                    {mono("swapdock.dev", 960.0, 790.0, 22.0, INK, "middle")}
                    {mono("OPEN SOURCE  ·  MIT  ·  PRE-LAUNCH v0.2", 960.0, 826.0, 13.0, DIM, "middle")}
                </g>

            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// The video.
// ---------------------------------------------------------------------------

pub struct LaunchVideoVideo<'a> {
    pub media: &'a LaunchVideoMedia,
    ship: Ship,
    problem: Problem,
    command: Command,
    window: Window,
    rollback: Rollback,
    nok8s: NoK8s,
    outro: Outro,
}

impl<'a> LaunchVideoVideo<'a> {
    pub fn new(media: &'a LaunchVideoMedia, _title: &'a str) -> Self {
        Self {
            media,
            ship: Ship,
            problem: Problem,
            command: Command,
            window: Window,
            rollback: Rollback,
            nok8s: NoK8s,
            outro: Outro,
        }
    }
}

impl std::fmt::Debug for LaunchVideoVideo<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchVideoVideo").finish()
    }
}

impl Video for LaunchVideoVideo<'_> {
    const FPS: usize = FPS;
    const WIDTH: usize = WIDTH;
    const HEIGHT: usize = HEIGHT;
    const BACKGROUND_COLOR: Color = Color::BLACK;

    fn duration(&self) -> Duration<'_> {
        Duration::Auto
    }

    fn audio(&self) -> AudioMap<'_> {
        AudioMap::from([AudioTrack::new("bed.wav", Second(0.0)..Eof)])
    }

    fn define_scenes(&self) -> Scenes<'_> {
        Scenes::from(vec![
            &self.ship as &dyn Scene,
            &self.problem,
            &self.command,
            &self.window,
            &self.rollback,
            &self.nok8s,
            &self.outro,
        ])
    }

    fn render_frame<'a>(&'a self, frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.global_index as f32 / FPS as f32;
        let progress = (t / TOTAL * 1600.0).clamp(0.5, 1600.0);
        let beat = (0.5 + 0.5 * (t * 2.0 * std::f32::consts::PI * 2.0).sin()) as f32;
        fframes::svgr!(
            <svg xmlns="http://www.w3.org/2000/svg" width={WIDTH} height={HEIGHT} viewBox="0 0 1920 1080">
                <defs>
                    <pattern id="grid" width="80" height="80" patternUnits="userSpaceOnUse">
                        <path d="M80 0H0V80" fill="none" stroke="#FFFFFF" stroke-opacity="0.028" />
                    </pattern>
                    <radialGradient id="glowA" cx="50%" cy="50%" r="50%">
                        <stop offset="0" stop-color={ACCENT} stop-opacity="0.30" />
                        <stop offset="1" stop-color={ACCENT} stop-opacity="0" />
                    </radialGradient>
                    <radialGradient id="glowB" cx="50%" cy="50%" r="50%">
                        <stop offset="0" stop-color={LIME} stop-opacity="0.16" />
                        <stop offset="1" stop-color={LIME} stop-opacity="0" />
                    </radialGradient>
                </defs>
                <rect width="1920" height="1080" fill={BG} />
                <rect width="1920" height="1080" fill="url(#grid)" />
                <circle cx={1500.0 + (t * 0.20).sin() * 90.0} cy={180.0 + (t * 0.13).cos() * 70.0} r="900" fill="url(#glowA)" opacity={0.35 + 0.12 * beat} />
                <circle cx={260.0 + (t * 0.16).cos() * 80.0} cy={900.0 + (t * 0.11).sin() * 60.0} r="760" fill="url(#glowB)" opacity="0.45" />
                {ctx.render_scenes(&frame)}
                <rect x="160" y="1012" width="1600" height="2" fill="#22242A" />
                <rect x="160" y="1012" width={progress} height="2" fill={ACCENT} />
            </svg>
        )
    }
}
