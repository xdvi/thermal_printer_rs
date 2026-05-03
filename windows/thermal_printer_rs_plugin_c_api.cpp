#include "include/thermal_printer_rs/thermal_printer_rs_plugin_c_api.h"

#include <flutter/plugin_registrar_windows.h>

#include "thermal_printer_rs_plugin.h"

void ThermalPrinterRsPluginCApiRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar) {
  thermal_printer_rs::ThermalPrinterRsPlugin::RegisterWithRegistrar(
      flutter::PluginRegistrarManager::GetInstance()
          ->GetRegistrar<flutter::PluginRegistrarWindows>(registrar));
}
