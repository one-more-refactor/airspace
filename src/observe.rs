//! The listening half.
//!
//! No monitor mode, no root, no injection, nothing that touches anybody else's
//! radio: bluez is already scanning for its own reasons, and every device in
//! range is already shouting. This just writes down what arrives.
//!
//! Which is the uncomfortable point of the whole tool. Nothing here is an
//! attack. It is an unprivileged user account reading a bus that any desktop
//! session can read, and the result is a log of who was in the building.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use zbus::zvariant::OwnedValue;
use zbus::{proxy, Connection};

use crate::model::Observation;

#[proxy(interface = "org.bluez.Adapter1", default_service = "org.bluez")]
trait Adapter {
    fn start_discovery(&self) -> zbus::Result<()>;
    fn set_discovery_filter(&self, filter: HashMap<String, OwnedValue>) -> zbus::Result<()>;
    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn discovering(&self) -> zbus::Result<bool>;
}

pub struct Listener {
    conn: Connection,
    adapter: AdapterProxy<'static>,
}

impl Listener {
    pub async fn new() -> Result<Listener> {
        let conn = Connection::system().await?;
        let adapter = AdapterProxy::builder(&conn).path("/org/bluez/hci0")?.build().await?;
        if !adapter.powered().await.unwrap_or(false) {
            anyhow::bail!("the bluetooth adapter is off");
        }
        Ok(Listener { conn, adapter })
    }

    /// Ask for an unfiltered scan that does NOT coalesce repeat advertisements.
    /// Without `DuplicateData` bluez reports a device once and then goes quiet
    /// about it, which turns a presence log into a single dot.
    pub async fn start(&self) -> Result<()> {
        let mut filter: HashMap<String, OwnedValue> = HashMap::new();
        filter.insert("Transport".into(), OwnedValue::try_from(zbus::zvariant::Value::from("auto"))?);
        filter.insert("DuplicateData".into(), OwnedValue::from(true));
        // RSSI is the whole point, so ask for everything audible at all.
        filter.insert("RSSI".into(), OwnedValue::from(-127i16));
        let _ = self.adapter.set_discovery_filter(filter).await;

        if !self.adapter.discovering().await.unwrap_or(false) {
            // Already-discovering is not an error worth stopping for: another
            // client (a desktop's bluetooth panel) may hold it.
            let _ = self.adapter.start_discovery().await;
        }
        Ok(())
    }

    /// Everything bluez currently knows about, as observations.
    pub async fn sweep(&self) -> Result<Vec<Observation>> {
        let om = zbus::fdo::ObjectManagerProxy::builder(&self.conn)
            .destination("org.bluez")?
            .path("/")?
            .build()
            .await?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let mut out = Vec::new();

        for (_path, ifaces) in om.get_managed_objects().await? {
            let Some(dev) = ifaces.get("org.bluez.Device1") else { continue };
            let get = |k: &str| dev.get(k);
            let addr = get("Address")
                .and_then(|v| <&str>::try_from(v).ok())
                .unwrap_or_default()
                .to_string();
            if addr.is_empty() {
                continue;
            }

            let mut company = Vec::new();
            let mut cmsg = Vec::new();
            if let Some(md) = get("ManufacturerData") {
                if let Ok(map) = <HashMap<u16, OwnedValue>>::try_from(md.clone()) {
                    for (id, val) in map {
                        company.push(id);
                        if let Ok(bytes) = <Vec<u8>>::try_from(val) {
                            if let Some(first) = bytes.first() {
                                cmsg.push((id, *first));
                            }
                        }
                    }
                }
            }

            let service = get("ServiceData")
                .and_then(|v| <HashMap<String, OwnedValue>>::try_from(v.clone()).ok())
                .map(|m| m.into_keys().collect::<Vec<_>>())
                .unwrap_or_default();

            out.push(Observation {
                t: now,
                addr,
                at: get("AddressType")
                    .and_then(|v| <&str>::try_from(v).ok())
                    .unwrap_or("unknown")
                    .to_string(),
                rssi: get("RSSI").and_then(|v| i16::try_from(v.clone()).ok()),
                name: get("Name")
                    .and_then(|v| <&str>::try_from(v).ok())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                company,
                cmsg,
                service,
                paired: get("Paired").and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false),
            });
        }
        Ok(out)
    }

    /// Keep discovery alive. It stops on its own across a suspend/resume, and
    /// a listener that has silently gone deaf still writes a file, which is the
    /// worst possible failure for a tool whose output is "nothing was there".
    pub async fn keep_alive(&self) {
        if !self.adapter.discovering().await.unwrap_or(false) {
            let _ = self.adapter.start_discovery().await;
        }
    }
}

pub const SWEEP: Duration = Duration::from_secs(2);
