//! The site.
//!
//! One self-contained HTML file, no network, no fonts, no scripts from
//! anywhere. It has to be openable from a USB stick on a machine with no
//! internet, because the point of the page is to be shown to somebody who does
//! not believe you.

use std::fmt::Write as _;

use crate::analyse::{Analysis, Track};
use crate::model::{band, rough_metres};

pub fn render(a: &Analysis) -> String {
    let mut h = String::new();
    let _ = write!(h, "{}", HEAD);

    let mins = (a.duration() as f32 / 60.0).max(0.1);
    let est = a.estimated_devices();

    // The headline is a sentence, not a number, because the number on its own
    // gets read as either scary or reassuring depending on the reader's mood.
    let _ = write!(
        h,
        r#"<header>
<h1>Within earshot</h1>
<p class="lede">In {:.0} minutes of listening — no monitor mode, no root, no
transmitting — this machine overheard <strong>{}</strong> addresses belonging to
roughly <strong>{}</strong> devices. {} of those addresses were permanent ones
that will still identify the same object next year.</p>
<p class="meta">{} sightings · {} · a passive read of the Bluetooth advertising
channels any desktop session can already see</p>
</header>"#,
        mins,
        a.tracks.len(),
        est,
        a.public_addrs(),
        a.sightings,
        window_text(a),
    );

    let _ = write!(h, "{}", room(a));
    let _ = write!(h, "{}", timeline(a));
    let _ = write!(h, "{}", devices(a));
    let _ = write!(h, "{}", limits(a));

    h.push_str("</main>\n");
    h
}

fn window_text(a: &Analysis) -> String {
    let d = a.duration();
    match d {
        0..=90 => format!("{d}s window"),
        91..=5400 => format!("{}m window", d / 60),
        _ => format!("{}h{}m window", d / 3600, (d % 3600) / 60),
    }
}

/// Distance rings. Deliberately not a radar: bearing is not observable from a
/// single antenna, and drawing one implies a direction this cannot know.
fn room(a: &Analysis) -> String {
    const W: f32 = 640.0;
    const C: f32 = W / 2.0;
    let rings = [(1.0, "1 m"), (3.0, "3 m"), (10.0, "10 m"), (30.0, "30 m")];
    // Log scale: the interesting difference is between 1 and 10 metres, not
    // between 30 and 40.
    let to_px = |m: f32| -> f32 { (m.max(0.5).log10() + 0.3) / (30f32.log10() + 0.3) * (C - 40.0) };

    let mut s = String::from(
        r#"<section><h2>How far away</h2>
<p class="note">Radius is distance estimated from signal strength. <strong>The angle
means nothing</strong> — one antenna cannot hear a direction, so each device is
placed at a fixed arbitrary bearing derived from its address. Anything claiming
to plot a bearing from a single receiver is showing you a decoration.</p>
<div class="figure">"#,
    );
    let _ = write!(s, r#"<svg viewBox="0 0 {W} {W}" role="img" aria-label="devices by estimated distance">"#);

    for (m, label) in rings {
        let r = to_px(m);
        let _ = write!(
            s,
            r#"<circle cx="{C}" cy="{C}" r="{r:.1}" class="ring"/><text x="{C}" y="{:.1}" class="ringlabel">{label}</text>"#,
            C - r - 6.0
        );
    }
    let _ = write!(s, r#"<circle cx="{C}" cy="{C}" r="4" class="here"/><text x="{C}" y="{:.1}" class="herelabel">here</text>"#, C + 20.0);

    for t in &a.tracks {
        let Some(rssi) = t.median_rssi.or(t.best_rssi) else { continue };
        let r = to_px(rough_metres(rssi));
        // Stable arbitrary bearing: the same address lands in the same place
        // every time the report is regenerated, which makes two reports
        // comparable without implying the position is real.
        let angle = (hash(&t.addr) % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
        let (x, y) = (C + r * angle.cos(), C + r * angle.sin());
        let class = if t.at == "public" { "dot public" } else { "dot" };
        let _ = write!(
            s,
            r#"<g class="{class}"><circle cx="{x:.1}" cy="{y:.1}" r="6"/><title>{} · {} dBm · {}</title></g>"#,
            esc(&t.label()),
            rssi,
            band(rssi)
        );
    }
    s.push_str("</svg></div>");
    s.push_str(r#"<p class="legend"><span class="swatch public"></span> permanent address &nbsp; <span class="swatch"></span> rotating address</p></section>"#);
    s
}

fn timeline(a: &Analysis) -> String {
    let dur = a.duration().max(1) as f32;
    let mut rows: Vec<&Track> = a.tracks.iter().collect();
    rows.sort_by_key(|t| t.first);

    let mut s = String::from(
        r#"<section><h2>When each address was audible</h2>
<p class="note">A rotating address that dies and is replaced by an identical-looking
one a few seconds later is the same device changing its number. Those are joined
below where the evidence supports it.</p><div class="tl">"#,
    );
    for t in rows {
        let left = (t.first.saturating_sub(a.window.0)) as f32 / dur * 100.0;
        let width = ((t.lifetime() as f32 / dur) * 100.0).max(0.6);
        let chained = a.chains.iter().any(|c| c.contains(&t.addr));
        let _ = write!(
            s,
            r#"<div class="tlrow"><span class="tlname">{}{}</span><span class="tlbar"><i style="left:{left:.2}%;width:{width:.2}%" class="{}"></i></span></div>"#,
            esc(&t.label()),
            if t.at == "public" { r#" <span class="pill">permanent</span>"# } else { "" },
            if chained { "seg chained" } else { "seg" },
        );
    }
    s.push_str("</div>");
    if !a.chains.is_empty() {
        let _ = write!(
            s,
            r#"<p class="note">{} rotation{} linked: an address stopped and a device with the
same advertising fingerprint appeared within 30 seconds. That is the crudest
possible linking rule and it still worked, which is the finding.</p>"#,
            a.chains.len(),
            if a.chains.len() == 1 { " was" } else { "s were" }
        );
    }
    s.push_str("</section>");
    s
}

fn devices(a: &Analysis) -> String {
    let mut rows: Vec<&Track> = a.tracks.iter().collect();
    rows.sort_by_key(|t| -(t.median_rssi.or(t.best_rssi).unwrap_or(-127) as i32));

    let mut s = String::from(r#"<section><h2>What each one said about itself</h2><div class="cards">"#);
    for t in rows {
        let rssi = t.median_rssi.or(t.best_rssi);
        let dist = rssi.map(|r| format!("{:.0} m · {}", rough_metres(r), band(r)))
            .unwrap_or_else(|| "not heard advertising".into());
        let leaks = t.leaks();
        let _ = write!(
            s,
            r#"<article class="card"><h3>{}</h3><p class="addr">{} <span class="pill{}">{}</span></p>
<p class="dist">{}{}</p>"#,
            esc(&t.label()),
            esc(&t.addr),
            if t.at == "public" { " on" } else { "" },
            esc(&t.at),
            dist,
            rssi.map(|r| format!(" · {r} dBm")).unwrap_or_default(),
        );
        if leaks.is_empty() {
            s.push_str(r#"<p class="none">Nothing beyond its presence and its distance. Which is still its presence and its distance.</p>"#);
        } else {
            s.push_str("<ul>");
            for l in leaks {
                let _ = write!(s, "<li>{}</li>", esc(&l));
            }
            s.push_str("</ul>");
        }
        s.push_str("</article>");
    }
    s.push_str("</div></section>");
    s
}

fn limits(a: &Analysis) -> String {
    let life = a
        .median_lifetime()
        .map(|r| format!("{r} seconds"))
        .unwrap_or_else(|| "not measurable in this window".into());
    format!(
        r#"<section class="limits"><h2>How good is this data, honestly</h2>
<dl>
<dt>It is Bluetooth only.</dt><dd>Wi-Fi probe requests are the richer source — a
phone that is not connected to anything asks after networks it has joined
before, which is a list of places its owner has been. Reading those needs the
radio in monitor mode, which most laptop Wi-Fi chips refuse to do.</dd>

<dt>Distance is a guess, not a measurement.</dt><dd>Signal strength converted with a
path-loss model. A body standing between you and a device costs about 10 dB,
which at these constants is a factor of 2.5 in apparent distance. Treat the
rings as "in the room / through a wall / in the building".</dd>

<dt>Randomization is doing its job here — mostly.</dt>
<dd>{} of {} addresses rotated. The {} that did not are permanent identifiers of
specific physical objects, and they will still be those objects next year.</dd>

<dt>An address going quiet is two different events.</dt>
<dd>Either the device rotated its identifier or it left. One receiver cannot
tell those apart, so the number below is an upper bound on how long an
identifier stayed usable — not a rotation interval. And {} of {} addresses were
still audible when the capture stopped, so their lifetimes are unknown rather
than long; counting them would make randomization look better the shorter you
listen. Excluding them, the median identifier lasted <strong>{life}</strong>.</dd>

<dt>The linking rule is the weakest one that works.</dt><dd>Matching on advertising
fingerprint and a 30-second handover gap is far below what published
de-randomization research achieves with frame timing, sequence numbers and
field ordering — none of which BlueZ even exposes. Whatever this found, a
determined observer finds more.</dd>

<dt>Nothing here was an attack.</dt><dd>No monitor mode, no injection, no root, nothing
transmitted. An unprivileged user account read a bus every desktop session can
read. That is the finding: this is the ambient, no-effort, default case.</dd>
</dl></section>
<footer>earshot · generated locally · no data left this machine</footer>"#,
        a.random_addrs(),
        a.tracks.len(),
        a.public_addrs(),
        a.censored(),
        a.tracks.len(),
    )
}

fn hash(s: &str) -> u64 {
    // FNV-1a. Stability across runs matters more than distribution quality.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const HEAD: &str = r#"<!doctype html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Within earshot</title>
<style>
:root{--bg:#fbfaf8;--fg:#16150f;--dim:#6a675c;--line:#e2ded2;--card:#fff;--acc:#b4552b;--pub:#b4552b;--rot:#7c8b7a}
@media (prefers-color-scheme:dark){:root{--bg:#100f0d;--fg:#eceae2;--dim:#8f8b7e;--line:#2a2823;--card:#181713;--acc:#e08a5a;--pub:#e08a5a;--rot:#8ea88b}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.6 ui-serif,Georgia,serif;-webkit-font-smoothing:antialiased}
main,header{max-width:860px;margin:0 auto;padding:0 24px}
header{padding-top:72px;padding-bottom:8px}
h1{font-size:clamp(2.2rem,6vw,3.4rem);line-height:1.05;margin:0 0 .4em;letter-spacing:-.02em}
h2{font-size:1.4rem;margin:0 0 .3em;letter-spacing:-.01em}
h3{font-size:1rem;margin:0 0 .2em}
.lede{font-size:1.2rem;margin:0 0 1em}
.meta,.note,.legend{color:var(--dim);font-size:.9rem}
.meta{font-family:ui-monospace,monospace;font-size:.8rem;border-top:1px solid var(--line);padding-top:12px}
section{margin:64px auto;max-width:860px;padding:0 24px}
.note{margin:0 0 20px;max-width:62ch}
.figure{display:flex;justify-content:center}
svg{width:100%;max-width:640px;height:auto}
.ring{fill:none;stroke:var(--line)}
.ringlabel,.herelabel{fill:var(--dim);font:11px ui-monospace,monospace;text-anchor:middle}
.here{fill:var(--fg)}
.dot circle{fill:var(--rot)}
.dot.public circle{fill:var(--pub)}
.legend{margin-top:8px;text-align:center}
.swatch{display:inline-block;width:10px;height:10px;border-radius:50%;background:var(--rot);vertical-align:middle}
.swatch.public{background:var(--pub)}
.tl{border-top:1px solid var(--line)}
.tlrow{display:grid;grid-template-columns:minmax(120px,34%) 1fr;gap:12px;align-items:center;padding:6px 0;border-bottom:1px solid var(--line)}
.tlname{font-size:.85rem;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.tlbar{position:relative;height:12px;background:color-mix(in srgb,var(--line) 60%,transparent);border-radius:2px}
.seg{position:absolute;top:0;height:12px;background:var(--rot);border-radius:2px;min-width:2px}
.seg.chained{background:var(--acc)}
.pill{font:10px/1.6 ui-monospace,monospace;text-transform:uppercase;letter-spacing:.06em;color:var(--dim);border:1px solid var(--line);border-radius:99px;padding:1px 7px}
.pill.on{color:var(--bg);background:var(--pub);border-color:var(--pub)}
.cards{display:grid;gap:16px;grid-template-columns:repeat(auto-fill,minmax(260px,1fr))}
.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:16px}
.addr{font-family:ui-monospace,monospace;font-size:.78rem;color:var(--dim);margin:.2em 0}
.dist{font-size:.85rem;color:var(--dim);margin:.2em 0 .6em}
.card ul{margin:0;padding-left:1.1em;font-size:.88rem}
.card li{margin:.25em 0}
.none{font-size:.88rem;color:var(--dim);margin:0}
.limits dt{font-weight:600;margin-top:1.1em}
.limits dd{margin:.2em 0 0;color:var(--dim)}
footer{max-width:860px;margin:0 auto;padding:40px 24px 80px;color:var(--dim);font-size:.8rem;font-family:ui-monospace,monospace;border-top:1px solid var(--line)}
</style>
<main>
"#;
