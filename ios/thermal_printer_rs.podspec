#
# thermal_printer_rs — iOS podspec
#
Pod::Spec.new do |s|
  s.name             = 'thermal_printer_rs'
  s.version          = '0.1.0'
  s.summary          = 'Cross-platform ESC/POS thermal printing via Rust + Flutter.'
  s.description      = <<-DESC
    Provides BLE printing support on iOS via CoreBluetooth.
    TCP/IP printing is handled natively by the Rust core via flutter_rust_bridge.
    USB and Bluetooth Classic (SPP) are not available on iOS without MFi certification.
  DESC
  s.homepage         = 'https://github.com/your-org/thermal_printer_rs'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'thermal_printer_rs' => 'dev@example.com' }
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'

  s.dependency 'Flutter'
  s.platform         = :ios, '13.0'

  # CoreBluetooth — required for BLE transport
  s.frameworks       = 'CoreBluetooth'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE'                         => 'YES',
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]'   => 'i386'
  }
  s.swift_version = '5.7'

  # === Cargokit Integration ===
  s.script_phase = {
    :name => 'Cargokit Build',
    :script => 'sh "$PODS_TARGET_SRCROOT/../cargokit/build_pod.sh" ../rust thermal_printer_rs',
    :execution_position => :before_compile,
    :input_files => ['${PODS_TARGET_SRCROOT}/../rust/src/**/*.rs', '${PODS_TARGET_SRCROOT}/../rust/Cargo.toml'],
    :output_files => ['${DERIVED_FILE_DIR}/libthermal_printer_rs.a']
  }
  s.vendored_libraries = 'libthermal_printer_rs.a'
end
