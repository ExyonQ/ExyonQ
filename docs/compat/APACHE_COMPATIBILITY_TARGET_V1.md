# ExyonQ Server — Apache Compatibility Target V1

**Estado:** propuesta de contrato de producto  
**Fecha:** 2026-10-11  
**Ámbito:** ExyonQ Server  
**Decisión de producto:** compatibilidad orientada a sustitución y migración, no clon interno de Apache.

---

## 1. Objetivo

Definir qué debe ofrecer ExyonQ Server para ser una alternativa real a Apache en instalaciones de producción y cuándo será legítimo afirmar que puede sustituirlo.

La meta no es copiar la implementación, los MPM ni cada módulo histórico de Apache. La meta es que una organización pueda retirar Apache de los despliegues incluidos en el alcance, migrar su configuración y aplicaciones, y conservar:

- comportamiento HTTP observable;
- seguridad y aislamiento;
- disponibilidad y operabilidad;
- compatibilidad con PHP y aplicaciones detrás de proxy;
- trazabilidad clara de todo lo que se convierte, cambia o no se admite.

**Regla de comunicación:** hasta cumplir los gates de este documento, ExyonQ no debe anunciar “sustitución completa de Apache”. La formulación correcta antes de entonces será la cobertura concreta verificada.

---

## 2. Definición operativa de “sustituir Apache al 100 %”

“100 %” solo es defendible respecto a un catálogo público, versionado y verificable de escenarios de despliegue. No puede significar “todo módulo escrito por terceros desde hace décadas”, porque Apache permite módulos nativos arbitrarios y handlers en proceso.

Para ExyonQ, el objetivo se divide en tres niveles:

| Nivel | Definición | Declaración permitida |
| --- | --- | --- |
| **A — Sustitución web PHP** | Sitios estáticos, virtual hosts, TLS, PHP-FPM, redirecciones y reescrituras habituales | “Alternativa para sitios PHP y contenido estático cubiertos” |
| **B — Sustitución de aplicación web** | Añade auth, proxy, WebSocket, balanceo, límites, observabilidad y operación | “Alternativa a Apache para el catálogo de aplicaciones web definido” |
| **C — Compatibilidad Apache V1** | Todo el catálogo V1 se migra sin pérdida semántica no declarada; lo no soportado se rechaza inequívocamente | “Compatibilidad Apache V1 verificada” |

La afirmación equivalente a “sustituir Apache al 100 %” solo podrá aplicarse al **Nivel C** y siempre acompañada de la versión del catálogo y de sus límites explícitos.

---

## 3. Principios no negociables

1. **Compatibilidad de comportamiento, no emulación interna.** ExyonQ mantendrá una arquitectura Rust propia.
2. **Cero conversión silenciosa.** Una directiva no soportada nunca puede ignorarse ni transformarse sin diagnóstico.
3. **Configuración centralizada y verificable.** El modo equivalente a `.htaccess` será opcional y estará desactivado por defecto.
4. **Aislamiento de aplicaciones.** PHP y otros runtimes viven fuera del proceso ExyonQ, mediante FastCGI o reverse proxy.
5. **Sin módulos nativos arbitrarios en el proceso.** La extensibilidad será mediante APIs controladas, procesos externos o sidecars.
6. **Seguridad antes que compatibilidad cosmética.** No se reproducirá una conducta insegura por el mero hecho de existir en Apache.
7. **Evidencia real.** Las pruebas de aceptación ejecutarán procesos reales: ExyonQ, PHP-FPM, upstreams HTTP y clientes HTTP; no mocks de la ruta de producción.
8. **No hay soporte implícito.** Toda directiva queda clasificada como soportada, soportada con diferencia declarada, convertida a alternativa, o rechazada.

---

## 4. Catálogo funcional V1

### 4.1 Configuración y virtual hosts — P0

ExyonQ debe poder expresar y validar:

- listeners TCP y Unix cuando aplique;
- virtual hosts por IP, puerto, `Host` y SNI;
- `ServerName`, aliases de host y host por defecto;
- docroot, index files, alias de rutas y redirecciones;
- tipos MIME, charset, cabeceras y páginas de error;
- includes controlados, variables y configuración por entorno;
- validación completa antes del reload;
- configuración efectiva inspectable por vhost y ruta.

**Equivalencia Apache que debe cubrir:** `Listen`, `VirtualHost`, `ServerName`, `ServerAlias`, `DocumentRoot`, `DirectoryIndex`, `Alias`, `Redirect`, `ErrorDocument`, `Include`.

**Gate de aceptación:** una configuración con múltiples dominios y certificados distintos debe validar, cargar, recargar y servir la respuesta esperada para cada combinación host/SNI/ruta.

### 4.2 Ficheros estáticos y reglas de directorio — P0

Debe cubrir:

- GET y HEAD correctos;
- range requests si se declaran soportados;
- ETag, Last-Modified, If-None-Match e If-Modified-Since;
- ficheros binarios y grandes;
- cache-control por ruta o tipo;
- MIME correcto;
- directorio índice y listado de directorios solo bajo activación explícita;
- denegación segura de ficheros especiales;
- aislamiento de docroot frente a traversal, symlinks y TOCTOU.

**Gate de aceptación:** corpus real de ficheros, rutas inválidas, enlaces simbólicos y ficheros especiales; ninguna fuga fuera del docroot ni panic/crash.

### 4.3 Reescritura y redirecciones — P0

Un sustituto práctico de Apache necesita un motor equivalente, aunque no idéntico internamente, a los usos habituales de `mod_rewrite`:

- reglas por host, ruta y directorio;
- condiciones sobre ruta, método, query string, cabeceras, cookies, variables y existencia de fichero/directorio;
- capturas y sustituciones;
- redirecciones 301, 302, 307 y 308;
- reescritura interna con límite de iteraciones;
- preservación o modificación explícita de query string;
- orden de evaluación documentado y determinista;
- modo de diagnóstico que explique la regla aplicada.

**No negociable:** no soportar reglas complejas es admisible en una primera versión; aceptarlas y comportarse de forma distinta no lo es.

**Gate de aceptación:** corpus versionado de configuraciones y peticiones con resultado esperado para URI final, status, `Location`, cabeceras y cuerpo.

### 4.4 TLS y certificados — P0

Debe incluir:

- TLS moderno y configuración segura por defecto;
- SNI y certificados por virtual host;
- carga y reload atómico de certificados;
- ACME y renovación automática, o una integración formalmente definida;
- redirección HTTP a HTTPS;
- HSTS y cabeceras de seguridad configurables;
- mTLS para rutas o vhosts cuando se habilite;
- logs que distingan fallo de handshake, error de certificado y petición HTTP.

**Equivalencia relevante:** `mod_ssl` para despliegues web comunes, no cada opción heredada.

**Gate de aceptación:** pruebas reales de handshake, SNI, renovación/reload, certificados inválidos, cipher policy y mTLS cuando se declare implementado.

### 4.5 PHP y FastCGI — P0

Debe soportar de manera estable:

- PHP-FPM real por socket Unix y TCP;
- selección de pool por sitio o ruta;
- `SCRIPT_FILENAME`, `PATH_INFO`, query string, métodos y cabeceras correctos;
- límites de body y ficheros subidos;
- timeouts de conexión, respuesta y transferencia;
- backpressure;
- fallo, reinicio y no disponibilidad de PHP-FPM;
- respuestas grandes y streaming;
- prevención de ejecución de rutas que no correspondan a scripts PHP autorizados;
- logs correlacionables entre request, FastCGI y error.

**Gate de aceptación:** aplicaciones PHP reales, incluido WordPress u otra aplicación representativa, con subida de ficheros, errores, timeout, reload de PHP-FPM y solicitud única verificada.

### 4.6 Reverse proxy y aplicaciones — P0

Debe cubrir:

- proxy HTTP/1.1 con streaming de request y response;
- upstream por TCP y, cuando se declare, Unix socket;
- WebSocket y upgrades HTTP;
- pools de upstreams;
- health checks;
- balanceo explícitamente documentado;
- timeouts separados: connect, request header, request body, first byte, idle y total;
- retries solo para operaciones idempotentes y bajo política explícita;
- preservación controlada de `Host`, `X-Forwarded-*`, `Forwarded`, `X-Request-Id`;
- protección frente a framing ambiguo y request smuggling;
- errores de upstream sin convertir fallos en respuestas de éxito.

**Gate de aceptación:** upstream sano, lento, caído, que reinicia, que devuelve body grande, WebSocket y respuestas malformadas.

### 4.7 Autenticación y autorización — P0

La cobertura inicial necesaria es:

- Basic Auth con almacenamiento seguro de hashes;
- autorización por ruta, método, identidad y red/IP;
- allow/deny y CIDR;
- integración JWT/OIDC como alternativa moderna;
- control de acceso antes de FastCGI o proxy;
- rate limiting y límites de conexiones por identidad, IP y ruta;
- auditoría sin guardar secretos.

La compatibilidad literal con todos los providers de `mod_auth*` no pertenece al V1. Lo esencial es que la política de acceso actual pueda migrarse a una política equivalente y verificable.

**Gate de aceptación:** combinaciones de rutas públicas/protegidas, credenciales válidas e inválidas, expiración, IP/CIDR, burst de rate limit y auditoría.

### 4.8 Compresión, caché y entrega — P1

Una alternativa madura debe añadir:

- gzip y Brotli negociados correctamente;
- exclusión de tipos ya comprimidos;
- Vary correcto;
- cache-control de estáticos;
- caché de proxy opcional, con claves, invalidación y bypass observables;
- envío eficiente de estáticos sin invalidar las garantías de seguridad.

**Gate de aceptación:** respuestas idénticas funcionalmente con y sin compresión, métricas de caché, invalidación y pruebas de contenido binario.

### 4.9 HTTP/2, HTTP/3 y protocolos — P1

- HTTP/2 completo y robusto es la siguiente prioridad tras P0.
- HTTP/3/QUIC debe entrar solo cuando sus pruebas de interoperabilidad, observabilidad y operación estén maduras.
- No se debe comprometer HTTP/1.1 correcto, proxy o FastCGI para adelantar protocolos.

**Gate de aceptación:** interoperabilidad con clientes reales, concurrencia, cancelación, reset de streams, límites y shutdown limpio.

### 4.10 Observabilidad y operaciones — P0

ExyonQ debe permitir operar sin depender de adivinar:

- access log y error log estructurados;
- correlation ID;
- métricas de requests, latencias, bytes, conexiones, upstreams, TLS, errores y límites;
- logs de configuración, reload y rechazo de directivas;
- endpoints administrativos autenticados;
- health y readiness;
- shutdown ordenado;
- reload atómico o con semántica explícita;
- límites de FD, memoria, workers, conexiones y cuerpos;
- diagnóstico de configuración efectiva.

**Gate de aceptación:** reinicios, reload bajo carga, rotación de logs, límites de recursos, upstreams fallidos y ausencia de panics/crashes.

---

## 5. `.htaccess`: decisión explícita

Apache permite configuración por directorio. Es una razón frecuente por la que una migración falla, pero también añade coste por request, configuración difícil de auditar y superficies de seguridad.

ExyonQ debe ofrecer dos caminos:

| Modo | Comportamiento | Prioridad |
| --- | --- | --- |
| **Migración recomendada** | Importar `.htaccess` hacia configuración central, mostrar diferencias y rechazos | P0 |
| **Compatibilidad controlada** | Leer reglas por directorio solo en vhosts habilitados y con cache/invalidador seguro | P2 |

No se activará una interpretación dinámica de `.htaccess` por defecto. La herramienta de migración debe generar un informe de cada fichero: convertido, convertido con diferencia o rechazado.

---

## 6. Importador Apache: producto, no parser auxiliar

El importador es parte esencial de la sustitución. Debe aceptar un conjunto explícito de configuraciones Apache y producir:

1. configuración ExyonQ equivalente;
2. lista de directivas convertidas;
3. diferencias de semántica, con severidad;
4. directivas bloqueantes no soportadas;
5. origen exacto: archivo, línea e include;
6. propuesta de alternativa cuando exista;
7. una prueba de validación contra la configuración generada.

### 6.1 Clasificación obligatoria de directivas

| Estado | Significado |
| --- | --- |
| **SUPPORTED_EXACT** | Comportamiento equivalente dentro del contrato |
| **SUPPORTED_DECLARED_DIFFERENCE** | Diferencia conocida, visible y aceptada por el operador |
| **MIGRATED_TO_EXYONQ_CONSTRUCT** | Se convierte a una construcción propia equivalente |
| **MANUAL_ACTION_REQUIRED** | Requiere una decisión explícita del operador |
| **REJECTED_UNSUPPORTED** | ExyonQ no la implementa; el despliegue no puede pasar inadvertidamente |

### 6.2 Prohibiciones

- No ignorar directivas desconocidas.
- No importar una configuración que quede válida pero cambie el acceso, el docroot, el proxy o las reescrituras sin marcarlo.
- No usar heurísticas que alteren una regla de seguridad.
- No declarar una migración correcta sin ejecutar la validación de la configuración resultante.

---

## 7. Extensibilidad y módulos Apache no equivalentes

El V1 no intentará ejecutar módulos binarios de Apache ni handlers embebidos como si fueran compatibles ABI. Eso arrastraría una superficie enorme de seguridad, mantenimiento y bloqueo arquitectónico.

La alternativa será:

- configuración declarativa nativa;
- integración por reverse proxy, FastCGI o sidecar;
- hooks definidos y estables, si se introducen, con límites de tiempo, memoria y permisos;
- API administrativa y de observabilidad;
- migradores específicos para usos frecuentes.

Ejemplos que requieren ruta de migración y no compatibilidad binaria: handlers Python/Perl embebidos, módulos C propios, autenticadores empresariales propietarios y filtros heredados.

---

## 8. Fases de ejecución

### Fase 0 — Inventario de compatibilidad

Crear un inventario de configuraciones Apache reales o representativas y clasificar cada directiva según la tabla de la sección 6.

**Salida obligatoria:**

- `APACHE_DIRECTIVE_CATALOG_V1.md`;
- corpus de configuraciones válidas e inválidas;
- top 20 de directivas y módulos por frecuencia;
- lista de bloqueantes de migración.

**Gate:** no iniciar una implementación amplia sin saber qué configuraciones quiere migrar ExyonQ.

### Fase 1 — Núcleo de sustitución web PHP

Cerrar configuración, virtual hosts, estáticos, TLS, rewrite, PHP-FPM, auth básica, logs y importador de estos elementos.

**Gate:** migración reproducible de un sitio PHP real con HTTPS y reglas de reescritura, sin Apache en el camino de datos.

### Fase 2 — Sustitución de aplicación web

Cerrar reverse proxy, WebSocket, pools, health checks, límites, auth moderna, rate limiting, métricas y operación bajo carga.

**Gate:** aplicación real detrás de proxy, recarga sin pérdida semántica, upstreams con fallo controlado y regresión completa.

### Fase 3 — Compatibilidad por directorio y servicios comunes

Completar importación de `.htaccess`, compresión, caché de proxy y reglas de acceso de mayor complejidad.

**Gate:** informe de migración completo de una instalación Apache con configuración central e instrucciones por directorio.

### Fase 4 — Protocolos y extensibilidad

HTTP/2 completo, HTTP/3 si está justificado por evidencia, y superficie de extensión segura.

**Gate:** interoperabilidad real, pruebas de carga comparables y auditoría de seguridad específica.

---

## 9. Matriz mínima de regresión

Toda funcionalidad Apache-compat debe aportar pruebas reales que preserven el contrato existente de ExyonQ:

| Área | Casos mínimos |
| --- | --- |
| Static | GET/HEAD, rangos si soportados, binarios, grandes, caché, traversal, symlinks, FIFO/sockets y TOCTOU |
| Rewrite | reglas, condiciones, captures, loops, query string, redirects y precedencia |
| TLS | SNI, múltiples certificados, handshake inválido, reload, HTTP→HTTPS y mTLS si aplica |
| FastCGI | PHP-FPM real, errores, timeout, caída/reinicio, body grande, uploads, PATH_INFO y ejecución única |
| Proxy | upstream sano/caído/lento, keepalive, bodies grandes, timeout, retry, WebSocket y framing |
| Auth | Basic, JWT/OIDC cuando exista, CIDR, rate limit, denegación y logs sin secretos |
| Operación | start/stop/reload, logs, FD, memoria, conexiones, señales y ausencia de panic/crash |
| Migración | conversión exacta, diferencia declarada, rechazo bloqueante, archivo/línea de origen e idempotencia |

La evidencia de cierre deberá provenir de Netcup AMD64 y Oracle ARM64 para las rutas de producción definidas por el proyecto. Las pruebas locales ayudan a iterar, pero no sustituyen dicha evidencia.

---

## 10. Gates de declaración pública

### Gate A — “Compatible con sitios PHP”

- 100 % del corpus A declarado se importa o se rechaza de forma bloqueante.
- Los sitios PHP definidos funcionan con TLS y rewrite.
- No hay diferencias silenciosas de docroot, acceso, redirects ni PHP.
- Static + FastCGI + Operaciones pasan en ambas arquitecturas de evidencia.

### Gate B — “Alternativa para aplicaciones web”

Además de Gate A:

- Proxy, WebSocket, auth, límites, health checks y observabilidad pasan el corpus B.
- Los fallos de upstream y reload son deterministas y observables.
- No hay panic, crash, fuga de recursos ni degradación no explicada durante las campañas definidas.

### Gate C — “Compatibilidad Apache V1 verificada”

Además de Gate B:

- 100 % de las directivas del catálogo V1 están clasificadas.
- Toda configuración declarada dentro de alcance migra con evidencia de comportamiento.
- Todo lo que queda fuera es rechazado de forma inequívoca con una ruta de migración documentada.
- El informe de compatibilidad, limitaciones y resultados se publica con versión y fecha.

---

## 11. Qué no se debe hacer

- No construir un clon de Apache ni una ABI de módulos C.
- No meter intérpretes de aplicaciones dentro del proceso principal.
- No abrir una fase de benchmark competitivo antes de cerrar una regresión funcional o de seguridad.
- No aceptar compatibilidad parcial sin diagnosticarla.
- No priorizar HTTP/3, cache avanzada o funciones vistosas mientras rewrite, TLS, PHP, proxy, auth y operación no estén sellados.
- No usar `.htaccess` dinámico como configuración predeterminada.

---

## 12. Próxima decisión técnica

**Acción única recomendada:** crear el inventario `APACHE_DIRECTIVE_CATALOG_V1.md` a partir de una muestra inicial de configuraciones reales y definir el corpus A (sitio PHP) y B (aplicación detrás de proxy).

Hasta que ese inventario exista, cualquier lista de funcionalidades seguirá siendo razonable pero no estará priorizada por demanda real de migración.

