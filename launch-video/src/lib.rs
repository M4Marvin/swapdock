//! swapdock — launch film, 34 seconds, 1920x1080 at 30 fps.
//!
//! The story: a deploy you did not notice, the three places that usually disagree,
//! the loop that fixes it, the two strategies, crash recovery, and the call to action.
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
pub const TOTAL: f32 = 34.0;

const BG: &str = "#08090A";
const CARD: &str = "#0E0F12";
const CARD2: &str = "#141519";
const INK: &str = "#F3F2EF";
const MUTED: &str = "#9B9CA4";
const DIM: &str = "#6F7079";
const LINE: &str = "#27282E";
const ACCENT: &str = "#8B8BFF";
const LIME: &str = "#B6F09C";
const RED: &str = "#F2616B";
const AMBER: &str = "#F2B544";

const SANS: &str = "Geist";
const MONO: &str = "IBM Plex Mono";
const W_SEMI: u16 = 600;
const W_MED: u16 = 500;
const W_REG: u16 = 400;

static EASE_OUT: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(0.65, &Easing::EaseOut));
static EASE_OUT_SLOW: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(1.0, &Easing::EaseOut));
static EASE_IN: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(0.28, &Easing::EaseIn));
static EASE_INOUT: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(0.8, &Easing::EaseInOut));
static SPRING: LazyLock<AnimationRuntime> = LazyLock::new(|| {
    AnimationRuntime::new(
        2.2,
        &Easing::Spring {
            mass: 1.0,
            stiffness: 220.0,
            damping: 24.0,
        },
    )
});
static DRAIN: LazyLock<AnimationRuntime> =
    LazyLock::new(|| AnimationRuntime::new(3.0, &Easing::EaseInOut));

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
        from: 44.0,
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

fn grow(frame: &Frame, start: f32, to: f32) -> f32 {
    frame
        .animate_runtime(AnimateRuntimeInput {
            on_second: start,
            from: 0.5,
            to,
            animation_runtime: &EASE_INOUT,
        })
        .max(0.5)
}

fn drain_p(frame: &Frame, start: f32) -> f32 {
    frame.animate_runtime(AnimateRuntimeInput {
        on_second: start,
        from: 0.0,
        to: 1.0,
        animation_runtime: &DRAIN,
    })
}

/// Largest font size at which `text` fits `max_width` at weight 600.
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

fn chrome<'a>(scene_no: &'a str, scene_name: &'a str) -> Svgr<'a> {
    fframes::svgr!(
        <g>
            {logo(160.0, 78.0, 1.0, 0.9)}
            <text x="232" y="101" font-family={SANS} font-size="25" font-weight={W_SEMI} fill={INK} letter-spacing="-0.5">
                "swapdock"
            </text>
            <text x="1760" y="100" text-anchor="end" font-family={MONO} font-size="17" fill={DIM} letter-spacing="1.5">
                {scene_no} "  /  " {scene_name}
            </text>
        </g>
    )
}

fn heading<'a>(
    frame: &mut Frame,
    ctx: &FFramesContext<'a, '_>,
    text: &'a str,
    x: f32,
    y: f32,
    preferred: usize,
    max_width: usize,
    fill: &'a str,
    start: f32,
    leave: Option<f32>,
) -> Svgr<'a> {
    let size = fit_size(frame, ctx, text, preferred, max_width);
    let opacity = p(frame, start) * leave.map(|l| out(frame, l)).unwrap_or(1.0);
    let dy = rise(frame, start);
    fframes::svgr!(
        <g transform={Transform::translate(0.0, dy)} opacity={opacity}>
            <text x={x} y={y} font-family={SANS} font-size={size} font-weight={W_SEMI} fill={fill} letter-spacing="-2.2">
                {text}
            </text>
        </g>
    )
}

fn sub<'a>(
    frame: &Frame,
    text: &'a str,
    x: f32,
    y: f32,
    start: f32,
    fill: &'a str,
    size: f32,
) -> Svgr<'a> {
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

fn card<'a>(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    stroke: &'a str,
    fill: &'a str,
    opacity: f32,
) -> Svgr<'a> {
    fframes::svgr!(
        <rect x={x} y={y} width={w} height={h} rx="16" fill={fill} stroke={stroke} stroke-width="1.4" opacity={opacity} />
    )
}

// ---------------------------------------------------------------------------
// 01 — Hook. A deploy happens, and nobody notices.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Hook;

impl Scene for Hook {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let s = frame.seconds();
        let new_in = p(&frame, 0.35);
        let new_dx = (1.0 - new_in) * 90.0;
        let health = p(&frame, 1.25);
        let old_dim = 1.0 - 0.62 * p(&frame, 2.3);
        let line = draw(&frame, 0.55, 560.0);
        let dot = p_slow(&frame, 1.45);
        let reveal = p(&frame, 2.55);

        let title_size = fit_size(&mut frame, ctx, "You didn't notice.", 78, 1550);

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("01", "DEPLOY 128")}

                <g opacity={p(&frame, 0.15)}>
                    {mono("front door", 200.0, 405.0, 16.0, DIM, "start")}
                </g>

                <g transform={Transform::translate(0.0, rise(&frame, -0.2))} opacity={p(&frame, -0.2)}>
                    {card(200.0, 430.0, 340.0, 160.0, LINE, CARD, 1.0)}
                    <text x="232" y="482" font-size="27" font-weight={W_SEMI} fill={INK}>"nginx"</text>
                    {mono(":8001", 232.0, 528.0, 26.0, MUTED, "start")}
                    {mono("ONE FRONT PORT", 232.0, 562.0, 13.0, DIM, "start")}
                </g>

                <g transform={Transform::translate(0.0, rise(&frame, -0.3))} opacity={old_dim * p(&frame, -0.3)}>
                    {card(1040.0, 620.0, 660.0, 170.0, LINE, CARD, 1.0)}
                    <text x="1074" y="674" font-size="26" font-weight={W_SEMI} fill={INK}>"blog"</text>
                    {mono("previous", 1074.0, 708.0, 15.0, DIM, "start")}
                    {mono(":9000", 1074.0, 752.0, 27.0, DIM, "start")}
                    {pill(1548.0, 676.0, 118.0, 32.0, "kept", DIM, CARD2, 1.0, 12.0)}
                </g>

                <g transform={Transform::translate(new_dx, rise(&frame, 0.35))} opacity={new_in}>
                    {card(1040.0, 330.0, 660.0, 200.0, LIME, CARD, 1.0)}
                    <text x="1074" y="386" font-size="27" font-weight={W_SEMI} fill={INK}>"blog"</text>
                    {mono("9c1f2ab", 1074.0, 422.0, 19.0, MUTED, "start")}
                    {mono(":9001", 1074.0, 486.0, 28.0, LIME, "start")}
                    {pill(1540.0, 356.0, 126.0, 34.0, "healthy", LIME, "#1A2A16", health, 12.0)}
                    <g opacity={health}>
                        <circle cx="1640" cy="470" r="26" fill="none" stroke={LIME} stroke-opacity="0.25" stroke-width="1.5" />
                        <circle cx="1640" cy="470" r={6.0 + 3.0 * (s * 2.6).sin().abs()} fill={LIME} />
                    </g>
                </g>

                <path d="M540 510 L1040 430" stroke={LIME} stroke-width="2" stroke-dasharray="560" stroke-dashoffset={line} />
                <path d="M540 510 L1040 705" stroke={DIM} stroke-width="1.5" stroke-dasharray="5 6" opacity={0.55 * p(&frame, -0.2)} />

                <circle cx={540.0 + 500.0 * dot} cy={510.0 - 80.0 * dot} r="4.5" fill={LIME} opacity={if dot > 0.0 && dot < 1.0 { 1.0 } else { 0.0 }} />

                <g opacity={reveal} transform={Transform::translate(0.0, rise(&frame, 2.55))}>
                    <text x="200" y="262" font-family={SANS} font-size={title_size} font-weight={W_SEMI} fill={INK} letter-spacing="-2.5">
                        "You didn't notice."
                    </text>
                    {mono("THAT'S THE POINT", 204.0, 300.0, 15.0, LIME, "start")}
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 02 — The problem: three places, three truths.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Problem;

impl Scene for Problem {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let s = frame.seconds();
        let leave = 3.55;
        let glitch = if (3.1..3.35).contains(&s) {
            (s * 95.0).sin() * 6.0
        } else {
            0.0
        };
        let title_svgr = heading(
            &mut frame, ctx, "Three places say what's live.", 160.0, 258.0, 64, 1500, INK, -0.5, Some(leave),
        );
        let sub_svgr = heading(
            &mut frame, ctx, "They disagree.", 160.0, 340.0, 64, 1200, RED, 1.9, Some(leave),
        );

        let cards = [
            ("REGISTRY", "release", "9c1f2a0", "v41", DIM),
            ("NGINX", "upstream", ":9002", "v42", RED),
            ("CONTAINER", "image", "9c1f2ac", "v43", DIM),
        ];
        let card_svgs: Vec<Svgr> = cards
            .iter()
            .enumerate()
            .map(|(i, (name, field, value, tag, tint))| {
                let start = -0.45 + i as f32 * 0.08;
                let x = 160.0 + i as f32 * 550.0 + glitch * (1.0 + i as f32 * 0.35);
                let op = p(&frame, start) * out(&frame, leave);
                let dy = rise(&frame, start);
                let blocked = i == 1;
                fframes::svgr!(
                    <g transform={Transform::translate(x, dy)} opacity={op}>
                        {card(0.0, 470.0, 500.0, 280.0, if blocked { RED } else { LINE }, CARD, 1.0)}
                        {mono(name, 34.0, 520.0, 15.0, DIM, "start")}
                        <text x="34" y="610" font-family={SANS} font-size="54" font-weight={W_SEMI} fill={if blocked { RED } else { INK }} letter-spacing="-1.5">
                            {*tag}
                        </text>
                        {mono(field, 34.0, 656.0, 15.0, DIM, "start")}
                        {mono(value, 34.0, 700.0, 27.0, if blocked { RED } else { MUTED }, "start")}
                        <circle cx="470" cy="505" r="4" fill={*tint} />
                    </g>
                )
            })
            .collect();

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("02", "THE PROBLEM")}
                {title_svgr}
                {sub_svgr}
                {card_svgs}
                <g opacity={p(&frame, 2.5) * out(&frame, leave)}>
                    {mono("ONE OF THEM IS WRONG", 160.0, 838.0, 15.0, DIM, "start")}
                    <path d="M160 770 H1760" stroke={LINE} stroke-width="1" />
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 03 — The turn: one loop.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Turn;

impl Scene for Turn {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(2.5)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let leave = 2.15;
        let title = heading(
            &mut frame, ctx, "One loop.", 160.0, 360.0, 118, 900, INK, -0.5, Some(leave),
        );
        let caption = sub(&frame, "Every deploy follows the same sequence.", 160.0, 420.0, 0.75, MUTED, 30.0);

        let merge = p_slow(&frame, 1.0);
        let chips = [
            ("registry", 500.0, 570.0),
            ("nginx", 810.0, 570.0),
            ("container", 1120.0, 570.0),
        ];
        let chip_svgs: Vec<Svgr> = chips
            .iter()
            .enumerate()
            .map(|(i, (label, x, y))| {
                let start = -0.2 + i as f32 * 0.08;
                let cx = 860.0 + (*x - 860.0) * (1.0 - merge);
                let op = p(&frame, start) * (1.0 - merge);
                fframes::svgr!(
                    <g opacity={op} transform={Transform::translate(cx - x, 0.0)}>
                        <rect x={*x} y={*y} width="230" height="66" rx="33" fill={CARD2} stroke={LINE} stroke-width="1.2" />
                        <text x={*x + 115.0} y={*y + 42.0} text-anchor="middle" font-family={MONO} font-size="21" font-weight={W_MED} fill={MUTED}>
                            {*label}
                        </text>
                    </g>
                )
            })
            .collect();

        let merged = p_slow(&frame, 1.3);
        let loop_len = 2100.0;
        let loop_draw = draw(&frame, 1.05, loop_len);
        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("03", "THE LOOP")}
                {title}
                {caption}
                {chip_svgs}
                <path d="M500 700 H1420" stroke={LINE} stroke-width="1" opacity={0.8 * (1.0 - merge)} />
                <rect x="560" y="480" width="800" height="300" rx="60" fill="none" stroke={ACCENT} stroke-width="1.5"
                    stroke-dasharray={loop_len} stroke-dashoffset={loop_draw} opacity={0.8} />
                <g opacity={merged}>
                    {pill(760.0, 610.0, 400.0, 66.0, "one source of truth", LIME, "#16220F", merged, 19.0)}
                    {check(1195.0, 628.0, 30.0, LIME, merged)}
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 04 — Start the new release beside the old, then gate it.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Green;

impl Scene for Green {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(5.5)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let leave = 5.1;
        let title = heading(
            &mut frame, ctx, "Start beside the old.", 160.0, 260.0, 66, 1450, INK, -0.5, Some(leave),
        );
        let caption = sub(&frame, "Traffic has not moved yet.", 160.0, 315.0, 0.7, MUTED, 30.0);

        let new_in = p(&frame, 0.35);
        let gate1 = p(&frame, 1.35);
        let gate2 = p(&frame, 2.25);
        let ready = p(&frame, 3.9);
        let bar = grow(&frame, 0.9, 600.0);

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("04", "GREEN")}
                {title}
                {caption}

                {mono("9000  /  OLD", 160.0, 430.0, 15.0, DIM, "start")}
                {mono("9001  /  GREEN", 160.0, 666.0, 15.0, LIME, "start")}
                <path d="M160 446 H1760" stroke={LINE} stroke-width="1" opacity="0.6" />
                <path d="M160 682 H1760" stroke={LINE} stroke-width="1" opacity="0.6" />

                <g opacity={0.45}>
                    {card(200.0, 470.0, 620.0, 130.0, LINE, CARD, 1.0)}
                    <text x="234" y="520" font-size="24" font-weight={W_SEMI} fill={INK}>"blog"</text>
                    {mono(":9000  ·  previous release", 234.0, 558.0, 16.0, DIM, "start")}
                </g>

                <g opacity={new_in} transform={Transform::translate((1.0 - new_in) * 70.0, rise(&frame, 0.35))}>
                    {card(200.0, 706.0, 620.0, 130.0, LIME, CARD, 1.0)}
                    <text x="234" y="756" font-size="24" font-weight={W_SEMI} fill={INK}>"blog"</text>
                    {mono(":9001  ·  9c1f2ab", 234.0, 794.0, 16.0, LIME, "start")}
                </g>

                <g opacity={p(&frame, 0.5)}>
                    {mono("container health", 1000.0, 505.0, 16.0, MUTED, "start")}
                    <rect x="1000" y="520" width="700" height="4" rx="2" fill={LINE} />
                    <rect x="1000" y="520" width={bar} height="4" rx="2" fill={LIME} />
                    {check(1608.0, 490.0, 26.0, LIME, gate1)}
                    {mono("healthy", 1700.0, 510.0, 15.0, LIME, "end")}
                </g>

                <g opacity={p(&frame, 0.9)}>
                    {mono("GET /health", 1000.0, 625.0, 16.0, MUTED, "start")}
                    <rect x="1000" y="640" width="700" height="4" rx="2" fill={LINE} />
                    <rect x="1000" y="640" width={grow(&frame, 1.7, 700.0)} height="4" rx="2" fill={LIME} />
                    {check(1632.0, 610.0, 26.0, LIME, gate2)}
                    {mono("200", 1700.0, 630.0, 15.0, LIME, "end")}
                </g>

                <g opacity={ready} transform={Transform::translate(0.0, rise(&frame, 3.9))}>
                    {pill(200.0, 860.0, 300.0, 52.0, "ready to flip", LIME, "#16220F", 1.0, 17.0)}
                    {check(520.0, 873.0, 28.0, LIME, ready)}
                </g>

                <g opacity={p(&frame, 3.2)}>
                    {mono("NO MAINTENANCE WINDOW", 160.0, 962.0, 14.0, DIM, "start")}
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 05 — Flip, drain, commit last.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Flip;

impl Scene for Flip {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(5.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let leave = 4.6;
        let title = heading(
            &mut frame, ctx, "Traffic moves once.", 160.0, 260.0, 66, 1400, INK, -0.5, Some(leave),
        );
        let commit = sub(&frame, "The registry commits last.", 160.0, 315.0, 0.8, MUTED, 30.0);

        let pulse = p_slow(&frame, 0.7);
        let old_dim = 1.0 - 0.55 * p(&frame, 2.5);
        let dp = drain_p(&frame, 1.0);
        let circ = 2.0 * std::f32::consts::PI * 62.0;
        let remain = (15.0 * (1.0 - dp)).ceil() as i32;
        let typed_line = "release = \"9c1f2ab\"";
        let n = (typed_line.chars().count() as f32 * p(&frame, 3.3)) as usize;
        let shown: String = typed_line.chars().take(n).collect();
        let done = p(&frame, 4.1);

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("05", "FLIP")}
                {title}
                {commit}

                <g opacity={p(&frame, -0.3)}>
                    {card(160.0, 470.0, 320.0, 150.0, LINE, CARD, 1.0)}
                    <text x="192" y="520" font-size="25" font-weight={W_SEMI} fill={INK}>"nginx"</text>
                    {mono(":8001", 192.0, 568.0, 24.0, MUTED, "start")}
                </g>

                <g opacity={old_dim * p(&frame, -0.2)}>
                    {card(720.0, 420.0, 520.0, 130.0, LINE, CARD, 1.0)}
                    <text x="754" y="470" font-size="23" font-weight={W_SEMI} fill={INK}>"blog"</text>
                    {mono(":9000  ·  old", 754.0, 508.0, 15.0, DIM, "start")}
                    <g opacity={p(&frame, 2.65)}>
                        {pill(1080.0, 452.0, 128.0, 34.0, "stopped", DIM, CARD2, 1.0, 12.0)}
                    </g>
                </g>

                <g opacity={p(&frame, -0.1)}>
                    {card(720.0, 660.0, 520.0, 130.0, LIME, CARD, 1.0)}
                    <text x="754" y="710" font-size="23" font-weight={W_SEMI} fill={INK}>"blog"</text>
                    {mono(":9001  ·  9c1f2ab", 754.0, 748.0, 15.0, LIME, "start")}
                </g>

                <path d="M480 545 L720 485" stroke={DIM} stroke-width="1.5" stroke-dasharray="5 6" opacity="0.5" />
                <path d="M480 545 C 570 545, 610 725, 720 725" stroke={LIME} stroke-width="2" stroke-dasharray="420" stroke-dashoffset={draw(&frame, 0.5, 420.0)} />
                <circle cx={480.0 + 240.0 * pulse} cy={545.0 + 180.0 * pulse * pulse} r="4.5" fill={LIME} opacity={if pulse > 0.0 && pulse < 1.0 { 1.0 } else { 0.0 }} />

                <g opacity={p(&frame, 1.0)}>
                    <circle cx="1500" cy="360" r="62" fill="none" stroke={LINE} stroke-width="2" />
                    <circle cx="1500" cy="360" r="62" fill="none" stroke={LIME} stroke-width="3"
                        stroke-dasharray={circ} stroke-dashoffset={circ * dp} stroke-linecap="round"
                        transform="rotate(-90 1500 360)" />
                    <text x="1500" y="376" text-anchor="middle" font-family={MONO} font-size="34" font-weight={W_SEMI} fill={INK}>
                        {format!("{remain}")}
                    </text>
                    {mono("drain", 1500.0, 462.0, 14.0, DIM, "middle")}
                </g>

                <g opacity={p(&frame, 3.1)}>
                    {card(160.0, 840.0, 1600.0, 86.0, LINE, CARD, 1.0)}
                    {mono("commit", 196.0, 892.0, 13.0, DIM, "start")}
                    <text x="320" y="893" font-family={MONO} font-size="22" font-weight={W_MED} fill={INK}>{shown}</text>
                    {check(1644.0, 866.0, 28.0, LIME, done)}
                </g>

                <g opacity={p(&frame, 4.2)}>
                    {mono("DRAIN 15 s", 160.0, 962.0, 14.0, DIM, "start")}
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 06 — Stateful apps take the other road.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Stateful;

impl Scene for Stateful {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(5.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let leave = 4.6;
        let title_svgr = heading(
            &mut frame, ctx, "SQLite can't run twice.", 160.0, 260.0, 64, 1500, INK, -0.5, Some(leave),
        );
        let second_svgr = heading(
            &mut frame, ctx, "So it doesn't.", 160.0, 340.0, 64, 900, LIME, 2.55, Some(leave),
        );
        let barrier = p(&frame, 1.3);
        let blocked = p(&frame, 1.55);
        let path = p(&frame, 2.8);
        let r1 = p(&frame, 3.0);
        let r2 = p(&frame, 3.35);
        let r3 = p(&frame, 3.7);
        let b_stroke = if blocked > 0.1 { RED } else { LINE };
        let b_fg = if blocked > 0.1 { RED } else { MUTED };

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("06", "STATEFUL")}
                {title_svgr}
                {second_svgr}

                <g opacity={p(&frame, -0.2)}>
                    <ellipse cx="960" cy="462" rx="86" ry="22" fill={CARD2} stroke={LINE} stroke-width="1.3" />
                    <path d="M874 462 v170 a86 22 0 0 0 172 0 v-170" fill={CARD} stroke={LINE} stroke-width="1.3" />
                    <ellipse cx="960" cy="560" rx="86" ry="22" fill="none" stroke={LINE} stroke-width="1" opacity="0.6" />
                    {mono("blog.sqlite", 960.0, 700.0, 19.0, MUTED, "middle")}
                </g>

                <g opacity={p(&frame, -0.2)}>
                    {card(240.0, 430.0, 340.0, 130.0, LINE, CARD, 1.0)}
                    <text x="274" y="480" font-size="21" font-weight={W_SEMI} fill={INK}>"container A"</text>
                    {mono("writing", 274.0, 518.0, 14.0, LIME, "start")}
                    <path d="M580 495 H866" stroke={LIME} stroke-width="1.5" stroke-dasharray="4 5" />
                </g>

                <g opacity={p(&frame, 0.85)}>
                    {card(1420.0, 430.0, 340.0, 130.0, b_stroke, CARD, 1.0)}
                    <text x="1454" y="480" font-size="21" font-weight={W_SEMI} fill={INK}>"container B"</text>
                    {mono("also writing", 1454.0, 518.0, 14.0, b_fg, "start")}
                    <path d="M1420 495 H1056" stroke={if blocked > 0.1 { RED } else { DIM }} stroke-width="1.5" stroke-dasharray="4 5" opacity={1.0 - 0.6 * blocked} />
                </g>

                <g opacity={barrier}>
                    <path d="M1240 400 V680" stroke={RED} stroke-width="3" />
                    <rect x="1050" y="700" width="380" height="38" rx="8" fill="#2A1215" stroke={RED} stroke-opacity="0.4" />
                    <text x="1240" y="726" text-anchor="middle" font-family={MONO} font-size="16" font-weight={W_MED} fill={RED}>
                        "writes_state = true"
                    </text>
                </g>

                <g opacity={blocked}>
                    <circle cx="1258" cy="495" r="16" fill={RED} opacity="0.15" />
                    <path d="M1252 489 L1264 501 M1264 489 L1252 501" stroke={RED} stroke-width="2" stroke-linecap="round" />
                </g>

                <g opacity={path}>
                    <path d="M160 780 H1760" stroke={LINE} stroke-width="1" />
                    {mono("REPLACE", 160.0, 748.0, 13.0, MUTED, "start")}
                </g>

                <g opacity={r1}>
                    {pill(560.0, 800.0, 190.0, 54.0, "stop", MUTED, CARD2, 1.0, 17.0)}
                </g>
                <path d="M760 827 H880" stroke={LINE} stroke-width="1.5" opacity={r2} />
                <g opacity={r2}>
                    {pill(890.0, 800.0, 190.0, 54.0, "start", AMBER, "#2A210E", 1.0, 17.0)}
                </g>
                <path d="M1090 827 H1210" stroke={LINE} stroke-width="1.5" opacity={r3} />
                <g opacity={r3}>
                    {pill(1220.0, 800.0, 230.0, 54.0, "healthy", LIME, "#16220F", 1.0, 17.0)}
                    {check(1478.0, 812.0, 28.0, LIME, r3)}
                </g>
                <g opacity={r2}>
                    {mono("1.2 s gap", 985.0, 890.0, 13.0, DIM, "middle")}
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 07 — Kill it mid-flip, and prove the state.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Recovery;

impl Scene for Recovery {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let s = frame.seconds();
        let leave = 3.65;
        let title = heading(
            &mut frame, ctx, "Kill it mid-flip.", 160.0, 260.0, 66, 1200, INK, -0.5, Some(leave),
        );
        let killed = if s > 1.5 { 1.0 } else { 0.0 };
        let evidence = p(&frame, 1.85);
        let c1 = p(&frame, 2.0);
        let c2 = p(&frame, 2.3);
        let c3 = p(&frame, 2.6);
        let done = p(&frame, 3.0);
        let caption = p(&frame, 1.35);

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("07", "RECOVERY")}
                {title}

                <g opacity={p(&frame, -0.2)}>
                    <path d="M280 600 H1640" stroke={LINE} stroke-width="2" opacity="0.55" />
                    <path d={format!("M280 600 H{}", 280.0 + grow(&frame, 0.25, 620.0))} stroke={LIME} stroke-width="2.4" />
                    {mono("start", 340.0, 660.0, 14.0, DIM, "middle")}
                    {mono("health", 620.0, 660.0, 14.0, DIM, "middle")}
                    {mono("flip", 900.0, 660.0, 14.0, if killed > 0.5 { RED } else { DIM }, "middle")}
                    {mono("stop", 1220.0, 660.0, 14.0, DIM, "middle")}
                    {mono("commit", 1520.0, 660.0, 14.0, DIM, "middle")}
                    <circle cx="340" cy="600" r="7" fill={LIME} />
                    <circle cx="620" cy="600" r="7" fill={LIME} />
                    <circle cx="900" cy="600" r="7" fill={if killed > 0.5 { RED } else { LIME }} />
                    <circle cx="1220" cy="600" r="7" fill={CARD} stroke={LINE} stroke-width="2" />
                    <circle cx="1520" cy="600" r="7" fill={if done > 0.5 { LIME } else { CARD }} stroke={if done > 0.5 { LIME } else { LINE }} stroke-width="2" />
                </g>

                <g opacity={killed * p(&frame, 1.2)}>
                    <path d="M900 570 L918 596 L902 606 L920 632" stroke={RED} stroke-width="3" fill="none" stroke-linecap="round" />
                    {mono("process killed", 900.0, 704.0, 14.0, RED, "middle")}
                </g>

                <g opacity={p(&frame, 1.6)}>
                    {card(700.0, 360.0, 760.0, 132.0, RED, CARD, 0.9)}
                    {mono("pending / blog.json", 724.0, 392.0, 13.0, RED, "start")}
                    {mono("{ \"phase\": \"flipped\",", 724.0, 428.0, 17.0, MUTED, "start")}
                    {mono("  \"release\": \"9c1f2ab\" }", 724.0, 456.0, 17.0, MUTED, "start")}
                </g>

                <g opacity={evidence}>
                    {mono("EVIDENCE", 160.0, 766.0, 13.0, DIM, "start")}
                </g>
                <g opacity={p(&frame, 1.85)}>
                    {check(180.0, 792.0, 26.0, LIME, c1)}
                    {mono("proxy_pass", 216.0, 812.0, 16.0, MUTED, "start")}
                </g>
                <g opacity={p(&frame, 2.1)}>
                    {check(570.0, 792.0, 26.0, LIME, c2)}
                    {mono("image id", 606.0, 812.0, 16.0, MUTED, "start")}
                </g>
                <g opacity={p(&frame, 2.4)}>
                    {check(960.0, 792.0, 26.0, LIME, c3)}
                    {mono("health", 996.0, 812.0, 16.0, MUTED, "start")}
                </g>

                <g opacity={caption}>
                    <text x="160" y="340" font-family={SANS} font-size="34" font-weight={W_REG} fill={MUTED}>
                        "It proves the state, or refuses."
                    </text>
                </g>
            </g>
        )
    }
}

// ---------------------------------------------------------------------------
// 08 — Outro.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Outro;

impl Scene for Outro {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.0)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let s = frame.seconds();
        let mark = p(&frame, -0.2);
        let word = p(&frame, 0.0);
        let title_size = fit_size(&mut frame, ctx, "Make the next deploy boring.", 92, 1500);
        let title = p(&frame, 0.8);
        let meta = p(&frame, 1.6);
        let cmd = p(&frame, 2.1);
        let flash = 0.5 + 0.5 * (s * 2.2).sin();

        fframes::svgr!(
            <g font-family={SANS}>
                {chrome("08", "END")}

                <g opacity={mark}>
                    {logo(880.0, 300.0, 2.2, 1.0)}
                    <path d="M916 336 H1004" stroke={INK} stroke-width="3" stroke-opacity="0.25" stroke-dasharray="90" stroke-dashoffset={draw(&frame, 0.3, 90.0)} />
                </g>

                <g opacity={word}>
                    <text x="960" y="480" text-anchor="middle" font-family={SANS} font-size="62" font-weight={W_SEMI} fill={INK} letter-spacing="-1.8">
                        "swapdock"
                    </text>
                </g>

                <g opacity={title} transform={Transform::translate(0.0, rise(&frame, 1.2))}>
                    <text x="960" y="650" text-anchor="middle" font-family={SANS} font-size={title_size} font-weight={W_SEMI} fill={INK} letter-spacing="-2.8">
                        "Make the next deploy boring."
                    </text>
                </g>

                <g opacity={meta}>
                    {mono("swapdock.dev", 960.0, 770.0, 24.0, INK, "middle")}
                    {mono("OPEN SOURCE  ·  MIT  ·  PRE-LAUNCH v0.2", 960.0, 808.0, 14.0, DIM, "middle")}
                </g>

                <g opacity={cmd}>
                    <rect x="560" y="850.0" width="800" height="58" rx="12" fill={CARD2} stroke={LINE} stroke-width="1" />
                    {mono("$  swapdock up blog --release 9c1f2ab", 590.0, 887.0, 18.0, MUTED, "start")}
                    <rect x="1252" y="864" width="8" height="30" fill={LIME} opacity={flash} />
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
    hook: Hook,
    problem: Problem,
    turn: Turn,
    green: Green,
    flip: Flip,
    stateful: Stateful,
    recovery: Recovery,
    outro: Outro,
}

impl<'a> LaunchVideoVideo<'a> {
    pub fn new(media: &'a LaunchVideoMedia, _title: &'a str) -> Self {
        Self {
            media,
            hook: Hook,
            problem: Problem,
            turn: Turn,
            green: Green,
            flip: Flip,
            stateful: Stateful,
            recovery: Recovery,
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
            &self.hook as &dyn Scene,
            &self.problem,
            &self.turn,
            &self.green,
            &self.flip,
            &self.stateful,
            &self.recovery,
            &self.outro,
        ])
    }

    fn render_frame<'a>(&'a self, frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.global_index as f32 / FPS as f32;
        let progress = (t / TOTAL * 1600.0).clamp(0.5, 1600.0);
        fframes::svgr!(
            <svg xmlns="http://www.w3.org/2000/svg" width={WIDTH} height={HEIGHT} viewBox="0 0 1920 1080">
                <defs>
                    <pattern id="grid" width="80" height="80" patternUnits="userSpaceOnUse">
                        <path d="M80 0H0V80" fill="none" stroke="#FFFFFF" stroke-opacity="0.032" />
                    </pattern>
                    <radialGradient id="glowA" cx="78%" cy="16%" r="58%">
                        <stop offset="0" stop-color={ACCENT} stop-opacity="0.16" />
                        <stop offset="1" stop-color={ACCENT} stop-opacity="0" />
                    </radialGradient>
                    <radialGradient id="glowB" cx="14%" cy="86%" r="52%">
                        <stop offset="0" stop-color={LIME} stop-opacity="0.06" />
                        <stop offset="1" stop-color={LIME} stop-opacity="0" />
                    </radialGradient>
                </defs>
                <rect width="1920" height="1080" fill={BG} />
                <rect width="1920" height="1080" fill="url(#grid)" />
                <rect width="1920" height="1080" fill="url(#glowA)" />
                <rect width="1920" height="1080" fill="url(#glowB)" />
                {ctx.render_scenes(&frame)}
                <rect x="160" y="1012" width="1600" height="2" fill="#22242A" />
                <rect x="160" y="1012" width={progress} height="2" fill={ACCENT} />
            </svg>
        )
    }
}
