import 'package:flutter/material.dart';
import 'package:thermal_printer_rs/thermal_printer_rs.dart';

void main() {
  runApp(const PrinterExampleApp());
}

class PrinterExampleApp extends StatelessWidget {
  const PrinterExampleApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'thermal_printer_rs — Example',
      debugShowCheckedModeBanner: false,
      theme: ThemeData.dark(useMaterial3: true).copyWith(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF00E5FF),
          brightness: Brightness.dark,
        ),
      ),
      home: const PrinterDemoPage(),
    );
  }
}

class PrinterDemoPage extends StatefulWidget {
  const PrinterDemoPage({super.key});

  @override
  State<PrinterDemoPage> createState() => _PrinterDemoPageState();
}

class _PrinterDemoPageState extends State<PrinterDemoPage> {
  final _ipController     = TextEditingController(text: '192.168.1.100');
  final _portController   = TextEditingController(text: '9100');
  bool  _connected        = false;
  bool  _loading          = false;
  String _log             = 'No activity.';

  void _setLog(String msg) => setState(() => _log = msg);
  void _setLoading(bool v) => setState(() => _loading = v);

  Future<void> _connect() async {
    _setLoading(true);
    try {
      await ThermalPrinterRs.initTcp(
        host: _ipController.text.trim(),
        port: int.tryParse(_portController.text) ?? 9100,
      );
      await ThermalPrinterRs.connect();
      setState(() => _connected = true);
      _setLog('[OK] Connected to ${_ipController.text}:${_portController.text}');
    } on PrinterException catch (e) {
      _setLog('[ERROR] Error: ${e.message}');
    } catch (e) {
      _setLog('[ERROR] Unexpected error: $e');
    } finally {
      _setLoading(false);
    }
  }

  Future<void> _printHello() async {
    _setLoading(true);
    try {
      // ── High-level call ───────────────────────────────────
      final result = await ThermalPrinterRs.printText('Hello World');
      // ───────────────────────────────────────────────────────────
      _setLog('[OK] Printed: ${result.bytesSent} bytes sent');
    } on PrinterException catch (e) {
      _setLog('[ERROR] ${e.message}');
    } finally {
      _setLoading(false);
    }
  }

  Future<void> _printReceipt() async {
    _setLoading(true);
    try {
      final result = await ThermalPrinterRs.printReceipt(
        title: 'SALE TICKET',
        lines: const [
          ('American Coffee',  '\$45.00'),
          ('Mixed Sandwich',   '\$89.00'),
          ('Mineral Water',    '\$20.00'),
        ],
        total: '\$154.00',
        qrData: 'https://mystore.com/invoice/00123',
      );
      _setLog('[OK] Receipt printed: ${result.bytesSent} bytes');
    } on PrinterException catch (e) {
      _setLog('[ERROR] ${e.message}');
    } finally {
      _setLoading(false);
    }
  }

  Future<void> _enqueueReceipt() async {
    // Note: No _setLoading(true) here because it's non-blocking!
    try {
      await ThermalPrinterRs.enqueueReceipt(
        title: 'ASYNC RECEIPT',
        lines: const [
          ('Background Item', '\$10.00'),
          ('Queue Success',   '\$0.00'),
        ],
        total: '\$10.00',
      );
      _setLog('[ENQUEUE] Job enqueued to background worker');
    } catch (e) {
      _setLog('[ERROR] Enqueue error: $e');
    }
  }

  Future<void> _disconnect() async {
    await ThermalPrinterRs.disconnect();
    setState(() => _connected = false);
    _setLog('[DISCONNECT] Disconnected.');
  }

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;

    return Scaffold(
      appBar: AppBar(
        title: const Text('thermal_printer_rs'),
        backgroundColor: cs.surfaceContainerHighest,
      ),
      body: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // ── Connection ────────────────────────────────────────────
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('TCP Configuration',
                        style: Theme.of(context).textTheme.titleMedium),
                    const SizedBox(height: 12),
                    Row(children: [
                      Expanded(
                        flex: 3,
                        child: TextField(
                          controller: _ipController,
                          decoration: const InputDecoration(
                            labelText: 'IP / Host',
                            border: OutlineInputBorder(),
                          ),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: TextField(
                          controller: _portController,
                          keyboardType: TextInputType.number,
                          decoration: const InputDecoration(
                            labelText: 'Port',
                            border: OutlineInputBorder(),
                          ),
                        ),
                      ),
                    ]),
                    const SizedBox(height: 12),
                    FilledButton.icon(
                      onPressed: _loading || _connected ? null : _connect,
                      icon: const Icon(Icons.wifi),
                      label: const Text('Connect'),
                    ),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 16),
            // ── Print operations ─────────────────────────────
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                FilledButton.icon(
                  onPressed: _loading || !_connected ? null : _printHello,
                  icon: const Icon(Icons.print),
                  label: const Text('Hello World'),
                ),
                FilledButton.icon(
                  onPressed: _loading || !_connected ? null : _printReceipt,
                  icon: const Icon(Icons.receipt_long),
                  label: const Text('Complete Receipt'),
                ),
                ElevatedButton.icon(
                  onPressed: !_connected ? null : _enqueueReceipt,
                  icon: const Icon(Icons.queue),
                  label: const Text('Enqueue (Async)'),
                  style: ElevatedButton.styleFrom(
                    backgroundColor: Colors.indigo,
                    foregroundColor: Colors.white,
                  ),
                ),
                OutlinedButton.icon(
                  onPressed: _loading || !_connected ? null : _disconnect,
                  icon: const Icon(Icons.wifi_off),
                  label: const Text('Disconnect'),
                ),
              ],
            ),
            const SizedBox(height: 16),
            // ── Log ─────────────────────────────────────────────────
            Expanded(
              child: Card(
                color: Colors.black87,
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('Log',
                          style: Theme.of(context)
                              .textTheme
                              .labelSmall
                              ?.copyWith(color: cs.primary)),
                      const SizedBox(height: 8),
                      if (_loading) const LinearProgressIndicator(),
                      const SizedBox(height: 8),
                      Text(_log,
                          style: const TextStyle(
                            fontFamily: 'monospace',
                            color: Colors.greenAccent,
                          )),
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
