package com.thermal.thermal_printer_rs

import android.app.PendingIntent
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.content.Intent
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbManager
import android.os.Build
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.MethodChannel.MethodCallHandler
import io.flutter.plugin.common.MethodChannel.Result
import kotlinx.coroutines.*
import java.io.IOException
import java.io.OutputStream
import java.util.UUID

/**
 * ThermalPrinterRsPlugin — Android native bridge.
 *
 * Exposes two transport mechanisms that cannot be implemented
 * in cross-platform Rust (libusb/btleplug do not work on Android):
 *
 *   1. USB printing via android.hardware.usb.UsbManager
 *   2. Bluetooth Classic (SPP) printing via android.bluetooth.BluetoothSocket
 *
 * All heavy I/O runs on Dispatchers.IO to avoid blocking the main thread.
 * Results are dispatched back to Flutter via MethodChannel.Result.
 */
class ThermalPrinterRsPlugin : FlutterPlugin, MethodCallHandler {

    private lateinit var channel: MethodChannel
    private lateinit var context: Context

    // ── Active connections (one at a time) ─────────────────────────

    private var usbConnection: UsbDeviceConnection? = null
    private var usbEndpointOut: android.hardware.usb.UsbEndpoint? = null
    private var usbEndpointIn: android.hardware.usb.UsbEndpoint? = null
    private var usbInterface: android.hardware.usb.UsbInterface? = null

    private var btSocket: BluetoothSocket? = null
    private var btOutputStream: OutputStream? = null
    private var btInputStream: java.io.InputStream? = null

    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    // SPP UUID — standard for serial port profile printers
    private val SPP_UUID: UUID = UUID.fromString("00001101-0000-1000-8000-00805F9B34FB")
    private val USB_TIMEOUT_MS = 3000

    // ── FlutterPlugin lifecycle ──────────────────────────────────

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        context = binding.applicationContext
        channel = MethodChannel(binding.binaryMessenger, "thermal_printer_rs/android")
        channel.setMethodCallHandler(this)
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel.setMethodCallHandler(null)
        scope.cancel()
        closeUsb()
        closeBluetooth()
    }

    // ── MethodChannel dispatcher ─────────────────────────────────

    override fun onMethodCall(call: MethodCall, result: Result) {
        when (call.method) {
            "usb_connect"    -> usbConnect(call, result)
            "usb_write"      -> usbWrite(call, result)
            "usb_read"       -> usbRead(call, result)
            "usb_disconnect" -> usbDisconnect(result)
            "bt_connect"     -> btConnect(call, result)
            "bt_write"       -> btWrite(call, result)
            "bt_read"        -> btRead(call, result)
            "bt_disconnect"  -> btDisconnect(result)
            "bt_list_paired" -> btListPaired(result)
            "bt_is_on"       -> btIsOn(result)
            "open_settings"  -> openSettings(result)
            "usb_list"       -> usbList(result)
            else             -> result.notImplemented()
        }
    }

    // ══════════════════════════════════════════════════════════════
    // USB
    // ══════════════════════════════════════════════════════════════

    /**
     * Connects to a USB printer by vendorId + productId.
     *
     * Dart call:
     * ```dart
     * channel.invokeMethod('usb_connect', {'vendorId': 1208, 'productId': 514});
     * ```
     */
    private fun usbConnect(call: MethodCall, result: Result) {
        val vendorId  = call.argument<Int>("vendorId")  ?: return result.error("INVALID_ARG", "vendorId required", null)
        val productId = call.argument<Int>("productId") ?: return result.error("INVALID_ARG", "productId required", null)

        scope.launch {
            try {
                val manager = context.getSystemService(Context.USB_SERVICE) as UsbManager
                val device = manager.deviceList.values.firstOrNull {
                    it.vendorId == vendorId && it.productId == productId
                } ?: run {
                    mainResult(result) { it.error("NOT_FOUND", "USB device $vendorId:$productId not found", null) }
                    return@launch
                }

                // Request permission if not already granted
                if (!manager.hasPermission(device)) {
                    val granted = requestUsbPermission(manager, device)
                    if (!granted) {
                        mainResult(result) { it.error("PERMISSION_DENIED", "USB permission denied by user", null) }
                        return@launch
                    }
                }

                openUsbDevice(manager, device)
                mainResult(result) { it.success(true) }

            } catch (e: Exception) {
                mainResult(result) { it.error("USB_ERROR", e.message, null) }
            }
        }
    }

    private fun openUsbDevice(manager: UsbManager, device: UsbDevice) {
        closeUsb()
        val iface = device.getInterface(0)
        
        // Find bulk-out and bulk-in endpoints
        var epOut: android.hardware.usb.UsbEndpoint? = null
        var epIn: android.hardware.usb.UsbEndpoint? = null
        
        for (i in 0 until iface.endpointCount) {
            val ep = iface.getEndpoint(i)
            if (ep.type == android.hardware.usb.UsbConstants.USB_ENDPOINT_XFER_BULK) {
                if (ep.direction == android.hardware.usb.UsbConstants.USB_DIR_OUT) {
                    epOut = ep
                } else if (ep.direction == android.hardware.usb.UsbConstants.USB_DIR_IN) {
                    epIn = ep
                }
            }
        }
        
        if (epOut == null) throw IOException("No bulk-out endpoint found on USB device")

        val connection = manager.openDevice(device)
            ?: throw IOException("Could not open USB device — check permissions")
        connection.claimInterface(iface, true)

        usbInterface   = iface
        usbEndpointOut = epOut
        usbEndpointIn  = epIn
        usbConnection  = connection
    }

    /**
     * Sends raw ESC/POS bytes to the connected USB printer.
     *
     * Dart call:
     * ```dart
     * channel.invokeMethod('usb_write', {'data': Uint8List(...)});
     * ```
     */
    private fun usbWrite(call: MethodCall, result: Result) {
        val data = call.argument<ByteArray>("data")
            ?: return result.error("INVALID_ARG", "data required", null)
        val conn     = usbConnection
        val endpoint = usbEndpointOut
        if (conn == null || endpoint == null) {
            return result.error("NOT_CONNECTED", "USB printer not connected", null)
        }

        scope.launch {
            try {
                val chunkSize = 4096
                var offset = 0
                while (offset < data.size) {
                    val end = minOf(offset + chunkSize, data.size)
                    // copyOfRange operates on the original ByteArray — no boxing to List<Byte>
                    val sent = conn.bulkTransfer(endpoint, data, offset, end - offset, USB_TIMEOUT_MS)
                    if (sent < 0) throw IOException("USB bulkTransfer returned $sent — possible disconnect")
                    offset = end
                }
                mainResult(result) { it.success(true) }
            } catch (e: Exception) {
                mainResult(result) { it.error("USB_WRITE_ERROR", e.message, null) }
            }
        }
    }

    /**
     * Reads raw bytes from the USB printer.
     */
    private fun usbRead(call: MethodCall, result: Result) {
        val bytesToRead = call.argument<Int>("bytes") ?: 1
        val timeoutMs = call.argument<Int>("timeoutMs") ?: USB_TIMEOUT_MS
        
        val conn = usbConnection
        val endpoint = usbEndpointIn
        
        if (conn == null) return result.error("NOT_CONNECTED", "USB printer not connected", null)
        if (endpoint == null) return result.error("NO_ENDPOINT", "Printer does not support USB reads", null)

        scope.launch {
            try {
                val buffer = ByteArray(bytesToRead)
                // Use withTimeout to ensure we don't hang if bulkTransfer ignores the timeout parameter in some OS versions
                val bytesRead = withTimeout(timeoutMs.toLong() + 500L) {
                    conn.bulkTransfer(endpoint, buffer, buffer.size, timeoutMs)
                }
                
                if (bytesRead < 0) {
                    mainResult(result) { it.error("USB_READ_TIMEOUT", "Timeout reading from USB", null) }
                } else {
                    val actualData = buffer.copyOf(bytesRead)
                    mainResult(result) { it.success(actualData) }
                }
            } catch (e: TimeoutCancellationException) {
                mainResult(result) { it.error("USB_READ_TIMEOUT", "Timeout reading from USB", null) }
            } catch (e: Exception) {
                mainResult(result) { it.error("USB_READ_ERROR", e.message, null) }
            }
        }
    }

    private fun usbDisconnect(result: Result) {
        closeUsb()
        result.success(true)
    }

    private fun closeUsb() {
        try {
            usbInterface?.let { usbConnection?.releaseInterface(it) }
            usbConnection?.close()
        } catch (_: Exception) {}
        usbConnection  = null
        usbEndpointOut = null
        usbEndpointIn  = null
        usbInterface   = null
    }

    private suspend fun requestUsbPermission(manager: UsbManager, device: UsbDevice): Boolean = suspendCancellableCoroutine { cont ->
        val action = "com.thermal.thermal_printer_rs.USB_PERMISSION"
        val receiver = object : android.content.BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                if (intent.action == action) {
                    ctx.unregisterReceiver(this)
                    if (intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) {
                        cont.resume(true) { }
                    } else {
                        cont.resume(false) { }
                    }
                }
            }
        }
        
        val filter = android.content.IntentFilter(action)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            context.registerReceiver(receiver, filter)
        }

        val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            PendingIntent.FLAG_MUTABLE
        } else {
            0
        }
        val intent = PendingIntent.getBroadcast(context, 0, Intent(action), flags)
        manager.requestPermission(device, intent)
        
        cont.invokeOnCancellation {
            try { context.unregisterReceiver(receiver) } catch (e: Exception) {}
        }
    }

    /** Returns a list of connected USB devices as maps. */
    private fun usbList(result: Result) {
        val manager = context.getSystemService(Context.USB_SERVICE) as UsbManager
        val devices = manager.deviceList.values.map { device ->
            mapOf(
                "name"      to device.deviceName,
                "vendorId"  to device.vendorId,
                "productId" to device.productId,
                "manufacturer" to (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) device.manufacturerName else null)
            )
        }
        result.success(devices)
    }

    // ══════════════════════════════════════════════════════════════
    // Bluetooth Classic (SPP)
    // ══════════════════════════════════════════════════════════════

    /**
     * Connects to a BT Classic printer by MAC address.
     *
     * Dart call:
     * ```dart
     * channel.invokeMethod('bt_connect', {'address': 'AA:BB:CC:DD:EE:FF'});
     * ```
     */
    private fun btConnect(call: MethodCall, result: Result) {
        val address = call.argument<String>("address")
            ?: return result.error("INVALID_ARG", "address required", null)

        scope.launch {
            try {
                val adapter = BluetoothAdapter.getDefaultAdapter()
                    ?: run {
                        mainResult(result) { it.error("BT_UNAVAILABLE", "Bluetooth not available on this device", null) }
                        return@launch
                    }

                if (!adapter.isEnabled) {
                    mainResult(result) { it.error("BT_DISABLED", "Bluetooth is disabled. Enable it and retry.", null) }
                    return@launch
                }

                val device: BluetoothDevice = try {
                    adapter.getRemoteDevice(address)
                } catch (e: IllegalArgumentException) {
                    mainResult(result) { it.error("INVALID_ADDRESS", "Invalid MAC address: $address", null) }
                    return@launch
                }

                closeBluetooth()
                adapter.cancelDiscovery() // Discovery slows down BT connections

                val socket = device.createRfcommSocketToServiceRecord(SPP_UUID)
                socket.connect() // Blocks until connected or throws

                btSocket       = socket
                btOutputStream = socket.outputStream
                btInputStream  = socket.inputStream

                mainResult(result) { it.success(true) }

            } catch (e: Exception) {
                mainResult(result) { it.error("BT_ERROR", "Bluetooth connection failed: ${e.message}", null) }
            }
        }
    }

    /**
     * Sends raw ESC/POS bytes to the connected BT Classic printer.
     *
     * Dart call:
     * ```dart
     * channel.invokeMethod('bt_write', {'data': Uint8List(...)});
     * ```
     */
    private fun btWrite(call: MethodCall, result: Result) {
        val data = call.argument<ByteArray>("data")
            ?: return result.error("INVALID_ARG", "data required", null)
        val stream = btOutputStream
            ?: return result.error("NOT_CONNECTED", "Bluetooth printer not connected", null)

        scope.launch {
            try {
                val chunkSize = 4096
                var offset = 0
                while (offset < data.size) {
                    val end = minOf(offset + chunkSize, data.size)
                    // Write a view into the original ByteArray — no boxing to List<Byte>
                    stream.write(data, offset, end - offset)
                    stream.flush()
                    // Small delay to prevent buffer overflow on cheap BT adapters
                    delay(10)
                    offset = end
                }
                mainResult(result) { it.success(true) }
            } catch (e: Exception) {
                mainResult(result) { it.error("BT_WRITE_ERROR", "Bluetooth write failed: ${e.message}", null) }
            }
        }
    }

    /**
     * Reads raw bytes from the connected BT Classic printer.
     */
    private fun btRead(call: MethodCall, result: Result) {
        val bytesToRead = call.argument<Int>("bytes") ?: 1
        val timeoutMs = call.argument<Int>("timeoutMs") ?: 1000
        val stream = btInputStream
            ?: return result.error("NOT_CONNECTED", "Bluetooth printer not connected", null)

        scope.launch {
            try {
                val buffer = ByteArray(bytesToRead)
                // Use withTimeout to prevent blocking forever if printer doesn't respond
                val bytesRead = withTimeout(timeoutMs.toLong()) {
                    var totalRead = 0
                    while (totalRead < bytesToRead) {
                        if (stream.available() > 0) {
                            val read = stream.read(buffer, totalRead, bytesToRead - totalRead)
                            if (read == -1) break
                            totalRead += read
                        } else {
                            delay(10) // Small delay to avoid CPU spinning
                        }
                    }
                    totalRead
                }

                if (bytesRead == 0) {
                    mainResult(result) { it.error("BT_READ_TIMEOUT", "Timeout reading from Bluetooth", null) }
                } else {
                    val actualData = buffer.copyOf(bytesRead)
                    mainResult(result) { it.success(actualData) }
                }
            } catch (e: TimeoutCancellationException) {
                mainResult(result) { it.error("BT_READ_TIMEOUT", "Timeout reading from Bluetooth", null) }
            } catch (e: Exception) {
                mainResult(result) { it.error("BT_READ_ERROR", "Bluetooth read failed: ${e.message}", null) }
            }
        }
    }

    private fun btDisconnect(result: Result) {
        closeBluetooth()
        result.success(true)
    }

    private fun closeBluetooth() {
        try { btInputStream?.close()  } catch (_: Exception) {}
        try { btOutputStream?.close() } catch (_: Exception) {}
        try { btSocket?.close()       } catch (_: Exception) {}
        btInputStream  = null
        btOutputStream = null
        btSocket       = null
    }

    /** Returns the list of paired (bonded) Bluetooth devices. */
    private fun btListPaired(result: Result) {
        val adapter = BluetoothAdapter.getDefaultAdapter()
        if (adapter == null || !adapter.isEnabled) {
            result.success(emptyList<Map<String, String>>())
            return
        }
        val devices = adapter.bondedDevices.map { device ->
            mapOf(
                "name"    to (device.name ?: "Unknown"),
                "address" to device.address,
                "type"    to when (device.type) {
                    BluetoothDevice.DEVICE_TYPE_CLASSIC -> "CLASSIC"
                    BluetoothDevice.DEVICE_TYPE_LE      -> "BLE"
                    BluetoothDevice.DEVICE_TYPE_DUAL    -> "DUAL"
                    else                                -> "UNKNOWN"
                }
            )
        }
        result.success(devices)
    }

    // ── Utility ────────────────────────────────────────────────────

    /** Checks if the Bluetooth adapter is powered on. */
    private fun btIsOn(result: Result) {
        val adapter = BluetoothAdapter.getDefaultAdapter()
        result.success(adapter?.isEnabled == true)
    }

    /** Opens the system Bluetooth settings. */
    private fun openSettings(result: Result) {
        try {
            val intent = Intent(android.provider.Settings.ACTION_BLUETOOTH_SETTINGS)
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            context.startActivity(intent)
            result.success(true)
        } catch (e: Exception) {
            result.error("INTENT_ERROR", "Could not open Bluetooth settings", e.message)
        }
    }

    /** Dispatches a result back to the main thread. */
    private fun mainResult(result: Result, block: (Result) -> Unit) {
        CoroutineScope(Dispatchers.Main).launch { block(result) }
    }
}
