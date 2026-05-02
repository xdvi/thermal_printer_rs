// ============================================================
// ThermalPrinterRsPlugin.swift — iOS native bridge
//
// Provides TCP (via Rust/FRB) and BLE (via CoreBluetooth) transports.
//
// TRANSPORT AVAILABILITY ON iOS:
//   TCP          — Rust/FRB handles this natively. No code needed here.
//   BLE          — CoreBluetooth (this file). Works for BLE-capable printers.
//   USB          — BLOCKED by Apple. Requires MFi certification.
//   BT Classic   — BLOCKED by Apple. SPP/RFCOMM requires MFi certification.
//
// IMPORTANT: Add NSBluetoothAlwaysUsageDescription to Info.plist.
// ============================================================

import Flutter
import UIKit
import CoreBluetooth

// MARK: - Plugin entry point

public class ThermalPrinterRsPlugin: NSObject, FlutterPlugin {

    public static func register(with registrar: FlutterPluginRegistrar) {
        let channel = FlutterMethodChannel(
            name: "thermal_printer_rs/ios",
            binaryMessenger: registrar.messenger()
        )
        let instance = ThermalPrinterRsPlugin()
        registrar.addMethodCallDelegate(instance, channel: channel)
    }

    // Single BLE manager instance — must be kept alive
    private let bleTransport = IosBleTransport()

    public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
        let args = call.arguments as? [String: Any]

        switch call.method {
        case "ble_scan":
            let timeout = args?["timeoutMs"] as? Int ?? 8000
            bleTransport.scan(timeoutMs: timeout, result: result)

        case "ble_connect":
            guard let uuid = args?["uuid"] as? String else {
                result(FlutterError(code: "INVALID_ARG", message: "uuid required", details: nil))
                return
            }
            let serviceUuid    = args?["serviceUuid"]        as? String
            let characteristicUuid = args?["characteristicUuid"] as? String
            bleTransport.connect(
                peripheralUUID:     uuid,
                serviceUUID:        serviceUuid,
                characteristicUUID: characteristicUuid,
                result:             result
            )

        case "ble_write":
            guard let data = args?["data"] as? FlutterStandardTypedData else {
                result(FlutterError(code: "INVALID_ARG", message: "data required", details: nil))
                return
            }
            bleTransport.write(data: data.data, result: result)

        case "ble_disconnect":
            bleTransport.disconnect(result: result)

        case "ble_state":
            result(bleTransport.stateString)

        default:
            result(FlutterMethodNotImplemented)
        }
    }
}

// MARK: - IosBleTransport

/// CoreBluetooth-based BLE transport for iOS.
///
/// Lifecycle:
///   1. scan()      — discovers nearby BLE peripherals
///   2. connect()   — connects by NSUUID and discovers services/characteristics
///   3. write()     — sends ESC/POS chunks respecting MTU
///   4. disconnect() — cancels the peripheral connection
///
/// All operations dispatch their result back to the main thread.
private class IosBleTransport: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {

    // Default UUIDs for common BLE thermal printers (Peripage, Goojprt, etc.)
    private static let defaultServiceUUID        = CBUUID(string: "000018f0-0000-1000-8000-00805f9b34fb")
    private static let defaultCharacteristicUUID = CBUUID(string: "00002af1-0000-1000-8000-00805f9b34fb")

    // CoreBluetooth chunk size — conservative default, will use actual MTU
    private static let defaultChunkSize = 182

    // ── State ──────────────────────────────────────────────────────

    private var central:        CBCentralManager!
    private var peripheral:     CBPeripheral?
    private var writeChar:      CBCharacteristic?

    private var scannedDevices:    [String: CBPeripheral] = [:]  // uuid -> peripheral
    private var pendingScanResult: FlutterResult?
    private var pendingConnResult: FlutterResult?
    private var pendingWriteResult: FlutterResult?
    private var pendingWriteData:   Data?

    private var targetServiceUUID:        CBUUID?
    private var targetCharacteristicUUID: CBUUID?

    private var scanTimer: Timer?

    var stateString: String {
        switch central?.state {
        case .poweredOn:   return "poweredOn"
        case .poweredOff:  return "poweredOff"
        case .unsupported: return "unsupported"
        case .unauthorized: return "unauthorized"
        case .resetting:   return "resetting"
        default:           return "unknown"
        }
    }

    override init() {
        super.init()
        central = CBCentralManager(delegate: self, queue: nil)
    }

    // ── Public API ─────────────────────────────────────────────────

    /// Scans for nearby BLE peripherals for [timeoutMs] milliseconds.
    /// Returns a list of {name, uuid} maps to Flutter.
    func scan(timeoutMs: Int, result: @escaping FlutterResult) {
        guard central.state == .poweredOn else {
            result(FlutterError(
                code:    "BT_UNAVAILABLE",
                message: "Bluetooth is not powered on. Current state: \(stateString)",
                details: nil
            ))
            return
        }

        scannedDevices.removeAll()
        pendingScanResult = result

        central.scanForPeripherals(withServices: nil, options: [
            CBCentralManagerScanOptionAllowDuplicatesKey: false
        ])

        scanTimer?.invalidate()
        scanTimer = Timer.scheduledTimer(withTimeInterval: Double(timeoutMs) / 1000.0, repeats: false) { [weak self] _ in
            self?.finishScan()
        }
    }

    /// Connects to a peripheral by its NSUUID string.
    /// Optionally customizes which service/characteristic to use.
    func connect(
        peripheralUUID:     String,
        serviceUUID:        String?,
        characteristicUUID: String?,
        result: @escaping FlutterResult
    ) {
        // Resolve the peripheral — may be from scan or known UUID
        let peripheral: CBPeripheral?

        if let known = scannedDevices[peripheralUUID.uppercased()] {
            peripheral = known
        } else if let uuid = UUID(uuidString: peripheralUUID) {
            // Try to retrieve from system if already bonded
            let retrieved = central.retrievePeripherals(withIdentifiers: [uuid])
            peripheral = retrieved.first
        } else {
            peripheral = nil
        }

        guard let p = peripheral else {
            result(FlutterError(
                code:    "NOT_FOUND",
                message: "Peripheral \(peripheralUUID) not found. Run ble_scan first.",
                details: nil
            ))
            return
        }

        // Disconnect any existing connection first
        if let current = self.peripheral {
            central.cancelPeripheralConnection(current)
        }

        targetServiceUUID        = serviceUUID.map { CBUUID(string: $0) } ?? Self.defaultServiceUUID
        targetCharacteristicUUID = characteristicUUID.map { CBUUID(string: $0) } ?? Self.defaultCharacteristicUUID
        pendingConnResult        = result

        self.peripheral     = p
        p.delegate          = self
        central.connect(p, options: nil)
    }

    /// Writes raw ESC/POS bytes to the connected peripheral in MTU-safe chunks.
    func write(data: Data, result: @escaping FlutterResult) {
        guard let p = peripheral, let char = writeChar else {
            result(FlutterError(
                code:    "NOT_CONNECTED",
                message: "No BLE printer connected. Call ble_connect first.",
                details: nil
            ))
            return
        }

        let writeType: CBCharacteristicWriteType = char.properties.contains(.writeWithoutResponse) 
            ? .withoutResponse 
            : .withResponse

        // Negotiate actual MTU (iOS 9+)
        let chunkSize = p.maximumWriteValueLength(for: writeType)
        var offset = 0

        while offset < data.count {
            let end   = min(offset + chunkSize, data.count)
            let chunk = data[offset..<end]
            p.writeValue(chunk, for: char, type: writeType)
            offset = end

            // Delay between chunks depending on response type
            let delay: TimeInterval = writeType == .withoutResponse ? 0.005 : 0.001
            Thread.sleep(forTimeInterval: delay)
        }

        result(true)
    }

    /// Disconnects the peripheral and clears state.
    func disconnect(result: @escaping FlutterResult) {
        if let p = peripheral {
            central.cancelPeripheralConnection(p)
        }
        peripheral  = nil
        writeChar   = nil
        result(true)
    }

    // ── Private helpers ────────────────────────────────────────────

    private func finishScan() {
        central.stopScan()
        scanTimer?.invalidate()
        scanTimer = nil

        guard let result = pendingScanResult else { return }
        pendingScanResult = nil

        let devices: [[String: String]] = scannedDevices.values.map { p in
            [
                "uuid": p.identifier.uuidString,
                "name": p.name ?? "Unknown"
            ]
        }
        DispatchQueue.main.async { result(devices) }
    }

    // ── CBCentralManagerDelegate ────────────────────────────────────

    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        // Nothing required here — state is checked lazily before each operation
    }

    func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String: Any],
        rssi RSSI: NSNumber
    ) {
        scannedDevices[peripheral.identifier.uuidString.uppercased()] = peripheral
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        // Discover only the target service to speed up discovery
        peripheral.discoverServices(targetServiceUUID.map { [$0] })
    }

    func centralManager(
        _ central: CBCentralManager,
        didFailToConnect peripheral: CBPeripheral,
        error: Error?
    ) {
        guard let result = pendingConnResult else { return }
        pendingConnResult = nil
        DispatchQueue.main.async {
            result(FlutterError(
                code:    "CONNECT_FAILED",
                message: error?.localizedDescription ?? "Unknown connection error",
                details: nil
            ))
        }
    }

    func centralManager(
        _ central: CBCentralManager,
        didDisconnectPeripheral peripheral: CBPeripheral,
        error: Error?
    ) {
        self.peripheral = nil
        self.writeChar  = nil
    }

    // ── CBPeripheralDelegate ───────────────────────────────────────

    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        if let error = error {
            guard let result = pendingConnResult else { return }
            pendingConnResult = nil
            DispatchQueue.main.async {
                result(FlutterError(
                    code:    "SERVICE_DISCOVERY_FAILED",
                    message: error.localizedDescription,
                    details: nil
                ))
            }
            return
        }

        guard let service = peripheral.services?.first(where: {
            $0.uuid == targetServiceUUID
        }) else {
            // Service not found — try discovering all characteristics in the first available service
            if let firstService = peripheral.services?.first {
                peripheral.discoverCharacteristics(nil, for: firstService)
            } else {
                failConnect(message: "Target service \(targetServiceUUID?.uuidString ?? "?") not found on peripheral")
            }
            return
        }

        peripheral.discoverCharacteristics([targetCharacteristicUUID!], for: service)
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didDiscoverCharacteristicsFor service: CBService,
        error: Error?
    ) {
        if let error = error {
            failConnect(message: error.localizedDescription)
            return
        }

        // Find the write characteristic
        let writeCharacteristic = service.characteristics?.first { char in
            char.uuid == targetCharacteristicUUID ||
            char.properties.contains(.writeWithoutResponse) ||
            char.properties.contains(.write)
        }

        guard let char = writeCharacteristic else {
            failConnect(message: "No writable characteristic found. Check UUIDs for this printer model.")
            return
        }

        self.writeChar = char

        guard let result = pendingConnResult else { return }
        pendingConnResult = nil
        DispatchQueue.main.async { result(true) }
    }

    private func failConnect(message: String) {
        guard let result = pendingConnResult else { return }
        pendingConnResult = nil
        if let p = peripheral { central.cancelPeripheralConnection(p) }
        peripheral = nil
        DispatchQueue.main.async {
            result(FlutterError(code: "CONNECT_FAILED", message: message, details: nil))
        }
    }
}
