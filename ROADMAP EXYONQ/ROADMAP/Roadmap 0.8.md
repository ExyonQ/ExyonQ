# Roadmap ExyonQ — v0.8 (F4 · TLS y certificados)

> Parte de [ROADMAP_PARIDAD_EXYONQ.md](../ROADMAP_PARIDAD_EXYONQ.md) · **Estado: 🔲 No iniciada (0/6)** · Base: v0.4.13

**Objetivo:** paridad TLS con `mod_ssl`/nginx + el ciclo ACME más cómodo del mercado (ventaja sobre los tres rivales).

**Cierra:** NG-P2, AP-P3 → paridad TLS.

---

## Requisitos

- [ ] **NG-12 — `ssl_protocols` / `ssl_ciphers` / curves configurables** (`[server.tls] protocols`, `ciphers`)
  - *Origen:* COMPAT_NGINX §5 · COMPAT_APACHE §5
  - *Criterio de aceptación:* SSL Labs A+ reproducible con config documentada
  - *Esfuerzo:* M

- [ ] **NG-13 — OCSP stapling + must-staple**
  - *Origen:* COMPAT_APACHE §4.16 · QUEJAS_*
  - *Criterio de aceptación:* Stapling verificado con `openssl s_client`
  - *Esfuerzo:* M

- [ ] **NG-14 — SSL session cache / tickets y rotación de claves**
  - *Origen:* COMPAT_NGINX §5
  - *Criterio de aceptación:* Handshake reanudado medible (ab/h2load)
  - *Esfuerzo:* S

- [ ] **Q-09 — ACME cero-manos**: DNS-01, wildcards, ARI, renovación con reload atómico y estado visible (`exyonqctl status` con expiración)
  - *Origen:* QUEJAS_NGINX §2.5
  - *Criterio de aceptación:* Renovación sin intervención en staging; wildcard funcionando; alerta de expiración
  - *Esfuerzo:* M

- [ ] **NG-15 — SNI multi-certificado en un mismo listener** (varios `[[server]]` sobre el mismo `listen` → un listener con `ResolvesServerCert` por `server_name`)
  - *Origen:* COMPAT_NGINX §5 (multi-`server{}` + `ssl_certificate` por vhost) · hallazgo real de producción Oracle 2026-10-24: solo se sirve el cert de `servers[0]` para cualquier SNI; `exyonq validate` acepta la config ambigua sin aviso
  - *Criterio de aceptación:* `openssl s_client -servername` devuelve el cert correcto por nombre en un único listener :443; `validate` rechaza o avisa configs que hoy se ignoran en silencio; E2E en host real
  - *Esfuerzo:* M

- [ ] **AP-02 — Renegociación / `SSLStrictSNIVHostCheck` equivalente** (SNI sin match → default determinista)
  - *Origen:* COMPAT_APACHE §4.16
  - *Criterio de aceptación:* Comportamiento documentado + tests
  - *Esfuerzo:* S

---

## Definition of Done (F4)

- [ ] La tabla TLS de las tres matrices sin ❌
- [ ] Guía "TLS hardening" publicada

**Dependencias:** 0.4 → 0.10 (seguridad usa auth/ACL) → 1.0.
