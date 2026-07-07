// ============================================================
// transport/ble.rs — BLE Transport (feature = "ble")
// ============================================================
//
// IMPORTANT WARNINGS:
//
//   btleplug covers BLE (Bluetooth Low Energy).
//   Economic thermal printers usually use BT Classic (SPP/RFCOMM),
//   NOT BLE. Verify with the manufacturer if your printer supports BLE.
//
//   For BT Classic on Android: use JNI + android.bluetooth.BluetoothSocket
//   For BT Classic on iOS:     not possible without MFi.
//
//   Service/Characteristic UUIDs vary by manufacturer.
//   Common examples for BLE printers:
//     Service:       000018f0-0000-1000-8000-00805f9b34fb  (Peripage, Goojprt)
//     Characteristic: 00002af1-0000-1000-8000-00805f9b34fb  (write)
//
//   Typical BLE MTU is 20 bytes (default) up to 512 bytes (negotiated).
//   Data MUST be sent in chunks <= negotiated MTU.

use async_trait::async_trait;
use btleplug::api::{Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Manager;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

use super::Transport;
use crate::errors::{PrinterError, Result};

// Default UUIDs (override when building if your printer differs)
const DEFAULT_PRINT_SERVICE_UUID: &str = "000018f0-0000-1000-8000-00805f9b34fb";
const DEFAULT_PRINT_CHAR_UUID: &str = "00002af1-0000-1000-8000-00805f9b34fb";

/// Default chunk size — conservative (ATT default MTU 23 minus 3 header = 20)
/// for maximum compatibility across BLE printers.
///
/// NOTE: btleplug 0.11 does NOT expose `request_mtu` or an MTU-changed event,
/// so the negotiated MTU cannot be queried here. Increasing this constant only
/// pays off on printers known to accept larger writes; do so per-deployment and
/// validate on real hardware (a too-large chunk silently drops data on the
/// `WithoutResponse` path).
const DEFAULT_CHUNK_SIZE: usize = 20;
const MAX_SCAN_MS: u64 = 5_000;

pub struct BleTransport {
    target_address: String,
    service_uuid: Uuid,
    char_uuid: Uuid,
    scan_timeout: Duration,
    chunk_size: usize,
    peripheral: Option<btleplug::platform::Peripheral>,
    /// Resolved write characteristic, cached once at connect time so each
    /// `write()` does not re-enumerate all characteristics.
    characteristic: Option<Characteristic>,
}

impl BleTransport {
    pub fn new(address: impl Into<String>, timeout_ms: u64) -> Self {
        Self {
            target_address: address.into(),
            service_uuid: Uuid::parse_str(DEFAULT_PRINT_SERVICE_UUID).unwrap(),
            char_uuid: Uuid::parse_str(DEFAULT_PRINT_CHAR_UUID).unwrap(),
            scan_timeout: Duration::from_millis(timeout_ms),
            chunk_size: DEFAULT_CHUNK_SIZE,
            peripheral: None,
            characteristic: None,
        }
    }

    async fn find_peripheral(
        central: &btleplug::platform::Adapter,
        target: &str,
    ) -> Result<Option<btleplug::platform::Peripheral>> {
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| PrinterError::ConnectionFailed(format!("Peripherals error: {e}")))?;

        Ok(peripherals
            .into_iter()
            .find(|p| p.id().to_string().to_uppercase().contains(target)))
    }

    pub fn with_uuids(mut self, service: &str, characteristic: &str) -> Result<Self> {
        self.service_uuid = Uuid::parse_str(service)
            .map_err(|e| PrinterError::InvalidConfig(format!("Invalid service UUID: {e}")))?;
        self.char_uuid = Uuid::parse_str(characteristic).map_err(|e| {
            PrinterError::InvalidConfig(format!("Invalid characteristic UUID: {e}"))
        })?;
        Ok(self)
    }
}

#[async_trait]
impl Transport for BleTransport {
    async fn connect(&mut self) -> Result<()> {
        let manager = Manager::new()
            .await
            .map_err(|e| PrinterError::ConnectionFailed(format!("BLE manager error: {e}")))?;

        let adapters = manager
            .adapters()
            .await
            .map_err(|e| PrinterError::ConnectionFailed(format!("BLE adapters error: {e}")))?;

        let central = adapters.into_iter().next().ok_or_else(|| {
            PrinterError::TransportUnavailable("No Bluetooth adapter found. Is BT enabled?".into())
        })?;

        let target = self.target_address.to_uppercase();
        let mut peripheral = Self::find_peripheral(&central, &target).await?;

        if peripheral.is_none() {
            let scan_ms = self.scan_timeout.as_millis().min(MAX_SCAN_MS as u128) as u64;
            info!(scan_ms, "BLE device not cached — scanning...");
            central
                .start_scan(ScanFilter::default())
                .await
                .map_err(|e| PrinterError::ConnectionFailed(format!("Scan error: {e}")))?;

            // Poll for the device instead of sleeping the full window: returns
            // as soon as the peripheral shows up, bounded by scan_ms.
            let scan_budget = Duration::from_millis(scan_ms);
            let poll_interval = Duration::from_millis(250);
            let started = std::time::Instant::now();
            loop {
                if let Some(p) = Self::find_peripheral(&central, &target).await? {
                    peripheral = Some(p);
                    break;
                }
                if started.elapsed() >= scan_budget {
                    break;
                }
                tokio::time::sleep(poll_interval).await;
            }
            central.stop_scan().await.ok();
        }

        let peripheral = peripheral.ok_or_else(|| {
            PrinterError::PrinterNotFound(format!(
                "BLE: printer {} not found during scan",
                self.target_address
            ))
        })?;

        peripheral
            .connect()
            .await
            .map_err(|e| PrinterError::ConnectionFailed(format!("BLE connect error: {e}")))?;

        peripheral
            .discover_services()
            .await
            .map_err(|e| PrinterError::ConnectionFailed(format!("Service discovery error: {e}")))?;

        // Resolve and cache the write characteristic once — avoids re-enumerating
        // all characteristics on every 20-byte write.
        let characteristic = peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == self.char_uuid)
            .ok_or_else(|| {
                PrinterError::ConnectionFailed(format!(
                    "BLE: characteristic {} not found",
                    self.char_uuid
                ))
            })?;

        self.characteristic = Some(characteristic);
        self.peripheral = Some(peripheral);
        info!(address = %self.target_address, "BLE connected");
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        let peripheral = self.peripheral.as_ref().ok_or_else(|| {
            PrinterError::TransportUnavailable("BLE: writing without active connection".into())
        })?;
        let char = self.characteristic.as_ref().ok_or_else(|| {
            PrinterError::TransportUnavailable(
                "BLE: write characteristic not resolved (not connected?)".into(),
            )
        })?;

        peripheral
            .write(char, data, WriteType::WithoutResponse)
            .await
            .map_err(|e| PrinterError::ConnectionFailed(format!("BLE write failed: {e}")))?;

        Ok(())
    }

    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        warn!("BLE read not implemented — most BLE printers are write-only");
        Ok(0)
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.characteristic = None;
        if let Some(p) = self.peripheral.take() {
            p.disconnect().await.ok();
            info!(address = %self.target_address, "BLE disconnected");
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.peripheral.is_some()
    }

    async fn check_liveness(&mut self) -> bool {
        if let Some(p) = &self.peripheral {
            p.is_connected().await.unwrap_or(false)
        } else {
            false
        }
    }

    fn transport_name(&self) -> &'static str {
        "BleTransport"
    }

    fn preferred_chunk_size(&self) -> usize {
        self.chunk_size
    }

    fn chunk_delay(&self) -> Duration {
        Duration::from_millis(20)
    }
}
