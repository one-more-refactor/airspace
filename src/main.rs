//! airspace — a passive radio observatory.
//!
//! Listens to what the devices around you broadcast on their own, without
//! being asked, and renders it as a page you can show somebody.
//!
//! It transmits nothing, needs no root, and uses no monitor mode. That is the
//! entire argument: if this is what an ordinary user account can see with no
//! effort and no special hardware, then it is the floor, not the ceiling.

mod analyse;
mod identity;
mod model;
mod observe;
mod report;
mod serve;
mod space;
mod wifi;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::io::AsyncWriteExt;

const USAGE: &str = "\
airspace — what the devices near you say out loud

USAGE:
    airspace serve [BIND]        listen and serve the live picture (default: 127.0.0.1:9970)
    airspace feed [URL]          listen here, report to a collector — this is how
                                 direction becomes possible: one ear hears a distance,
                                 three ears at known positions hear a place
    airspace wifi IFACE          listen on a monitor interface too, feeding the
                                 same map. Needs CAP_NET_RAW — see the README.
    airspace watch [SECONDS]     listen and append to the capture (default: until Ctrl-C)
    airspace report [OUT.html]   render the capture as a page (default: airspace.html)
    airspace doctor              what this machine\'s radios can and cannot hear
    airspace whoami [SECONDS]    watch for addresses that resolve to a configured
                                 identity — the check that a key actually works

    Room size and this node\'s position live in ~/.config/airspace/config.toml.

    --capture PATH               where observations live
                                 (default: ~/.local/share/airspace/observations.jsonl)
";

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let capture = capture_path(&args);
    let positional: Vec<&String> = {
        let mut v = Vec::new();
        let mut skip = false;
        for a in args.iter().skip(1) {
            if skip {
                skip = false;
                continue;
            }
            if a == "--capture" {
                skip = true;
                continue;
            }
            v.push(a);
        }
        v
    };

    match args.first().map(String::as_str).unwrap_or("help") {
        "serve" => {
            let cfg = space::Config::load()?;
            let bind = positional
                .first()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "127.0.0.1:9970".to_string());
            serve::serve(space::State::new(cfg), &bind).await
        }
        "feed" => {
            let cfg = space::Config::load()?;
            let url = positional
                .first()
                .map(|s| s.to_string())
                .unwrap_or_else(|| cfg.collector.url.clone());
            if url.is_empty() {
                anyhow::bail!("no collector: pass a URL or set collector.url in {}",
                    space::Config::path().display());
            }
            let token = cfg.collector.token.clone();
            serve::feed(space::State::new(cfg), &url, &token).await
        }
        "wifi" => {
            let cfg = space::Config::load()?;
            let iface = positional.first().map(|s| s.to_string()).unwrap_or_else(|| "mon0".into());
            let url = cfg.collector.url.clone();
            let token = cfg.collector.token.clone();
            init_wifi(cfg, &iface, &url, &token).await
        }
        "watch" => {
            let secs = positional.first().and_then(|s| s.parse::<u64>().ok());
            watch(&capture, secs).await
        }
        "report" => {
            let out = positional
                .first()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("airspace.html"));
            render(&capture, &out)
        }
        "whoami" => {
            let secs = positional.first().and_then(|s| s.parse::<u64>().ok()).unwrap_or(60);
            whoami(secs).await
        }
        "doctor" => doctor().await,
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
}

/// The Wi-Fi ear: a privileged listener that feeds the unprivileged collector.
///
/// It runs as its own process on purpose. Capture needs CAP_NET_RAW, and the
/// rest of airspace deliberately needs nothing at all. Folding the two together
/// would make the whole tool demand a privilege for the sake of one of its two
/// radios, and the Bluetooth half's whole argument is that it needs none.
async fn init_wifi(cfg: space::Config, iface: &str, url: &str, token: &str) -> Result<()> {
    let url = if url.is_empty() {
        "http://127.0.0.1:9970/ingest".to_string()
    } else {
        url.to_string()
    };
    let (host, path) = serve::split_url(&url)?;
    let node = cfg.node.clone();
    let mut sniffer = wifi::Sniffer::open(iface)?;
    eprintln!("wifi: listening on {iface}, feeding {host}{path} as node {:?}", node.name);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<model::Observation>(4096);
    std::thread::Builder::new()
        .name("wifi-sniff".into())
        .spawn(move || loop {
            match sniffer.recv() {
                Ok(Some(f)) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    if tx.blocking_send(f.observation(now)).is_err() {
                        return;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("wifi capture stopped: {e}");
                    return;
                }
            }
        })?;

    // A busy channel is thousands of frames a second, almost all of them the
    // same handful of devices saying nothing new. Collapse to one observation
    // per address per flush — but merge rather than overwrite, or a data frame
    // arriving after a beacon would erase the network name the beacon carried.
    let mut pending: HashMap<String, model::Observation> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            Some(o) = rx.recv() => {
                match pending.get_mut(&o.addr) {
                    Some(prev) => {
                        let keep_name = prev.name.clone();
                        let keep_doing = prev.doing.clone();
                        *prev = o;
                        if prev.name.is_none() { prev.name = keep_name; }
                        if prev.doing.is_none() { prev.doing = keep_doing; }
                    }
                    None => { pending.insert(o.addr.clone(), o); }
                }
            }
            _ = tick.tick() => {
                if pending.is_empty() { continue; }
                let obs: Vec<model::Observation> = pending.drain().map(|(_, v)| v).collect();
                let n = obs.len();
                let body = serde_json::to_vec(&serve::Batch { node: node.clone(), obs })?;
                if let Err(e) = serve::post(&host, &path, token, &body).await {
                    eprintln!("collector unreachable: {e}");
                } else {
                    eprint!("\r{n} wi-fi devices in the last sweep   ");
                }
            }
        }
    }
}

/// Watch for addresses that resolve to a configured identity.
///
/// This exists because an identity key that does not work fails silently: the
/// device simply never appears, which is indistinguishable from it being
/// switched off. A phone rotates its address every fifteen minutes or so, so
/// give this a minute and expect one or two hits, not a stream.
async fn whoami(secs: u64) -> Result<()> {
    let cfg = space::Config::load()?;
    if cfg.identities.is_empty() {
        anyhow::bail!(
            "no [[identity]] blocks in {} — add a name and the IRK from the device's bond file",
            space::Config::path().display()
        );
    }
    for id in &cfg.identities {
        match id.key() {
            Some(_) => eprintln!("watching for {:?}", id.name),
            None => eprintln!("{:?}: the irk is not 32 hex characters", id.name),
        }
    }

    let radio = observe::Listener::new().await?;
    radio.start().await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut candidates = 0usize;

    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(observe::SWEEP).await;
        let _ = radio.keep_alive().await;
        for o in radio.sweep().await.unwrap_or_default() {
            // Only resolvable private addresses are even eligible.
            if identity::parse_addr(&o.addr).is_some_and(|a| a[0] & 0xc0 == 0x40) {
                candidates += 1;
            }
            // Try the key as written AND byte-reversed. BlueZ's info file and
            // the specification's notation disagree about order, and which one
            // a given stack wrote is not knowable from the hex alone — so this
            // reports which orientation matched rather than guessing.
            if let Some(bytes) = identity::parse_addr(&o.addr) {
                for id in &cfg.identities {
                    let Some(k) = id.key() else { continue };
                    let mut rev = k;
                    rev.reverse();
                    let hit = if identity::resolves(&k, &bytes) {
                        Some("as-written")
                    } else if identity::resolves(&rev, &bytes) {
                        Some("byte-reversed")
                    } else {
                        None
                    };
                    if let Some(order) = hit {
                        if seen.insert(o.addr.clone(), id.name.clone()).is_none() {
                            println!("{}  ->  {}   [key {order}]   ({} dBm)",
                                     o.addr, id.name, o.rssi.unwrap_or(0));
                        }
                    }
                }
            }
        }
    }

    println!();
    if seen.is_empty() {
        println!(
            "No address resolved, out of {candidates} resolvable-private ones seen.\n\
             Either the device was not advertising, or the key is not the right key —\n\
             a bond that only produced a [LinkKey] and no [IdentityResolvingKey] gives\n\
             you a classic-bluetooth bond that cannot resolve anything."
        );
    } else {
        println!("{} address(es) resolved to {} identity(ies).", seen.len(), 
                 seen.values().collect::<std::collections::HashSet<_>>().len());
    }
    Ok(())
}

fn capture_path(args: &[String]) -> PathBuf {
    if let Some(i) = args.iter().position(|a| a == "--capture") {
        if let Some(p) = args.get(i + 1) {
            return PathBuf::from(p);
        }
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default())
                .join(".local/share")
        });
    base.join("airspace/observations.jsonl")
}

async fn watch(capture: &PathBuf, secs: Option<u64>) -> Result<()> {
    if let Some(dir) = capture.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = observe::Listener::new().await?;
    listener.start().await?;

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(capture)
        .await?;

    match secs {
        Some(s) => eprintln!("Listening for {s}s → {}", capture.display()),
        None => eprintln!("Listening until Ctrl-C → {}", capture.display()),
    }

    // Only write an address when something about it changed, or every 30s
    // regardless. A device sitting still would otherwise produce one line
    // every two seconds forever, and the capture stops being readable.
    let mut last: std::collections::HashMap<String, (u64, Option<i16>)> = Default::default();
    let deadline = secs.map(|s| tokio::time::Instant::now() + std::time::Duration::from_secs(s));
    let mut written = 0usize;

    loop {
        if let Some(d) = deadline {
            if tokio::time::Instant::now() >= d {
                break;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(observe::SWEEP) => {}
            _ = tokio::signal::ctrl_c() => break,
        }
        listener.keep_alive().await;

        for o in listener.sweep().await.unwrap_or_default() {
            let changed = match last.get(&o.addr) {
                Some((t, r)) => o.t.saturating_sub(*t) >= 30 || *r != o.rssi,
                None => true,
            };
            if !changed {
                continue;
            }
            last.insert(o.addr.clone(), (o.t, o.rssi));
            let mut line = serde_json::to_string(&o)?;
            line.push('\n');
            file.write_all(line.as_bytes()).await?;
            written += 1;
        }
        file.flush().await?;
        eprint!("\r{written} observations, {} addresses  ", last.len());
    }
    eprintln!("\nWrote {written} observations to {}", capture.display());
    Ok(())
}

fn render(capture: &PathBuf, out: &PathBuf) -> Result<()> {
    let text = std::fs::read_to_string(capture)
        .map_err(|e| anyhow::anyhow!("{}: {e} — run `airspace watch` first", capture.display()))?;
    let obs: Vec<model::Observation> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if obs.is_empty() {
        anyhow::bail!("{} has no usable observations", capture.display());
    }
    let a = analyse::analyse(&obs);
    std::fs::write(out, report::render(&a))?;
    println!(
        "{} addresses, ~{} devices, {} sightings → {}",
        a.tracks.len(),
        a.estimated_devices(),
        a.sightings,
        out.display()
    );
    Ok(())
}

/// What this machine can actually hear, said plainly, including the part where
/// the answer is "less than you hoped".
async fn doctor() -> Result<()> {
    println!("Bluetooth");
    match observe::Listener::new().await {
        Ok(l) => {
            let _ = l.start().await;
            println!("  ✓ adapter present and powered");
            println!("  ✓ advertising channels readable with no root and no monitor mode");
            println!("    → `airspace watch` works on this machine right now");
        }
        Err(e) => println!("  ✗ {e}"),
    }

    println!("\nWi-Fi");
    let mut wifi = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/sys/class/net") {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().join("wireless").exists() || e.path().join("phy80211").exists() {
                let driver = std::fs::read_link(e.path().join("device/driver"))
                    .ok()
                    .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "?".into());
                wifi.push((name, driver));
            }
        }
    }
    if wifi.is_empty() {
        println!("  ✗ no wireless interface found");
    }
    for (name, driver) in &wifi {
        println!("  · {name} ({driver})");
        // The honest check needs nl80211, which means `iw`. Saying "unknown"
        // is better than guessing from the driver name.
        match std::process::Command::new("iw").args(["phy"]).output() {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout);
                let monitor = s.contains("* monitor");
                println!(
                    "    {} monitor mode {}",
                    if monitor { "✓" } else { "✗" },
                    if monitor { "supported — probe requests are readable" } else { "not offered by this driver" }
                );
            }
            Err(_) => println!("    ? install `iw` to find out whether monitor mode is offered"),
        }
    }
    println!(
        "\n  Probe requests are the richer source: a phone not connected to anything\n  \
         asks after networks it has joined before, which is a list of places its\n  \
         owner has been. Reading them needs monitor mode. If the line above says\n  \
         no, a USB adapter on mt7921au / mt7612u is the usual fix."
    );
    Ok(())
}
