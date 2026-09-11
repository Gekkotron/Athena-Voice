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

pub struct Mqtt {
    client: EspMqttClient<'static>,
}

impl Mqtt {
    /// Connects to the broker and forwards every inbound message into
    /// `tx` as `(topic, payload)`. (Re-)subscribes to the five session
    /// response filters on every `Connected` event, so ESP-IDF's
    /// automatic reconnects re-establish the subscriptions too.
    /// `filters` and `client_id` are built by the caller at boot, before
    /// any radio or peripheral is initialised: this device corrupts those
    /// values somewhere during Wi-Fi bring-up, and strings built while
    /// they were known-good cannot be re-read wrong here.
    pub fn connect(
        url: &str,
        client_id: &str,
        filters: &[String],
        tx: Sender<(String, Vec<u8>)>,
    ) -> Result<Self, EspError> {
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
