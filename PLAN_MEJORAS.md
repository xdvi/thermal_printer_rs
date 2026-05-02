# Plan de mejoras para `thermal_printer_rs`

## Resumen ejecutivo

Si, es posible hacer `thermal_printer_rs` mas eficiente que `thermal_printer_plus`, pero no por usar Rust automaticamente.

Hoy `thermal_printer_rs` pierde eficiencia por tres causas principales:

1. Hace copias innecesarias de buffers antes de escribir.
2. Usa `block_in_place` + `block_on` en rutas criticas de conexion e impresion.
3. Tiene una arquitectura mixta entre singleton global, `Mutex<Box<dyn Transport>>` y worker de fondo incompleto.

Si se corrigen esas tres areas primero, el proyecto puede aspirar a quedar por delante de `thermal_printer_plus` en la ruta critica de impresion.

## Objetivo realista

Objetivo a 2-3 iteraciones tecnicas:

- 20% a 40% menos RAM incremental en impresion de texto y tickets medianos.
- 15% a 30% menos CPU en impresion normal.
- 20% a 35% menos latencia p95 en `connect + print + flush` por TCP y USB desktop.
- Variabilidad menor en payloads grandes por mejor chunking y menos copias.

Objetivo agresivo, pero posible si se rediseña el hot path:

- 30% a 50% menos RAM que `thermal_printer_plus` en tickets sin imagen.
- 20% a 35% menos CPU que `thermal_printer_plus` en TCP/USB desktop.

Donde sera mas dificil ganar:

- Android Bluetooth Classic, porque hoy depende de puente nativo separado.
- BLE, porque el limite real suele ser el enlace y el MTU, no tanto el lenguaje.

## Hallazgos actuales

### 1. Bloqueo y sobrecarga async en la ruta critica

Archivos:

- `rust/src/printer.rs`
- `rust/src/api/simple.rs`

Problema:

- `connect()`, `disconnect()` y `send_buffer()` usan `tokio::task::block_in_place()` y luego `Handle::current().block_on(...)`.
- Eso mete sobrecarga evitable, complica la concurrencia y reduce la ventaja de Rust.

Impacto:

- Mas CPU por impresion.
- Mayor riesgo de cuellos de botella y bloqueos sutiles.

### 2. Copias extra de memoria

Archivos:

- `rust/src/printer.rs`
- `rust/src/escpos_adapter.rs`

Problema:

- `send_buffer()` clona `buf` con `buf.to_vec()` antes de escribir.
- `MemoryDriver` usa `Arc<Mutex<Vec<u8>>>` aunque la construccion del ticket es local y sincronica.

Impacto:

- Mas RAM en payloads medianos y grandes.
- Mas CPU por asignacion, copia y lock.

### 3. Worker de fondo incompleto

Archivo:

- `rust/src/jobs.rs`

Problema:

- `Disconnect` no esta implementado realmente.
- No hay cancelacion, no hay ack por trabajo, no hay estado observable, no hay shutdown coordinado.

Impacto:

- Dificulta hacer una cola realmente eficiente y confiable.

### 4. Android nativo con puntos de costo y fragilidad

Archivo:

- `android/src/main/kotlin/com/thermal/thermal_printer_rs/ThermalPrinterRsPlugin.kt`

Problema:

- USB usa `Thread.sleep(2000)` para esperar permisos.
- BT Classic escribe de una sola vez, sin politica uniforme de chunking.

Impacto:

- Latencia artificial.
- Peor estabilidad con payloads grandes.

### 5. API publica aun limitada frente a `thermal_printer_plus`

Archivos:

- `lib/thermal_printer_rs.dart`
- `README.md`
- `CHANGELOG.md`

Problema:

- Falta API rica para estado, eventos, capacidades, cancelacion y escritura raw unificada.
- Documentacion aun minima.

Impacto:

- Aunque el core mejore, el paquete seguira viendose menos completo.

## Plan por fases

## Fase 0 - Baseline y medicion

Objetivo:

- Dejar de estimar y empezar a medir.

Entregables:

- Benchmark Rust para `build_text`, `build_receipt`, `send_buffer` y cola de trabajo.
- Benchmark Flutter para `init + printText`, `init + printReceipt`, `enqueueText`.
- Casos base:
  - texto 2 KiB
  - ticket 8 KiB
  - QR 4 KiB
  - bitmap 128 KiB
- Metricas:
  - RAM incremental
  - CPU promedio
  - latencia p50 y p95
  - numero de copias por payload
  - throughput bytes/s

Prioridad:

- Alta, antes de optimizar mas.

## Fase 1 - Rehacer la ruta critica de IO

Objetivo:

- Eliminar el costo principal de concurrencia y copias.

Cambios:

1. Reemplazar `Arc<Mutex<Box<dyn Transport>>>` por un task propietario del transporte.
2. El API publico debe enviar comandos a ese task por canal y recibir respuesta por `oneshot`.
3. Eliminar `block_in_place` y `block_on` de `printer.rs` y `api/simple.rs`.
4. Hacer que `send_buffer` trabaje con buffer poseido y no con `&[u8]` copiado internamente.
5. Guardar `JoinHandle` del worker y cerrar ordenadamente al desconectar.

Resultado esperado:

- Menos CPU.
- Menos RAM transitoria.
- Mejor estabilidad bajo concurrencia.

Impacto estimado:

- RAM: mejora de 10% a 25%.
- CPU: mejora de 10% a 20%.

## Fase 2 - Optimizar generacion de ESC/POS

Objetivo:

- Reducir locks y realocaciones al construir tickets.

Cambios:

1. Reemplazar `MemoryDriver` basado en `Arc<Mutex<Vec<u8>>>` por una variante mas liviana.
2. Prealocar capacidad para tickets comunes.
3. Separar builders para texto, ticket, QR y barcode con estimacion de capacidad.
4. Revisar `format_line()` para evitar trabajo extra en cada fila.
5. Introducir buffer reutilizable para builders repetitivos.

Resultado esperado:

- Menos asignaciones.
- Menos CPU por ticket.

Impacto estimado:

- RAM: mejora de 5% a 15%.
- CPU: mejora de 8% a 15%.

## Fase 3 - Politica uniforme de chunking y backpressure

Objetivo:

- Tener una politica consistente por transporte.

Cambios:

1. Definir `chunk_size` por transporte:
   - TCP: 8 KiB o mayor si el benchmark lo valida.
   - USB: 4 KiB por compatibilidad inicial.
   - BLE: dinamico segun MTU negociado.
   - Android BT Classic: 4 KiB u 8 KiB con pruebas.
2. Exponer configuracion opcional avanzada desde Dart.
3. Agregar backpressure real cuando la cola se sature.
4. Permitir cancelacion de trabajos pendientes.

Resultado esperado:

- Menos picos de RAM.
- Menos errores intermitentes en impresoras lentas.

Impacto estimado:

- RAM: mejora de 5% a 10%.
- Estabilidad: mejora alta.

## Fase 4 - Corregir plataformas nativas debiles

Objetivo:

- Quitar cuellos de botella fuera del core Rust.

Cambios Android:

1. Reemplazar `Thread.sleep(2000)` en USB por callback real de permisos.
2. Unificar politica de chunking en USB y BT Classic.
3. Evitar escritura monolitica en BT Classic.
4. Medir tiempo de descubrimiento, conexion y escritura por separado.

Cambios BLE:

1. Negociar MTU si la plataforma lo permite.
2. Ajustar delay entre chunks segun capacidad real del dispositivo.
3. Hacer configurable el modo `WithoutResponse` vs `WithResponse` cuando aplique.

Resultado esperado:

- Menor latencia visible.
- Mejor throughput en hardware real.

## Fase 5 - Completar API publica y observabilidad

Objetivo:

- Que el paquete no solo sea rapido, sino tambien utilizable y mantenible.

Cambios:

1. Agregar `state`, `events` y `runtimeCapabilities`.
2. Exponer errores estables y tipados.
3. Unificar la historia de Bluetooth:
   - BLE en Rust donde aplique.
   - BT Classic Android como transporte de primera clase.
4. Exponer `writeBytes()` como API publica principal.
5. Agregar `TicketBuilder` o API equivalente si se busca competir directamente con `thermal_printer_plus`.
6. Mejorar `README.md`, ejemplos y `CHANGELOG.md`.

Resultado esperado:

- Mejor adopcion.
- Menos costo de soporte.
- Comparacion mas justa frente a `thermal_printer_plus`.

## Fase 6 - Calidad, pruebas y CI de rendimiento

Objetivo:

- Evitar regresiones.

Cambios:

1. Agregar tests de carga para payloads medianos y grandes.
2. Agregar bench automatizado con umbrales minimos.
3. Fallar CI si sube RAM, CPU o latencia por encima de tolerancias definidas.
4. Probar hardware real al menos en:
   - TCP 9100
   - USB desktop
   - Android USB
   - Android BT Classic
   - BLE compatible

## Orden recomendado

Orden con mejor retorno tecnico:

1. Fase 0
2. Fase 1
3. Fase 2
4. Fase 3
5. Fase 4
6. Fase 5
7. Fase 6

No recomiendo empezar por nuevas features antes de cerrar Fase 1 y Fase 2.

## Metas de comparacion contra `thermal_printer_plus`

Para considerar que `thermal_printer_rs` ya lo supero:

### Ticket de texto simple

- RAM incremental <= 4 MiB
- CPU promedio <= 3%
- Latencia p95 <= 0.85x de `thermal_printer_plus`

### Ticket mediano

- RAM incremental <= 7 MiB
- CPU promedio <= 5%
- Latencia p95 <= 0.80x de `thermal_printer_plus`

### Payload grande

- RAM incremental <= 14 MiB
- Sin duplicacion completa del payload en la ruta critica
- Error rate <= `thermal_printer_plus`

## Riesgos

1. Reescribir concurrencia sin medir puede empeorar estabilidad.
2. Optimizar solo Rust no arregla cuellos nativos de Android.
3. BLE puede seguir limitado por hardware aunque el codigo mejore mucho.
4. Competir en completitud con `thermal_printer_plus` requiere trabajo de API, no solo rendimiento.

## Recomendacion final

Si el objetivo principal es ganar en eficiencia real, yo haria primero esto:

1. Benchmark base.
2. Eliminar `block_in_place` y copia de `buf.to_vec()`.
3. Rehacer worker con ownership del transporte.
4. Optimizar `MemoryDriver` y builders.
5. Corregir Android USB permission flow y chunking BT Classic.

Con esas cinco acciones, `thermal_printer_rs` ya tendria posibilidades reales de quedar por delante de `thermal_printer_plus` en rendimiento bruto.
