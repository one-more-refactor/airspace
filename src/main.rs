//! earshot — a passive radio observatory.
//!
//! Listens to what the devices around you broadcast on their own, without
//! being asked, and renders it as a page you can show somebody.
//!
//! It transmits nothing, needs no root, and uses no monitor mode. That is the
//! entire argument: if this is what an ordinary user account can see with no
//! effort and no special hardware, then it is the floor, not the ceiling.

mod analyse;
mod model;
mod observe;
mod report;

use std::path::PathBuf;

use anyhow::Result;
use tokio::io::AsyncWriteExt;

const USAGE: &str = "\
earshot — what the devices near you say out loud

USAGE:
    earshot watch [SECONDS]      listen and append to the capture (default: until Ctrl-C)
    earshot report [OUT.html]    render the capture as a page (default: earshot.html)
    earshot doctor               what this machine's radios can and cannot hear

    --capture PATH               where observations live
                                 (default: ~/.local/share/earshot/observations.jsonl)
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
        "watch" => {
            let secs = positional.first().and_then(|s| s.parse::<u64>().ok());
            watch(&capture, secs).await
        }
        "report" => {
            let out = positional
                .first()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("earshot.html"));
            render(&capture, &out)
        }
        "doctor" => doctor().await,
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
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
    base.join("earshot/observations.jsonl")
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
        .map_err(|e| anyhow::anyhow!("{}: {e} — run `earshot watch` first", capture.display()))?;
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
            println!("    → `earshot watch` works on this machine right now");
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
