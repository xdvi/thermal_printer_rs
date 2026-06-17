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
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Manager;
use std::time::Duration;
use uuid::Uuid;
use tracing::{info, warn};

use crate::errors::{PrinterError, Result};
use super::Transport;

// Default UUIDs (override when building if your printer differs)
const DEFAULT_PRINT_SERVICE_UUID: &str = "000018f0-0000-1000-8000-00805f9b34fb";
const DEFAULT_PRINT_CHAR_UUID:    &str = "00002af1-0000-1000-8000-00805f9b34fb";

/// Default chunk size — conservative for maximum compatibility.
/// Negotiate MTU with the printer to increase it.
const DEFAULT_CHUNK_SIZE: usize = 182;

pub struct BleTransport {
    target_address: String,
    service_uuid:   Uuid,
    char_uuid:      Uuid,
    scan_timeout:   Duration,
    chunk_size:     usize,
    peripheral:     Option<btleplug::platform::Peripheral>,
}

impl BleTransport {
    pub fn new(
        address: impl Into<String>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            target_address: address.into(),
            service_uuid:   Uuid::parse_str(DEFAULT_PRINT_SERVICE_UUID).unwrap(),
            char_uuid:      Uuid::parse_str(DEFAULT_PRINT_CHAR_UUID).unwrap(),
            scan_timeout:   Duration::from_millis(timeout_ms),
            chunk_size:     DEFAULT_CHUNK_SIZE,
            peripheral:     None,
        }
    }

    pub fn with_uuids(mut self, service: &str, characteristic: &str) -> Result<Self> {
        self.service_uuid = Uuid::parse_str(service)
            .map_err(|e| PrinterError::InvalidConfig(format!("Invalid service UUID: {e}")))?;
        self.char_uuid = Uuid::parse_str(characteristic)
            .map_err(|e| PrinterError::InvalidConfig(format!("Invalid characteristic UUID: {e}")))?;
        Ok(self)
    }
}

#[async_trait]
impl Transport for BleTransport {
    async fn connect(&mut self) -> Result<()> {
        let manager = Manager::new().await
            .map_err(|e| PrinterError::ConnectionFailed(format!("BLE manager error: {e}")))?;

        let adapters = manager.adapters().await
            .map_err(|e| PrinterError::ConnectionFailed(format!("BLE adapters error: {e}")))?;

        let central = adapters.into_iter().next()
            .ok_or_else(|| PrinterError::TransportUnavailable(
                "No Bluetooth adapter found. Is BT enabled?".into()
            ))?;

        // Scan for BLE devices
        info!("Scanning for BLE devices...");
        central.start_scan(ScanFilter::default()).await
            .map_err(|e| PrinterError::ConnectionFailed(format!("Scan error: {e}")))?;

        tokio::time::sleep(self.scan_timeout).await;
        central.stop_scan().await.ok();

        // Find printer by MAC address
        let peripherals = central.peripherals().await
            .map_err(|e| PrinterError::ConnectionFailed(format!("Peripherals error: {e}")))?;

        let target = self.target_address.to_uppercase();
        let peripheral = peripherals.into_iter().find(|p| {
            p.id().to_string().to.uppercase().contains(&target)
        }).ok_or_else(|| PrinterError::PrinterNotFound(
            format!("BLE: printer {} not found during scan", self.target_address)
        ))?;

        peripheral.connect().await
            .map_err(|e| PrinterError::ConnectionFailed(format!("BLE connect error: {e}")))?;

        peripheral.discover_services().await
            .map_err(|e| PrinterError::ConnectionFailed(format!("Service discovery error: {e}")))?;

        info!(address = %self.target_address, "BLE connected");
        self.peripheral = Some(peripheral);
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        let peripheral = self.peripheral.as_ref().ok_or_else(|| {
            PrinterError::TransportUnavailable("BLE: writing without active connection".into())
        })?;

        let characteristics = peripheral.characteristics();
        let char = characteristics.iter().find(|c| c.uuid == self.char_uuid)
            .ok_or_else(|| PrinterError::TransportUnavailable(
                format!("BLE: characteristic {} not found", self.char_uuid)
            ))?;

        peripheral.write(char, data, WriteType::WithoutResponse).await
            .map_err(|e| PrinterError::ConnectionFailed(format!("BLE write failed: {e}")))?;

        Ok(())
    }

    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        warn!("BLE read not implemented — most BLE printers are write-only");
        Ok(0)
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(p) = self.peripheral.take() {
            p.disconnect().await.ok();
            info!(address = %self.target_address, "BLE disconnected");
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.peripheral.is_some()
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
