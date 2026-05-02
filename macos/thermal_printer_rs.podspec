#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint thermal_printer_rs.podspec` to validate before publishing.
#
Pod::Spec.new do |s|
  s.name             = 'thermal_printer_rs'
  s.version          = '0.0.1'
  s.summary          = 'A new Flutter plugin project.'
  s.description      = <<-DESC
A new Flutter plugin project.
                       DESC
  s.homepage         = 'http://example.com'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'Your Company' => 'email@example.com' }

  s.source           = { :path => '.' }
  s.source_files = 'Classes/**/*'

  # If your plugin requires a privacy manifest, for example if it collects user
  # data, update the PrivacyInfo.xcprivacy file to describe your plugin's
  # privacy impact, and then uncomment this line. For more information,
  # see https://developer.apple.com/documentation/bundleresources/privacy_manifest_files
  # s.resource_bundles = {'thermal_printer_rs_privacy' => ['Resources/PrivacyInfo.xcprivacy']}

  s.dependency 'FlutterMacOS'

  s.platform = :osx, '10.11'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
  s.swift_version = '5.0'

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
