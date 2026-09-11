//! Wi-Fi and MQTT transport. Thin adapters over esp-idf-svc: all protocol
//! decisions live in `session.rs`.
#![cfg(feature = "hardware")]

use std::sync::mpsc::Sender;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::mqtt::client::{EspMqttClient, EventPayload, MqttClientConfiguration, QoS};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::EspError;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};
use log::{info, warn};


/// Joins the configured network, retrying forever — the satellite is a
/// headless appliance and must eventually come up when the AP does.
pub fn connect_wifi(
    modem: Modem,
    sysloop: EspSystemEventLoop,
    ssid: &str,
    pass: &str,
) -> Result<BlockingWifi<EspWifi<'static>>, EspError> {
    let nvs = EspDefaultNvsPartition::take()?;
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(modem, sysloop.clone(), Some(nvs))?,
        sysloop,
    )?;
    let auth_method = if pass.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: ssid.try_into().map_err(|()| EspError::from_infallible::<
            { esp_idf_svc::sys::ESP_ERR_INVALID_ARG },
        >())?,
        password: pass.try_into().map_err(|()| EspError::from_infallible::<
            { esp_idf_svc::sys::ESP_ERR_INVALID_ARG },
        >())?,
        auth_method,
        ..Default::default()
    }))?;
    wifi.start()?;
    loop {
        match wifi.connect().and_then(|()| wifi.wait_netif_up()) {
            Ok(()) => break,
            Err(e) => {
                warn!("wifi join failed ({e}); retrying in 5 s");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
    info!("wifi up: {:?}", wifi.wifi().sta_netif().get_ip_info()?);
    Ok(wifi)
}

/// Longest plausible MQTT topic filter. Mirrors `config::MAX_LEN`: a
/// length past this is not a topic, it is memory read as one.
const MAX_TOPIC_LEN: usize = 512;

/// Everything `Mqtt::connect` needs, in one struct passed by reference.
///
/// This is not a tidiness choice — it is load-bearing. Passed as separate
/// parameters, this call needs 8 machine words (indirect return, two
/// `&str`, the slice's pointer *and* length, the Sender). The Xtensa
/// windowed ABI hands over only six in `a2`–`a7`, so the rest travel in
/// the caller's frame at `[caller_sp + 0…]`.
///
/// Those stack-passed words do not survive the call. Measured on an
/// ESP32-S3 (esp-idf v5.3.3, Rust esp toolchain, `opt-level = "s"`): the
/// caller logs `filters.len() == 5`, and the first statement of the
/// callee logs `2151180676` — `0x80386BC4`, whose top bits are the
/// windowed ABI's `call8` window-increment marker, i.e. a saved return
/// address. `subscribe` then iterated 2.1 billion "topics" across the
/// stack until one asked `CString::new` for a gigabyte and aborted the
/// firmware. The register-passed arguments were always intact, which is
/// why the broker connection itself succeeded every time.
///
/// In the disassembly the caller writes those words to its outgoing-
/// argument area at `[caller_sp + 0…8]`, but the callee — which opens
/// with `entry a1, 0x2a0` and then realigns `a1` upward by 1–64 bytes
/// for an over-aligned local — never reads that area at all: its highest
/// `a1` offset is 100, a frame slot holding spilled state. The exact
/// mislowering is not pinned down further here; what is established is
/// the boundary, and that removing the stack-passed words removes the
/// failure.
///
/// One reference is one word, so nothing is passed on the stack and the
/// fields are read through a pointer into the caller's frame. Keep it
/// that way: adding parameters back to `connect` reopens the bug.
pub struct Setup<'a> {
    pub url: &'a str,
    pub client_id: &'a str,
    /// Fixed-size on purpose: the length is then a compile-time constant
    /// rather than a value that has to survive the call.
    pub filters: &'a [String; 5],
    pub tx: Sender<(String, Vec<u8>)>,
}

pub struct Mqtt {
    client: EspMqttClient<'static>,
}

impl Mqtt {
    /// Connects to the broker and forwards every inbound message into
    /// `tx` as `(topic, payload)`. (Re-)subscribes to the five session
    /// response filters on every `Connected` event, so ESP-IDF's
    /// automatic reconnects re-establish the subscriptions too.
    /// Takes its arguments as one `Setup` reference rather than as
    /// separate parameters — see that type for why the distinction
    /// decides whether this function works at all.
    pub fn connect(setup: Setup<'_>) -> Result<Self, EspError> {
        let Setup { url, client_id, filters, tx } = setup;
        let conf = MqttClientConfiguration {
            client_id: Some(client_id),
            keep_alive_interval: Some(Duration::from_secs(15)),
            ..Default::default()
        };
        // Subscribing from inside the event callback would deadlock the
        // MQTT task, so the callback only signals; a helper thread owns
        // the (re-)subscribe calls.
        let (conn_tx, conn_rx) = std::sync::mpsc::channel::<()>();
        let client = EspMqttClient::new_cb(url, &conf, move |event| match event.payload() {
            EventPayload::Received { topic, data, .. } => {
                if let Some(topic) = topic {
                    let _ = tx.send((topic.to_string(), data.to_vec()));
                }
            }
            EventPayload::Connected(_) => {
                info!("mqtt connected");
                let _ = conn_tx.send(());
            }
            EventPayload::Disconnected => warn!("mqtt disconnected; esp-idf will reconnect"),
            _ => {}
        })?;
        let mut mqtt = Self { client };
        // First Connected may already be queued; the subscriber thread
        // needs its own client handle, but EspMqttClient isn't Clone —
        // so block here for the first connect, then let the callback's
        // signals drive resubscription synchronously via enqueue.
        conn_rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| EspError::from_infallible::<{ esp_idf_svc::sys::ESP_ERR_TIMEOUT }>())?;
        for f in filters {
            // `subscribe` turns the topic into a CString, so a wrong
            // length here asks the allocator for that many bytes and
            // aborts the firmware. Cheap to check, and it fails with the
            // topic named instead of as an out-of-memory panic.
            if f.is_empty() || f.len() > MAX_TOPIC_LEN {
                warn!("refusing implausible topic filter (len {})", f.len());
                return Err(EspError::from_infallible::<
                    { esp_idf_svc::sys::ESP_ERR_INVALID_SIZE },
                >());
            }
            mqtt.client.subscribe(f, QoS::AtLeastOnce)?;
        }
        // Keep re-subscribing after later reconnects.
        std::thread::Builder::new()
            .name("mqtt-resub".into())
            .stack_size(4096)
            .spawn({
                let c_filters: Vec<std::ffi::CString> = filters
                    .iter()
                    .map(|f| std::ffi::CString::new(f.as_str()).expect("no NUL in topic"))
                    .collect();
                let raw = mqtt.client.handle() as usize;
                move || {
                    while conn_rx.recv().is_ok() {
                        for f in &c_filters {
                            // Safety: the raw handle outlives the process;
                            // esp_mqtt_client_subscribe is thread-safe.
                            unsafe {
                                esp_idf_svc::sys::esp_mqtt_client_subscribe_single(
                                    raw as _,
                                    f.as_ptr(),
                                    1,
                                );
                            }
                        }
                    }
                }
            })
            .map_err(|_| EspError::from_infallible::<{ esp_idf_svc::sys::ESP_FAIL }>())?;
        Ok(mqtt)
    }

    /// QoS 0, non-retained: audio frames tolerate loss and must not queue.
    pub fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), EspError> {
        self.client.publish(topic, QoS::AtMostOnce, false, payload)?;
        Ok(())
    }
}
