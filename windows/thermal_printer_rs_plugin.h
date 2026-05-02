#ifndef FLUTTER_PLUGIN_THERMAL_PRINTER_RS_PLUGIN_H_
#define FLUTTER_PLUGIN_THERMAL_PRINTER_RS_PLUGIN_H_

#include <flutter/method_channel.h>
#include <flutter/plugin_registrar_windows.h>

#include <memory>

namespace thermal_printer_rs {

class ThermalPrinterRsPlugin : public flutter::Plugin {
 public:
  static void RegisterWithRegistrar(flutter::PluginRegistrarWindows *registrar);

  ThermalPrinterRsPlugin();

  virtual ~ThermalPrinterRsPlugin();

  // Disallow copy and assign.
  ThermalPrinterRsPlugin(const ThermalPrinterRsPlugin&) = delete;
  ThermalPrinterRsPlugin& operator=(const ThermalPrinterRsPlugin&) = delete;

  // Called when a method is called on this plugin's channel from Dart.
  void HandleMethodCall(
      const flutter::MethodCall<flutter::EncodableValue> &method_call,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);
};

}  // namespace thermal_printer_rs

#endif  // FLUTTER_PLUGIN_THERMAL_PRINTER_RS_PLUGIN_H_
