# Las tools de los bots: qué se porta y en qué orden

3 de noviembre de 2026. Las tools MCP de los bots SRE salen de una colección privada de herramientas de operación (Rust, Go y Python) escritas por el autor para una instalación concreta. Aquí entran **reescritas en versión generalista**: el mismo reparto hexagonal, el mismo contrato (`doctor`, `discover`, escrituras con vista previa), y **nada de aquella instalación**: ni cuentas, ni regiones, ni entornos, ni equipos, ni nombres de dominio como valores por defecto. Todo llega por configuración. `make -C tools vendor-words` lo comprueba en cada CI.

```text
tools/
├── mcp/               el servidor: ToolHost sobre rmcp, catálogo, argumentos     hecho
├── argo-workflows/    el primer tool, solo lectura                               hecho
├── kube/              un tool por verbo kubectl; escrituras solo con KUBE_WRITES  hecho
├── grafana/           Loki, Prometheus, dashboards y alertas con token de SA        hecho
├── opsgenie/          alertas de un alcance con API key; el daemon espejo abre incidentes  hecho
├── aws/               CloudWatch, Lambda y Logs por el CLI `aws`; regiones de una allowlist   hecho
├── messaging/         SQS y SNS por el CLI `aws`; escrituras con MESSAGING_WRITES y evidencia  hecho
└── <los de abajo>/    uno por PR
```

## Triaje

| Bucket | Qué | Por qué |
|---|---|---|
| **Se porta** (valor SRE genérico) | abajo, en orden | lo que `diag-k8s`, `diag-cloud` y `fix` necesitan ([diseño §7](design.md)) |
| **Genérico pero no SRE** | flujo git con commits firmados y PR, Sonar local, sincronización de repos, ITs de Java, linter de contratos API | útiles para un bot de código, no para esta brigada; se deciden aparte |
| **De dominio, no portables** | stock, ubicaciones, mapa de almacén, SKU, picking, sorting, reglas DRL, pre-synchro, producto, minería de eventos, pedidos XML, clasificación, bump de Maven (v1 y v2), sesión de portal, token de servicio, cookie de Chrome, e2e de webapp, y toda tool Python ya portada a Rust | hablan el lenguaje de una instalación; no hay versión generalista posible |

## Orden del port

| # | Tool | Origen (LOC) | Qué cambia en la versión generalista | Para |
|---|---|---|---|---|
| 1 | `mcp/` | servidor compartido (900) | nombre y textos; `doctor`/`discover` como constructores del catálogo | todo |
| 2 | `argo-workflows/` | (800) | se añaden `doctor` y `discover`; `http` solo para localhost; namespace por defecto `argo` | fase 3, Argo CD/Workflows |
| 3 | `kube/` | **nuevo**, no existe en el origen | hecho: 8 lecturas en todo servidor y 5 escrituras solo con `KUBE_WRITES=true`; namespace contra la allowlist `KUBE_NAMESPACES`, nunca `secrets`, manifiestos sin los kinds del §8.1, `diff` que devuelve `differences`, `apply` tras diff y dry-run de servidor, `scale` dentro de `KUBE_MAX_REPLICAS` | fase 1, `diag-k8s` y `fix` |
| 4 | `grafana/` | Loki y dashboards (7 200) | hecho: token de service account en vez de la cookie del navegador; fuera la promoción stage→pro, las escrituras de dashboards, el almacén SQLite y las convenciones de nombres; entran Prometheus y las alertas por el proxy de datasources; 10 tools, solo lectura | `diag-k8s`, el bot `sre-grafana` |
| 5 | `opsgenie/` | MCP + daemon (7 100) | hecho: API key REST (`GenieKey`) en vez de la sesión SSO de Edge; alcance por equipos y escalados, por nombre o id, nunca vacío; 5 lecturas y, con `OPSGENIE_WRITES=true`, `alert_acknowledge`, `alert_note` y `alert_close` con vista previa; el cierre lleva la nota en la misma llamada y comprueba el `202` por `requests/{id}`; `opsgenie-mirror` abre un incidente de axlr-bots por alerta nueva y lo resuelve al cerrarse (línea base, estado 0600, `--once`, `--dry-run`): la entrada «espejo» del diseño §5, en un solo sentido | entrada de alertas |
| 6 | `aws/` (un crate: core, métricas y Lambda) | sesiones, CloudWatch, Lambda (280 + 3 400 + 2 200 + 2 600) | hecho: subproceso `aws` CLI con `--output json` en vez del SDK (el CLI resuelve IRSA, SSO y perfiles igual para todos; sin crates del SDK en CI); sin perfil, cuenta ni región por defecto: `AWS_MCP_REGIONS` es una allowlist y sin ella cada llamada nombra la suya, nunca la del entorno; 9 tools de solo lectura (`metric_query`, `lambda_health`, `alarms`, `lambda_functions`, `lambda_function`, `logs_filter`, `logs_groups`); series compactas, paginación propia con `--max-items`, errores del CLI traducidos, sin variables de entorno de las funciones | fase 2, `diag-cloud` |
| 7 | `messaging/` (un crate: SNS, SQS y DLQ) | SNS/SQS, DLQ (1 300 + 2 400 + 2 000) | hecho: sobre `aws-mcp` como librería (CLI, regiones, errores); sin alias ni catálogos de colas: cada llamada nombra su cola (URL o nombre) o su topic; 8 lecturas (`queues`, `queue` con histórico, `peek` que rechaza colas con RedrivePolicy, `redrive_status`, `topics`, `topic`) y, con `MESSAGING_WRITES=true`, `send`, `publish`, `redrive` nativo, `redrive_cancel` y `drain`, todas con vista previa y `confirm=true`; `drain` guarda cada lote en un JSON Lines 0600 antes de borrarlo y nunca usa `PurgeQueue`; fuera los tokens de aprobación con frase y las etiquetas de vertical | `fix` en colas |
| 8 | `db-live-queries/` | túneles SSM y consultas (7 900) | sin hosts ni puertos fijos; cada BD por configuración | `diag-cloud`, los bots `dba-*` |
| 9 | `redis/` | búsqueda y purga de claves (Python, 400) | reescrito en Rust; purga con vista previa | `dba-redis` |

Tamaños: 1 y 2, un día; 3, dos o tres; 4 y 5, tres o cuatro cada uno; 6 a 8, dos o tres cada uno; 9, uno.

## Reglas que cumple cada tool

| Regla | Dónde se comprueba |
|---|---|
| `doctor` y `discover` en el catálogo | test de contrato del tool |
| stdout es del protocolo; nada de secretos en respuestas ni errores | tests del `doctor` (`!contains(token)`) |
| escritura: vista previa por defecto, `confirm=true` para ejecutar, el camino de vista previa no llama al puerto de escritura | test del caso de uso con doble |
| respuestas acotadas: páginas, `tail`, bytes | dominio |
| dominio, puertos, casos de uso y adaptadores separados; casos de uso probados con dobles | layout del crate |
| 80 % de líneas por crate, `make -C tools ci` en verde | `tools/Makefile`, CI |
| sin palabras del proveedor | `make -C tools vendor-words` |
